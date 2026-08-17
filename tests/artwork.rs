use std::{
    collections::VecDeque,
    io::Cursor,
    sync::{Arc, Mutex},
};

use image::{DynamicImage, ImageFormat};
use nowplayd::{
    artwork::{ArtworkCache, ArtworkCoordinator, BinaryChunkSource, MAX_ARTWORK_BYTES},
    mpd::{
        BinaryCommand, BinaryResponse, CommandFailure, ConnectionConfig, LiveCommandConnection,
        MpdAddress, MpdError,
    },
    platform::PublicationIntent,
    state::{MediaKey, OccurrenceId, PlaybackState, PlayerState},
};
use tempfile::TempDir;
use tokio::{
    io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
};
use url::Url;

struct FakeSource {
    responses: VecDeque<Result<BinaryResponse, MpdError>>,
    requests: Vec<(BinaryCommand, String, usize)>,
}

impl FakeSource {
    fn new(responses: impl IntoIterator<Item = Result<BinaryResponse, MpdError>>) -> Self {
        Self {
            responses: responses.into_iter().collect(),
            requests: Vec::new(),
        }
    }
}

impl BinaryChunkSource for FakeSource {
    async fn read_binary(
        &mut self,
        kind: BinaryCommand,
        uri: &str,
        offset: usize,
    ) -> Result<BinaryResponse, MpdError> {
        self.requests.push((kind, uri.into(), offset));
        self.responses
            .pop_front()
            .expect("scripted artwork response")
    }
}

fn state(uri: &str, occurrence: u64, elapsed: u64) -> PlayerState {
    PlayerState {
        occurrence: Some(OccurrenceId(occurrence)),
        media_key: Some(MediaKey(uri.into())),
        playback: PlaybackState::Playing,
        elapsed: Some(std::time::Duration::from_secs(elapsed)),
        ..PlayerState::default()
    }
}

fn png() -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::new_rgba8(3, 2)
        .write_to(&mut bytes, ImageFormat::Png)
        .unwrap();
    bytes.into_inner()
}

fn no_exist(command: &str) -> MpdError {
    MpdError::Command(CommandFailure {
        code: 50,
        command_index: 0,
        command: Some(command.into()),
        message: "No file exists".into(),
    })
}

async fn resolve_lookup(coordinator: &mut ArtworkCoordinator, source: &mut FakeSource) {
    let scheduled = coordinator.step(source).await.unwrap();
    let completed = coordinator.complete_job(scheduled.job.unwrap().run());
    assert!(completed.publication.is_none());
}

fn resolve_job_if_any(
    coordinator: &mut ArtworkCoordinator,
    mut update: nowplayd::artwork::ArtworkUpdate,
) -> nowplayd::artwork::ArtworkUpdate {
    match update.job.take() {
        Some(job) => coordinator.complete_job(job.run()),
        None => update,
    }
}

#[tokio::test]
async fn albumart_ack_falls_back_and_empty_readpicture_publishes_explicit_no_art() {
    let temp = TempDir::new().unwrap();
    let mut source = FakeSource::new([
        Err(no_exist("albumart")),
        Ok(BinaryResponse {
            total_size: None,
            bytes: Vec::new(),
        }),
    ]);
    let mut coordinator = ArtworkCoordinator::new(ArtworkCache::new(temp.path().into()));

    let first = coordinator.observe_state(state("track.flac", 1, 0));
    assert_eq!(first.publication.unwrap().cover_url, None);
    resolve_lookup(&mut coordinator, &mut source).await;
    assert!(
        coordinator
            .step(&mut source)
            .await
            .unwrap()
            .publication
            .is_none()
    );
    let no_art = coordinator.step(&mut source).await.unwrap();

    assert_eq!(no_art.publication.unwrap().cover_url, None);
    assert_eq!(
        source.requests,
        [
            (BinaryCommand::AlbumArt, "track.flac".into(), 0),
            (BinaryCommand::ReadPicture, "track.flac".into(), 0),
        ]
    );
}

