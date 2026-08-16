use std::{
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    process::{Child, Command as ProcessCommand, Stdio},
    time::Duration,
};

use mpd_protocol::{AsyncConnection, Command};
use tempfile::TempDir;
use tokio::{net::UnixStream, time::sleep};

pub const FIRST_TITLE: &str = "Nowplayd Fixture One";
pub const SECOND_TITLE: &str = "Nowplayd Fixture Two";

pub struct MpdHarness {
    temp: Option<TempDir>,
    child: Option<Child>,
    socket: PathBuf,
}

impl MpdHarness {
    pub async fn start() -> io::Result<Self> {
        let temp = tempfile::tempdir()?;
        let root = temp.path();
        let music = root.join("music");
        let playlists = root.join("playlists");
        fs::create_dir_all(&music)?;
        fs::create_dir_all(&playlists)?;

        let first = music.join("one.wav");
        let second = music.join("two.wav");
        write_silent_wav(&first, FIRST_TITLE)?;
        write_silent_wav(&second, SECOND_TITLE)?;

        let socket = root.join("mpd.sock");
        let config = root.join("mpd.conf");
        fs::write(
            &config,
            format!(
                "music_directory \"{}\"\n\
                 playlist_directory \"{}\"\n\
                 db_file \"{}\"\n\
                 log_file \"{}\"\n\
                 pid_file \"{}\"\n\
                 state_file \"{}\"\n\
                 sticker_file \"{}\"\n\
                 bind_to_address \"{}\"\n\
                 zeroconf_enabled \"no\"\n\
                 audio_output {{\n\
                   type \"null\"\n\
                   name \"nowplayd test null\"\n\
                 }}\n",
                config_path(&music),
                config_path(&playlists),
                config_path(&root.join("database")),
                config_path(&root.join("mpd.log")),
                config_path(&root.join("mpd.pid")),
                config_path(&root.join("state")),
                config_path(&root.join("sticker.sql")),
                config_path(&socket),
            ),
        )?;

        let mpd: OsString =
            std::env::var_os("MPD_BIN").unwrap_or_else(|| OsString::from("/opt/homebrew/bin/mpd"));
        let child = ProcessCommand::new(mpd)
            .arg("--no-daemon")
            .arg(&config)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;

        let mut harness = Self {
            temp: Some(temp),
            child: Some(child),
            socket,
        };
        harness.wait_until_ready().await?;
        Ok(harness)
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }

    pub async fn seed_and_play(
        &self,
    ) -> Result<AsyncConnection<UnixStream>, Box<dyn std::error::Error>> {
        let stream = UnixStream::connect(&self.socket).await?;
        let mut client = AsyncConnection::connect(stream).await?;
        run(&mut client, Command::new("clear")).await?;
        update_database(&mut client).await?;
        wait_for_title(&mut client, "one.wav", FIRST_TITLE).await?;
        wait_for_title(&mut client, "two.wav", SECOND_TITLE).await?;
        run(&mut client, Command::new("add").argument("one.wav")).await?;
        run(&mut client, Command::new("add").argument("two.wav")).await?;
        run(&mut client, Command::new("play").argument(0_u8)).await?;
        Ok(client)
    }

    pub fn shutdown(mut self) -> io::Result<PathBuf> {
        self.stop_child()?;
        let temp = self.temp.take().expect("temporary directory still owned");
        let path = temp.path().to_path_buf();
        temp.close()?;
        Ok(path)
    }

    async fn wait_until_ready(&mut self) -> io::Result<()> {
        for _ in 0..100 {
            if let Some(status) = self.child.as_mut().expect("child present").try_wait()? {
                return Err(io::Error::other(format!(
                    "isolated mpd exited before its socket was ready: {status}"
                )));
            }
            if self.socket.exists() && UnixStream::connect(&self.socket).await.is_ok() {
                return Ok(());
            }
            sleep(Duration::from_millis(20)).await;
        }
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "isolated mpd socket was not ready within two seconds",
        ))
    }

    fn stop_child(&mut self) -> io::Result<()> {
        if let Some(mut child) = self.child.take() {
            if child.try_wait()?.is_none() {
                child.kill()?;
            }
            child.wait()?;
        }
        Ok(())
    }
}

