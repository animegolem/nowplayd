use std::time::Duration;

use nowplayd::{
    mpd::{BinaryCommand, CommandConnection, IdleConnection, MpdError, Subsystem},
    state::{MediaKey, OccurrenceId, PlaybackState, PlayerState, SongMetadata},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, DuplexStream, duplex},
    net::TcpListener,
};

use nowplayd::mpd::{ConnectionConfig, MpdAddress};

const REFRESH_REQUEST: &str = "command_list_ok_begin\nstatus\ncurrentsong\ncommand_list_end\n";

fn scripted_server(steps: Vec<(&'static str, &'static str)>) -> DuplexStream {
    let (client, server) = duplex(16 * 1024);
    tokio::spawn(serve_script(server, steps));
    client
}

async fn serve_script<IO>(io: IO, steps: Vec<(&'static str, &'static str)>)
where
    IO: AsyncRead + AsyncWrite + Unpin,
{
    let (read, mut write) = tokio::io::split(io);
    let mut read = BufReader::new(read);
    write.write_all(b"OK MPD 0.24.0\n").await.unwrap();

    for (expected, response) in steps {
        let mut request = String::new();
        loop {
            let mut line = String::new();
            let count = read.read_line(&mut line).await.unwrap();
            assert_ne!(count, 0, "client closed before sending {expected:?}");
            request.push_str(&line);
            if !request.starts_with("command_list_ok_begin\n") || line == "command_list_end\n" {
                break;
            }
        }
        assert_eq!(request, expected);
        write.write_all(response.as_bytes()).await.unwrap();
    }
}

#[tokio::test]
async fn fixture_maps_exact_coherent_snapshot_fields() {
    let io = scripted_server(vec![(
        REFRESH_REQUEST,
        include_str!("fixtures/snapshot-playing.txt"),
    )]);
    let mut connection = CommandConnection::from_io(io).await.unwrap();

    assert_eq!(
        connection.refresh().await.unwrap(),
        PlayerState {
            occurrence: Some(OccurrenceId(42)),
            media_key: Some(MediaKey("albums/example/track.flac".into())),
            metadata: SongMetadata {
                title: Some("Fixture Title".into()),
                artists: vec!["First Artist".into(), "Second Artist".into()],
                album: Some("Fixture Album".into()),
            },
            playback: PlaybackState::Playing,
            elapsed: Some(Duration::from_millis(12_500)),
            duration: Some(Duration::from_millis(180_250)),
        }
    );
}

#[tokio::test]
async fn no_current_song_maps_to_absent_identities_and_metadata() {
    let io = scripted_server(vec![(
        REFRESH_REQUEST,
        include_str!("fixtures/snapshot-no-song.txt"),
    )]);
    let mut connection = CommandConnection::from_io(io).await.unwrap();

    assert_eq!(
        connection.refresh().await.unwrap(),
        PlayerState {
            playback: PlaybackState::Stopped,
            ..PlayerState::default()
        }
    );
}

#[tokio::test]
async fn song_uri_without_occurrence_id_is_rejected() {
    let io = scripted_server(vec![(
        REFRESH_REQUEST,
        include_str!("fixtures/snapshot-file-without-id.txt"),
    )]);
    let mut connection = CommandConnection::from_io(io).await.unwrap();

    assert!(matches!(
        connection.refresh().await.unwrap_err(),
        MpdError::MissingField("Id")
    ));
}

#[tokio::test]
async fn mismatched_snapshot_is_discarded_and_retried() {
    let io = scripted_server(vec![
        (
            REFRESH_REQUEST,
            include_str!("fixtures/snapshot-mismatch.txt"),
        ),
        (
            REFRESH_REQUEST,
            include_str!("fixtures/snapshot-playing.txt"),
        ),
    ]);
    let mut connection = CommandConnection::from_io(io).await.unwrap();

    let state = connection.refresh().await.unwrap();
    assert_eq!(state.occurrence, Some(OccurrenceId(42)));
    assert_eq!(state.metadata.title.as_deref(), Some("Fixture Title"));
}

