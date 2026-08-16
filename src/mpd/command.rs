use mpd_protocol::{AsyncConnection, Command, CommandList, response::Frame};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::state::{MediaKey, OccurrenceId, PlaybackState, PlayerState, SongMetadata};

use super::{
    ConnectionConfig, MpdError, MpdIo, Result, authenticate, collect_frames, exactly_one, open_io,
    parse_optional_duration, parse_optional_u64,
};

/// Binary MPD query supported by the transport primitive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryCommand {
    AlbumArt,
    ReadPicture,
}

impl BinaryCommand {
    fn name(self) -> &'static str {
        match self {
            Self::AlbumArt => "albumart",
            Self::ReadPicture => "readpicture",
        }
    }
}

/// One chunk returned by an MPD binary query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinaryResponse {
    pub total_size: Option<usize>,
    pub bytes: Vec<u8>,
}

/// The MPD role that performs queries and playback commands and never idles.
#[derive(Debug)]
pub struct CommandConnection<IO = MpdIo> {
    inner: AsyncConnection<IO>,
}

impl CommandConnection<MpdIo> {
    pub async fn connect(config: &ConnectionConfig) -> Result<Self> {
        let io = open_io(&config.address).await?;
        let mut connection = Self::from_io(io).await?;
        if let Some(password) = &config.password {
            authenticate(&mut connection.inner, password).await?;
        }
        Ok(connection)
    }
}

impl<IO> CommandConnection<IO>
where
    IO: AsyncRead + AsyncWrite + Unpin,
{
    pub async fn from_io(io: IO) -> Result<Self> {
        Ok(Self {
            inner: AsyncConnection::connect(io).await?,
        })
    }

    /// Refresh until `status.songid` and `currentsong.Id` describe one song.
    pub async fn refresh(&mut self) -> Result<PlayerState> {
        loop {
            match self.refresh_once().await {
                Err(MpdError::IncoherentSnapshot { .. }) => continue,
                result => return result,
            }
        }
    }

    pub async fn toggle(&mut self) -> Result<()> {
        self.run(Command::new("pause")).await
    }

    pub async fn play(&mut self) -> Result<()> {
        self.run(Command::new("play")).await
    }

    pub async fn pause(&mut self) -> Result<()> {
        self.run(Command::new("pause").argument(1_u8)).await
    }

    pub async fn next(&mut self) -> Result<()> {
        self.run(Command::new("next")).await
    }

    pub async fn previous(&mut self) -> Result<()> {
        self.run(Command::new("previous")).await
    }

    pub async fn ping(&mut self) -> Result<()> {
        self.run(Command::new("ping")).await
    }

    /// Read one MPD-managed chunk. Chunk scheduling belongs to AI-IMP-004.
    pub async fn read_binary(
        &mut self,
        kind: BinaryCommand,
        uri: &str,
        offset: usize,
    ) -> Result<BinaryResponse> {
        let response = self
            .inner
            .command(Command::new(kind.name()).argument(uri).argument(offset))
            .await?;
        let mut frame = exactly_one(response)?;
        let total_size = frame
            .find("size")
            .map(|value| {
                value.parse().map_err(|_| MpdError::InvalidField {
                    field: "size",
                    value: value.into(),
                })
            })
            .transpose()?;
        let bytes = frame.take_binary().map_or_else(Vec::new, Into::into);

        Ok(BinaryResponse { total_size, bytes })
    }

    async fn refresh_once(&mut self) -> Result<PlayerState> {
        let response = self
            .inner
            .command_list(
                CommandList::new(Command::new("status")).command(Command::new("currentsong")),
            )
            .await?;
        let frames = collect_frames(response)?;
        if frames.len() != 2 {
            return Err(MpdError::UnexpectedFrameCount {
                expected: 2,
                actual: frames.len(),
            });
        }

        map_snapshot(&frames[0], &frames[1])
    }

    async fn run(&mut self, command: Command) -> Result<()> {
        let response = self.inner.command(command).await?;
        let frame = exactly_one(response)?;
        if !frame.is_empty() {
            return Err(MpdError::UnexpectedFields {
                actual: frame.fields_len(),
            });
        }
        Ok(())
    }
}

fn map_snapshot(status: &Frame, current: &Frame) -> Result<PlayerState> {
    let status_song_id = parse_optional_u64(status, "songid")?;
    let current_song_id = parse_optional_u64(current, "Id")?;
    if status_song_id != current_song_id {
        return Err(MpdError::IncoherentSnapshot {
            status_song_id,
            current_song_id,
        });
    }

    let playback = match status
        .find("state")
        .ok_or(MpdError::MissingField("state"))?
    {
        "play" => PlaybackState::Playing,
        "pause" => PlaybackState::Paused,
        "stop" => PlaybackState::Stopped,
        value => {
            return Err(MpdError::InvalidField {
                field: "state",
                value: value.into(),
            });
        }
    };

    let media_key = current.find("file").map(|value| MediaKey(value.into()));
    match (current_song_id, media_key.as_ref()) {
        (Some(_), None) => return Err(MpdError::MissingField("file")),
        (None, Some(_)) => return Err(MpdError::MissingField("Id")),
        _ => {}
    }

    let metadata = SongMetadata {
        title: current.find("Title").map(Into::into),
        artists: current
            .fields()
            .filter(|(key, _)| *key == "Artist")
            .map(|(_, value)| value.into())
            .collect(),
        album: current.find("Album").map(Into::into),
    };

    Ok(PlayerState {
        occurrence: current_song_id.map(OccurrenceId),
        media_key,
        metadata,
        playback,
        elapsed: parse_optional_duration(status, "elapsed")?,
        duration: parse_optional_duration(status, "duration")?
            .or(parse_optional_duration(current, "duration")?),
    })
}