impl Drop for MpdHarness {
    fn drop(&mut self) {
        let _ = self.stop_child();
    }
}

async fn run(
    client: &mut AsyncConnection<UnixStream>,
    command: Command,
) -> Result<(), Box<dyn std::error::Error>> {
    let response = client.command(command).await?;
    for frame in response {
        if let Err(error) = frame {
            return Err(io::Error::other(format!("MPD command failed: {error:?}")).into());
        }
    }
    Ok(())
}

async fn update_database(
    client: &mut AsyncConnection<UnixStream>,
) -> Result<(), Box<dyn std::error::Error>> {
    run(client, Command::new("update")).await?;
    run(
        client,
        Command::new("idle").argument("update").argument("database"),
    )
    .await?;
    for _ in 0..100 {
        let response = client.command(Command::new("status")).await?;
        let mut updating = false;
        for frame in response {
            match frame {
                Ok(frame) => updating |= frame.find("updating_db").is_some(),
                Err(error) => {
                    return Err(io::Error::other(format!(
                        "MPD status failed while updating: {error:?}"
                    ))
                    .into());
                }
            }
        }
        if !updating {
            return Ok(());
        }
        sleep(Duration::from_millis(20)).await;
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "isolated MPD database update did not finish within two seconds",
    )
    .into())
}

async fn wait_for_title(
    client: &mut AsyncConnection<UnixStream>,
    uri: &str,
    expected: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    for _ in 0..100 {
        let response = client.command(Command::new("lsinfo").argument(uri)).await?;
        let mut observed = None;
        for frame in response {
            match frame {
                Ok(frame) => observed = frame.find("Title").map(str::to_owned),
                Err(error) => {
                    return Err(io::Error::other(format!(
                        "MPD lsinfo failed while checking tags: {error:?}"
                    ))
                    .into());
                }
            }
        }
        if observed.as_deref() == Some(expected) {
            return Ok(());
        }
        sleep(Duration::from_millis(20)).await;
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        format!("isolated MPD did not index title {expected:?} for {uri}"),
    )
    .into())
}

fn config_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

fn write_silent_wav(path: &Path, title: &str) -> io::Result<()> {
    const SAMPLE_RATE: u32 = 8_000;
    // Long enough that scheduling jitter cannot let playback finish before the
    // post-`next` snapshot is collected, while still only 80 KiB per fixture.
    const SAMPLE_COUNT: usize = 40_000;

    let mut body = Vec::new();
    push_chunk(
        &mut body,
        b"fmt ",
        &[
            1, 0, // PCM
            1, 0, // mono
            0x40, 0x1f, 0, 0, // 8 kHz
            0x80, 0x3e, 0, 0, // 16 kB/s
            2, 0, // block alignment
            16, 0, // bits per sample
        ],
    );

    let mut info = b"INFO".to_vec();
    push_info(&mut info, b"INAM", title);
    push_info(&mut info, b"IART", "nowplayd test");
    push_info(&mut info, b"IPRD", "transport fixtures");
    push_chunk(&mut body, b"LIST", &info);

    let silence = vec![0_u8; SAMPLE_COUNT * 2];
    push_chunk(&mut body, b"data", &silence);

    let mut wav = b"RIFF".to_vec();
    wav.extend_from_slice(&(body.len() as u32 + 4).to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(&body);
    debug_assert_eq!(SAMPLE_RATE, 8_000);
    fs::write(path, wav)
}

fn push_chunk(target: &mut Vec<u8>, id: &[u8; 4], data: &[u8]) {
    target.extend_from_slice(id);
    target.extend_from_slice(&(data.len() as u32).to_le_bytes());
    target.extend_from_slice(data);
    if data.len() % 2 == 1 {
        target.push(0);
    }
}

fn push_info(target: &mut Vec<u8>, id: &[u8; 4], value: &str) {
    let mut nul_terminated = value.as_bytes().to_vec();
    nul_terminated.push(0);
    push_chunk(target, id, &nul_terminated);
}