#[tokio::test]
async fn command_ack_is_typed_and_does_not_become_transport_failure() {
    let io = scripted_server(vec![
        ("next\n", include_str!("fixtures/ack.txt")),
        (
            REFRESH_REQUEST,
            include_str!("fixtures/snapshot-playing.txt"),
        ),
    ]);
    let mut connection = CommandConnection::from_io(io).await.unwrap();

    let error = connection.next().await.unwrap_err();
    match error {
        MpdError::Command(error) => {
            assert_eq!(error.code, 5);
            assert_eq!(error.command.as_deref(), Some("next"));
            assert_eq!(error.message, "No next song");
        }
        error => panic!("expected command error, got {error:?}"),
    }
    assert_eq!(
        connection.refresh().await.unwrap().occurrence,
        Some(OccurrenceId(42)),
        "a command ACK must leave the transport usable"
    );
}

#[tokio::test]
async fn idle_role_sends_only_the_ruled_filter_and_maps_events() {
    let io = scripted_server(vec![(
        "idle player mixer options\n",
        include_str!("fixtures/idle.txt"),
    )]);
    let mut connection = IdleConnection::from_io(io).await.unwrap();

    assert_eq!(
        connection.next_event().await.unwrap(),
        vec![Subsystem::Player, Subsystem::Options]
    );
}

#[tokio::test]
async fn binary_primitive_preserves_bytes_and_total_size() {
    let io = scripted_server(vec![(
        "albumart albums/example/track.flac 12\n",
        "size: 15\nbinary: 3\nabc\nOK\n",
    )]);
    let mut connection = CommandConnection::from_io(io).await.unwrap();

    assert_eq!(
        connection
            .read_binary(BinaryCommand::AlbumArt, "albums/example/track.flac", 12)
            .await
            .unwrap(),
        nowplayd::mpd::BinaryResponse {
            total_size: Some(15),
            bytes: b"abc".to_vec(),
        }
    );
}

#[tokio::test]
async fn configured_tcp_connection_authenticates_before_refresh() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        serve_script(
            stream,
            vec![
                ("password \"sentinel secret\"\n", "OK\n"),
                (
                    REFRESH_REQUEST,
                    include_str!("fixtures/snapshot-playing.txt"),
                ),
            ],
        )
        .await;
    });
    let config = ConnectionConfig {
        address: MpdAddress::Tcp(address.to_string()),
        password: Some("sentinel secret".into()),
    };

    let mut connection = CommandConnection::connect(&config).await.unwrap();
    assert_eq!(
        connection.refresh().await.unwrap().occurrence,
        Some(OccurrenceId(42))
    );
    server.await.unwrap();
}

#[tokio::test]
async fn configured_idle_connection_authenticates_before_idling() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        serve_script(
            stream,
            vec![
                ("password sentinel-secret\n", "OK\n"),
                (
                    "idle player mixer options\n",
                    include_str!("fixtures/idle.txt"),
                ),
            ],
        )
        .await;
    });
    let config = ConnectionConfig {
        address: MpdAddress::Tcp(address.to_string()),
        password: Some("sentinel-secret".into()),
    };

    let mut connection = IdleConnection::connect(&config).await.unwrap();
    assert_eq!(
        connection.next_event().await.unwrap(),
        vec![Subsystem::Player, Subsystem::Options]
    );
    server.await.unwrap();
}

#[tokio::test]
async fn idle_connection_drop_surfaces_as_transport_error() {
    let io = scripted_server(vec![("idle player mixer options\n", "")]);
    let mut connection = IdleConnection::from_io(io).await.unwrap();

    assert!(matches!(
        connection.next_event().await.unwrap_err(),
        MpdError::Transport(_)
    ));
}