#[tokio::test]
async fn command_failure_is_not_misclassified_as_no_art_or_fallback() {
    let temp = TempDir::new().unwrap();
    let mut source = FakeSource::new([Err(MpdError::Command(CommandFailure {
        code: 4,
        command_index: 0,
        command: Some("albumart".into()),
        message: "permission denied".into(),
    }))]);
    let mut coordinator = ArtworkCoordinator::new(ArtworkCache::new(temp.path().into()));
    coordinator.observe_state(state("track.flac", 1, 0));
    resolve_lookup(&mut coordinator, &mut source).await;

    let failed = coordinator.step(&mut source).await.unwrap();
    assert!(failed.warning.unwrap().contains("permission denied"));
    assert_eq!(failed.publication.unwrap().cover_url, None);
    assert_eq!(
        source.requests.len(),
        1,
        "permission errors never fall back"
    );
    assert!(!coordinator.has_pending_work());
}

#[tokio::test]
async fn same_media_republish_retains_url_and_fetches_zero_additional_chunks() {
    let temp = TempDir::new().unwrap();
    let image = png();
    let mut source = FakeSource::new([Ok(BinaryResponse {
        total_size: Some(image.len()),
        bytes: image,
    })]);
    let mut coordinator = ArtworkCoordinator::new(ArtworkCache::new(temp.path().into()));
    coordinator.observe_state(state("track.flac", 1, 1));
    resolve_lookup(&mut coordinator, &mut source).await;
    let scheduled = coordinator.step(&mut source).await.unwrap();
    let art = resolve_job_if_any(&mut coordinator, scheduled);
    let cover = art.publication.unwrap().cover_url.unwrap();

    let occurrence_change = coordinator.observe_state(state("track.flac", 2, 9));
    let publication = occurrence_change.publication.unwrap();
    assert_eq!(publication.intent, PublicationIntent::PlaybackOnly);
    assert_eq!(publication.cover_url.as_deref(), Some(cover.as_str()));
    assert_eq!(
        publication.state.elapsed,
        Some(std::time::Duration::from_secs(9))
    );
    assert_eq!(source.requests.len(), 1);
    assert!(!coordinator.has_pending_work());
}

#[tokio::test]
async fn held_art_is_removed_only_when_new_media_resolves_no_art() {
    let temp = TempDir::new().unwrap();
    let image = png();
    let mut source = FakeSource::new([
        Ok(BinaryResponse {
            total_size: Some(image.len()),
            bytes: image,
        }),
        Err(no_exist("albumart")),
        Ok(BinaryResponse {
            total_size: None,
            bytes: Vec::new(),
        }),
    ]);
    let mut coordinator = ArtworkCoordinator::new(ArtworkCache::new(temp.path().into()));
    coordinator.observe_state(state("a.flac", 1, 1));
    resolve_lookup(&mut coordinator, &mut source).await;
    let fetched = coordinator.step(&mut source).await.unwrap();
    let art = resolve_job_if_any(&mut coordinator, fetched);
    let held_url = art.publication.unwrap().cover_url.unwrap();

    let transition = coordinator.observe_state(state("b.flac", 2, 0));
    assert_eq!(
        transition.publication.unwrap().cover_url.as_deref(),
        Some(held_url.as_str())
    );
    resolve_lookup(&mut coordinator, &mut source).await;
    assert!(
        coordinator
            .step(&mut source)
            .await
            .unwrap()
            .publication
            .is_none()
    );
    let resolved = coordinator.step(&mut source).await.unwrap();
    let publication = resolved.publication.unwrap();
    assert_eq!(publication.intent, PublicationIntent::FullMetadata);
    assert_eq!(publication.cover_url, None);
}

#[test]
fn occurrence_only_change_makes_no_platform_publication() {
    let mut coordinator = ArtworkCoordinator::new(ArtworkCache::disabled());
    let initial = state("track.flac", 1, 9);
    coordinator.observe_state(initial.clone());

    let mut occurrence_only = initial;
    occurrence_only.occurrence = Some(OccurrenceId(2));
    assert!(
        coordinator
            .observe_state(occurrence_only)
            .publication
            .is_none()
    );
}

#[tokio::test]
async fn corrupt_cached_reuse_is_logged_and_refetched_before_url_publication() {
    let temp = TempDir::new().unwrap();
    let image = png();
    let mut first_source = FakeSource::new([Ok(BinaryResponse {
        total_size: Some(image.len()),
        bytes: image.clone(),
    })]);
    let mut first = ArtworkCoordinator::new(ArtworkCache::new(temp.path().into()));
    first.observe_state(state("track.flac", 1, 0));
    resolve_lookup(&mut first, &mut first_source).await;
    let scheduled = first.step(&mut first_source).await.unwrap();
    let first_art = resolve_job_if_any(&mut first, scheduled);
    let path = Url::parse(&first_art.publication.unwrap().cover_url.unwrap())
        .unwrap()
        .to_file_path()
        .unwrap();
    std::fs::write(path, b"corrupt").unwrap();

    let mut second = ArtworkCoordinator::new(ArtworkCache::new(temp.path().into()));
    let observed = second.observe_state(state("track.flac", 9, 0));
    assert_eq!(observed.publication.unwrap().cover_url, None);
    let mut second_source = FakeSource::new([Ok(BinaryResponse {
        total_size: Some(image.len()),
        bytes: image,
    })]);
    let scheduled_lookup = second.step(&mut second_source).await.unwrap();
    let rejected = second.complete_job(scheduled_lookup.job.unwrap().run());
    assert!(rejected.warning.unwrap().contains("refetching"));
    let scheduled = second.step(&mut second_source).await.unwrap();
    let refetched = resolve_job_if_any(&mut second, scheduled);
    assert!(refetched.publication.unwrap().cover_url.is_some());
    assert_eq!(second_source.requests.len(), 1);
}

#[tokio::test]
async fn invalid_bytes_and_strict_assembly_failures_never_publish_a_url() {
    let cases = [
        BinaryResponse {
            total_size: None,
            bytes: vec![1],
        },
        BinaryResponse {
            total_size: Some(MAX_ARTWORK_BYTES + 1),
            bytes: vec![1],
        },
        BinaryResponse {
            total_size: Some(4),
            bytes: Vec::new(),
        },
        BinaryResponse {
            total_size: Some(1),
            bytes: vec![1, 2],
        },
        BinaryResponse {
            total_size: Some(4),
            bytes: b"nope".to_vec(),
        },
    ];

    for response in cases {
        let temp = TempDir::new().unwrap();
        let mut source = FakeSource::new([Ok(response)]);
        let mut coordinator = ArtworkCoordinator::new(ArtworkCache::new(temp.path().into()));
        coordinator.observe_state(state("track.flac", 1, 0));
        resolve_lookup(&mut coordinator, &mut source).await;
        let update = coordinator.step(&mut source).await.unwrap();
        let failed = resolve_job_if_any(&mut coordinator, update);
        assert!(failed.warning.is_some());
        assert_eq!(failed.publication.unwrap().cover_url, None);
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
    }
}

#[tokio::test]
async fn changed_declared_size_is_rejected_without_partial_final_file() {
    let temp = TempDir::new().unwrap();
    let mut source = FakeSource::new([
        Ok(BinaryResponse {
            total_size: Some(4),
            bytes: vec![1, 2],
        }),
        Ok(BinaryResponse {
            total_size: Some(5),
            bytes: vec![3, 4],
        }),
    ]);
    let mut coordinator = ArtworkCoordinator::new(ArtworkCache::new(temp.path().into()));
    coordinator.observe_state(state("track.flac", 1, 0));
    resolve_lookup(&mut coordinator, &mut source).await;
    assert!(
        coordinator
            .step(&mut source)
            .await
            .unwrap()
            .publication
            .is_none()
    );
    let failed = coordinator.step(&mut source).await.unwrap();

    assert!(failed.warning.unwrap().contains("size changed"));
    assert_eq!(failed.publication.unwrap().cover_url, None);
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
}

async fn read_request(reader: &mut BufReader<tokio::net::tcp::OwnedReadHalf>) -> String {
    let mut request = String::new();
    reader.read_line(&mut request).await.unwrap();
    request
}

async fn greet_and_authenticate(
    stream: TcpStream,
    requests: &Arc<Mutex<Vec<String>>>,
) -> (
    BufReader<tokio::net::tcp::OwnedReadHalf>,
    tokio::net::tcp::OwnedWriteHalf,
) {
    let (read, mut write) = stream.into_split();
    write.write_all(b"OK MPD 0.24.0\n").await.unwrap();
    let mut reader = BufReader::new(read);
    let auth = read_request(&mut reader).await;
    requests.lock().unwrap().push(auth.clone());
    assert_eq!(auth, "password secret\n");
    write.write_all(b"OK\n").await.unwrap();
    (reader, write)
}

async fn binary_response<W>(write: &mut W, total: usize, bytes: &[u8])
where
    W: AsyncWrite + Unpin,
{
    write
        .write_all(format!("size: {total}\nbinary: {}\n", bytes.len()).as_bytes())
        .await
        .unwrap();
    write.write_all(bytes).await.unwrap();
    write.write_all(b"\nOK\n").await.unwrap();
}

#[tokio::test]
async fn nonzero_offset_transport_drop_reauthenticates_and_reissues_exact_offset_once() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let image = png();
    let split = image.len() / 2;
    let requests = Arc::new(Mutex::new(Vec::new()));
    let server_requests = requests.clone();
    let server_image = image.clone();
    let server = tokio::spawn(async move {
        let (first, _) = listener.accept().await.unwrap();
        let (mut first_read, mut first_write) =
            greet_and_authenticate(first, &server_requests).await;
        let offset_zero = read_request(&mut first_read).await;
        server_requests.lock().unwrap().push(offset_zero.clone());
        assert_eq!(offset_zero, "albumart track.flac 0\n");
        binary_response(&mut first_write, server_image.len(), &server_image[..split]).await;
        let dropped = read_request(&mut first_read).await;
        server_requests.lock().unwrap().push(dropped.clone());
        assert_eq!(dropped, format!("albumart track.flac {split}\n"));
        drop(first_read);
        drop(first_write);

        let (second, _) = listener.accept().await.unwrap();
        let (mut second_read, mut second_write) =
            greet_and_authenticate(second, &server_requests).await;
        let retried = read_request(&mut second_read).await;
        server_requests.lock().unwrap().push(retried.clone());
        assert_eq!(retried, format!("albumart track.flac {split}\n"));
        binary_response(
            &mut second_write,
            server_image.len(),
            &server_image[split..],
        )
        .await;
    });

    let config = ConnectionConfig {
        address: MpdAddress::Tcp(address.to_string()),
        password: Some("secret".into()),
    };
    let mut live = LiveCommandConnection::connect(config).await.unwrap();
    let temp = TempDir::new().unwrap();
    let mut coordinator = ArtworkCoordinator::new(ArtworkCache::new(temp.path().into()));
    coordinator.observe_state(state("track.flac", 1, 0));
    let lookup = coordinator.step(&mut live).await.unwrap();
    assert!(
        coordinator
            .complete_job(lookup.job.unwrap().run())
            .publication
            .is_none()
    );

    assert!(
        coordinator
            .step(&mut live)
            .await
            .unwrap()
            .publication
            .is_none()
    );
    let scheduled = coordinator.step(&mut live).await.unwrap();
    let completed = resolve_job_if_any(&mut coordinator, scheduled);
    assert!(completed.publication.unwrap().cover_url.is_some());
    server.await.unwrap();

    let requests = requests.lock().unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.as_str() == format!("albumart track.flac {split}\n"))
            .count(),
        2
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.as_str() == "password secret\n")
            .count(),
        2
    );
}
