//! Strict MPD artwork assembly, validation, cache, and publication state.

use std::{
    error::Error,
    fmt,
    fs::{self, File, OpenOptions},
    future::Future,
    io::{self, Cursor, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use image::{GenericImageView, ImageFormat, ImageReader};
use sha2::{Digest, Sha256};
use url::Url;

use crate::{
    mpd::{BinaryCommand, BinaryResponse, LiveCommandConnection, LivenessClock, MpdError},
    state::{MediaKey, PlayerState},
};

/// Maximum compressed artwork accepted from MPD.
pub const MAX_ARTWORK_BYTES: usize = 10 * 1024 * 1024;
/// Maximum width or height accepted by the v1 decoder boundary.
pub const MAX_ARTWORK_DIMENSION: u32 = 8192;

const ACK_NO_EXIST: u64 = 50;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// Full application-owned publication. Artwork is part of every metadata write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtworkPublication {
    pub state: PlayerState,
    pub cover_url: Option<String>,
}

/// Typed artwork failure. Only exhausted MPD transport errors are worker-fatal.
#[derive(Debug)]
pub enum ArtworkError {
    Mpd(MpdError),
    MissingSize,
    SizeChanged { expected: usize, actual: usize },
    TooLarge { size: usize, limit: usize },
    ZeroProgress { offset: usize, total: usize },
    Overrun { end: usize, total: usize },
    OffsetOverflow,
    Allocation { size: usize },
    Io(io::Error),
    UnsupportedFormat,
    InvalidImage(String),
    Dimensions { width: u32, height: u32 },
    InvalidFileUrl(PathBuf),
    MissingHome,
}

impl ArtworkError {
    pub fn is_transport(&self) -> bool {
        matches!(self, Self::Mpd(error) if error.is_transport())
    }
}

impl fmt::Display for ArtworkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mpd(error) => write!(f, "artwork MPD operation failed: {error}"),
            Self::MissingSize => f.write_str("artwork response omitted required size"),
            Self::SizeChanged { expected, actual } => write!(
                f,
                "artwork response size changed from {expected} to {actual}"
            ),
            Self::TooLarge { size, limit } => {
                write!(f, "artwork size {size} exceeds {limit}-byte limit")
            }
            Self::ZeroProgress { offset, total } => write!(
                f,
                "artwork response made no progress at offset {offset} of {total}"
            ),
            Self::Overrun { end, total } => {
                write!(
                    f,
                    "artwork response ended at {end}, beyond declared size {total}"
                )
            }
            Self::OffsetOverflow => f.write_str("artwork response offset overflowed"),
            Self::Allocation { size } => {
                write!(f, "could not reserve {size} bytes for artwork")
            }
            Self::Io(error) => write!(f, "artwork cache I/O failed: {error}"),
            Self::UnsupportedFormat => f.write_str("artwork format is not a supported JPEG or PNG"),
            Self::InvalidImage(error) => write!(f, "artwork decode failed: {error}"),
            Self::Dimensions { width, height } => write!(
                f,
                "artwork dimensions {width}x{height} exceed {MAX_ARTWORK_DIMENSION}x{MAX_ARTWORK_DIMENSION}"
            ),
            Self::InvalidFileUrl(path) => {
                write!(
                    f,
                    "artwork cache path is not a valid file URL: {}",
                    path.display()
                )
            }
            Self::MissingHome => f.write_str("cannot resolve the user cache directory"),
        }
    }
}

impl Error for ArtworkError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Mpd(error) => Some(error),
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<MpdError> for ArtworkError {
    fn from(error: MpdError) -> Self {
        Self::Mpd(error)
    }
}

impl From<io::Error> for ArtworkError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// One-call binary seam used by the one-chunk-per-turn scheduler and fixtures.
pub trait BinaryChunkSource {
    fn read_binary(
        &mut self,
        kind: BinaryCommand,
        uri: &str,
        offset: usize,
    ) -> impl Future<Output = Result<BinaryResponse, MpdError>>;
}

impl<C> BinaryChunkSource for LiveCommandConnection<C>
where
    C: LivenessClock,
{
    async fn read_binary(
        &mut self,
        kind: BinaryCommand,
        uri: &str,
        offset: usize,
    ) -> Result<BinaryResponse, MpdError> {
        LiveCommandConnection::read_binary(self, kind, uri, offset).await
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ValidatedFormat {
    Jpeg,
    Png,
}

impl ValidatedFormat {
    fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
        }
    }

    fn image_format(self) -> ImageFormat {
        match self {
            Self::Jpeg => ImageFormat::Jpeg,
            Self::Png => ImageFormat::Png,
        }
    }
}

#[derive(Clone, Debug)]
struct CachedArtwork {
    path: PathBuf,
    url: String,
}

/// Cache writer with current+previous runtime retention.
#[derive(Clone, Debug)]
pub struct ArtworkCache {
    root: PathBuf,
    enabled: bool,
    current: Option<PathBuf>,
    previous: Option<PathBuf>,
}

impl ArtworkCache {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            enabled: true,
            current: None,
            previous: None,
        }
    }

    pub fn disabled() -> Self {
        Self {
            root: PathBuf::new(),
            enabled: false,
            current: None,
            previous: None,
        }
    }

    fn lookup(&self, key: &MediaKey) -> Result<Option<CachedArtwork>, ArtworkError> {
        if !self.enabled {
            return Ok(None);
        }
        let digest = media_digest(key);
        let mut candidates = Vec::new();
        for extension in ["jpg", "png"] {
            let path = self.root.join(format!("{digest}.{extension}"));
            if !path.exists() {
                continue;
            }
            let modified = fs::metadata(&path)?.modified()?;
            candidates.push((modified, extension, path));
        }
        candidates.sort_by(|left, right| right.0.cmp(&left.0));
        if let Some((_, extension, path)) = candidates.into_iter().next() {
            let bytes = fs::read(&path)?;
            let validated = validate_image(&bytes)?;
            if validated.extension() != extension {
                return Err(ArtworkError::UnsupportedFormat);
            }
            return Ok(Some(cached_artwork(path)?));
        }
        Ok(None)
    }

    fn store(&self, key: &MediaKey, bytes: &[u8]) -> Result<CachedArtwork, ArtworkError> {
        if !self.enabled {
            return Err(ArtworkError::MissingHome);
        }
        if bytes.len() > MAX_ARTWORK_BYTES {
            return Err(ArtworkError::TooLarge {
                size: bytes.len(),
                limit: MAX_ARTWORK_BYTES,
            });
        }
        let format = validate_image(bytes)?;
        fs::create_dir_all(&self.root)?;

        let digest = media_digest(key);
        let final_path = self.root.join(format!("{digest}.{}", format.extension()));
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temp_path = self
            .root
            .join(format!(".{digest}.tmp-{}-{sequence}", std::process::id()));
        let mut temp = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)?;

        if let Err(error) = write_sync_rename(&mut temp, &temp_path, &final_path, bytes) {
            let _ = fs::remove_file(&temp_path);
            return Err(error);
        }

        cached_artwork(final_path)
    }

    fn activate(&mut self, artwork: &CachedArtwork) -> Result<(), ArtworkError> {
        if self.current.as_ref() == Some(&artwork.path) {
            return Ok(());
        }

        let stale = self.previous.take();
        self.previous = self.current.replace(artwork.path.clone());
        if let Some(stale) = stale
            && self.current.as_ref() != Some(&stale)
            && self.previous.as_ref() != Some(&stale)
        {
            match fs::remove_file(stale) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        self.cleanup_unretained()
    }

    fn cleanup_unretained(&self) -> Result<(), ArtworkError> {
        for entry in fs::read_dir(&self.root)? {
            let path = entry?.path();
            let is_artwork = matches!(
                path.extension().and_then(|extension| extension.to_str()),
                Some("jpg" | "png")
            );
            if is_artwork
                && self.current.as_ref() != Some(&path)
                && self.previous.as_ref() != Some(&path)
            {
                fs::remove_file(path)?;
            }
        }
        Ok(())
    }
}

fn write_sync_rename(
    temp: &mut File,
    temp_path: &Path,
    final_path: &Path,
    bytes: &[u8],
) -> Result<(), ArtworkError> {
    temp.write_all(bytes)?;
    temp.sync_all()?;
    fs::rename(temp_path, final_path)?;
    Ok(())
}

fn cached_artwork(path: PathBuf) -> Result<CachedArtwork, ArtworkError> {
    let url = Url::from_file_path(&path)
        .map_err(|()| ArtworkError::InvalidFileUrl(path.clone()))?
        .to_string();
    Ok(CachedArtwork { path, url })
}

fn media_digest(key: &MediaKey) -> String {
    let digest = Sha256::digest(key.0.as_bytes());
    format!("{digest:x}")
}

fn validate_image(bytes: &[u8]) -> Result<ValidatedFormat, ArtworkError> {
    if bytes.len() > MAX_ARTWORK_BYTES {
        return Err(ArtworkError::TooLarge {
            size: bytes.len(),
            limit: MAX_ARTWORK_BYTES,
        });
    }

    let guessed = image::guess_format(bytes).map_err(|_| ArtworkError::UnsupportedFormat)?;
    let format = match guessed {
        ImageFormat::Jpeg => ValidatedFormat::Jpeg,
        ImageFormat::Png => ValidatedFormat::Png,
        _ => return Err(ArtworkError::UnsupportedFormat),
    };
    let dimensions = ImageReader::with_format(Cursor::new(bytes), format.image_format())
        .into_dimensions()
        .map_err(|error| ArtworkError::InvalidImage(error.to_string()))?;
    if dimensions.0 > MAX_ARTWORK_DIMENSION || dimensions.1 > MAX_ARTWORK_DIMENSION {
        return Err(ArtworkError::Dimensions {
            width: dimensions.0,
            height: dimensions.1,
        });
    }

    let decoded = ImageReader::with_format(Cursor::new(bytes), format.image_format())
        .decode()
        .map_err(|error| ArtworkError::InvalidImage(error.to_string()))?;
    let decoded_dimensions = decoded.dimensions();
    if decoded_dimensions != dimensions {
        return Err(ArtworkError::InvalidImage(
            "decoded dimensions differ from image header".into(),
        ));
    }
    Ok(format)
}

/// Resolve the v1 per-user production cache directory without config ownership.
pub fn default_cache_dir() -> Result<PathBuf, ArtworkError> {
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join("Library/Caches/nowplayd"))
            .ok_or(ArtworkError::MissingHome)
    }

    #[cfg(not(target_os = "macos"))]
    {
        if let Some(root) = std::env::var_os("XDG_CACHE_HOME") {
            return Ok(PathBuf::from(root).join("nowplayd"));
        }
        std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join(".cache/nowplayd"))
            .ok_or(ArtworkError::MissingHome)
    }
}

#[derive(Debug)]
struct ArtworkFetch {
    generation: u64,
    key: MediaKey,
    kind: BinaryCommand,
    expected_size: Option<usize>,
    bytes: Vec<u8>,
}

#[derive(Debug)]
enum FetchStep {
    Pending,
    Complete(Option<Vec<u8>>),
}

impl ArtworkFetch {
    fn new(generation: u64, key: MediaKey) -> Self {
        Self {
            generation,
            key,
            kind: BinaryCommand::AlbumArt,
            expected_size: None,
            bytes: Vec::new(),
        }
    }

    async fn step<S>(&mut self, source: &mut S) -> Result<FetchStep, ArtworkError>
    where
        S: BinaryChunkSource,
    {
        let offset = self.bytes.len();
        let response = match source.read_binary(self.kind, &self.key.0, offset).await {
            Ok(response) => response,
            Err(MpdError::Command(failure)) if failure.code == ACK_NO_EXIST => {
                return Ok(self.fallback_or_none());
            }
            Err(error) => return Err(error.into()),
        };

        if response.bytes.is_empty() && response.total_size.unwrap_or_default() == 0 {
            return Ok(self.fallback_or_none());
        }

        let total = response.total_size.ok_or(ArtworkError::MissingSize)?;
        if total > MAX_ARTWORK_BYTES {
            return Err(ArtworkError::TooLarge {
                size: total,
                limit: MAX_ARTWORK_BYTES,
            });
        }
        match self.expected_size {
            Some(expected) if expected != total => {
                return Err(ArtworkError::SizeChanged {
                    expected,
                    actual: total,
                });
            }
            None => {
                self.bytes
                    .try_reserve_exact(total)
                    .map_err(|_| ArtworkError::Allocation { size: total })?;
                self.expected_size = Some(total);
            }
            Some(_) => {}
        }
        if response.bytes.is_empty() {
            return Err(ArtworkError::ZeroProgress { offset, total });
        }
        let end = offset
            .checked_add(response.bytes.len())
            .ok_or(ArtworkError::OffsetOverflow)?;
        if end > total {
            return Err(ArtworkError::Overrun { end, total });
        }
        self.bytes.extend_from_slice(&response.bytes);
        if end == total {
            return Ok(FetchStep::Complete(Some(std::mem::take(&mut self.bytes))));
        }
        Ok(FetchStep::Pending)
    }

    fn fallback_or_none(&mut self) -> FetchStep {
        match self.kind {
            BinaryCommand::AlbumArt => {
                self.kind = BinaryCommand::ReadPicture;
                self.expected_size = None;
                self.bytes.clear();
                FetchStep::Pending
            }
            BinaryCommand::ReadPicture => FetchStep::Complete(None),
        }
    }
}

#[derive(Debug)]
enum PendingWork {
    Lookup { generation: u64, key: MediaKey },
    Fetch(ArtworkFetch),
}

#[derive(Debug)]
enum ArtworkJobKind {
    Lookup {
        generation: u64,
        key: MediaKey,
        cache: ArtworkCache,
    },
    Store {
        generation: u64,
        key: MediaKey,
        cache: ArtworkCache,
        bytes: Vec<u8>,
    },
}

/// Blocking cache/decode work safe to run away from the command owner.
#[derive(Debug)]
pub struct ArtworkJob(ArtworkJobKind);

/// Generation-tagged cache/decode completion returned to the worker.
#[derive(Debug)]
pub struct ArtworkJobResult {
    generation: u64,
    key: MediaKey,
    result: ArtworkJobResultKind,
}

#[derive(Debug)]
enum ArtworkJobResultKind {
    Lookup(Result<Option<CachedArtwork>, ArtworkError>),
    Store(Result<CachedArtwork, ArtworkError>),
}

impl ArtworkJob {
    pub fn run(self) -> ArtworkJobResult {
        match self.0 {
            ArtworkJobKind::Lookup {
                generation,
                key,
                cache,
            } => ArtworkJobResult {
                generation,
                result: ArtworkJobResultKind::Lookup(cache.lookup(&key)),
                key,
            },
            ArtworkJobKind::Store {
                generation,
                key,
                cache,
                bytes,
            } => ArtworkJobResult {
                generation,
                result: ArtworkJobResultKind::Store(cache.store(&key, &bytes)),
                key,
            },
        }
    }
}

/// Result of observing state or performing one artwork scheduling turn.
#[derive(Debug)]
pub struct ArtworkUpdate {
    pub publication: Option<ArtworkPublication>,
    pub warning: Option<String>,
    pub job: Option<ArtworkJob>,
}

impl ArtworkUpdate {
    fn pending() -> Self {
        Self {
            publication: None,
            warning: None,
            job: None,
        }
    }
}

/// Owns application media generation, latest state, current URL, and fetch work.
#[derive(Debug)]
pub struct ArtworkCoordinator {
    cache: ArtworkCache,
    generation: u64,
    current_key: Option<MediaKey>,
    latest_state: PlayerState,
    current_cover_url: Option<String>,
    pending: Option<PendingWork>,
}

impl ArtworkCoordinator {
    pub fn new(cache: ArtworkCache) -> Self {
        Self {
            cache,
            generation: 0,
            current_key: None,
            latest_state: PlayerState::default(),
            current_cover_url: None,
            pending: None,
        }
    }

    /// Observe one coherent MPD snapshot and produce its immediate full publish.
    pub fn observe_state(&mut self, state: PlayerState) -> ArtworkUpdate {
        let media_changed = self.current_key != state.media_key;
        self.latest_state = state;
        if media_changed {
            self.generation = self.generation.wrapping_add(1);
            self.current_key = self.latest_state.media_key.clone();
            self.current_cover_url = None;
            self.pending = None;

            if let Some(key) = self.current_key.clone() {
                self.pending = Some(PendingWork::Lookup {
                    generation: self.generation,
                    key,
                });
            }
        }

        ArtworkUpdate {
            publication: Some(self.publication()),
            warning: None,
            job: None,
        }
    }

    pub fn has_pending_work(&self) -> bool {
        self.pending.is_some()
    }

    /// Perform exactly one cache-completion or MPD-chunk scheduling turn.
    pub async fn step<S>(&mut self, source: &mut S) -> Result<ArtworkUpdate, ArtworkError>
    where
        S: BinaryChunkSource,
    {
        let Some(work) = self.pending.take() else {
            return Ok(ArtworkUpdate::pending());
        };
        match work {
            PendingWork::Lookup { generation, key } => Ok(ArtworkUpdate {
                publication: None,
                warning: None,
                job: Some(ArtworkJob(ArtworkJobKind::Lookup {
                    generation,
                    key,
                    cache: self.cache.clone(),
                })),
            }),
            PendingWork::Fetch(mut fetch) => match fetch.step(source).await {
                Ok(FetchStep::Pending) => {
                    self.pending = Some(PendingWork::Fetch(fetch));
                    Ok(ArtworkUpdate::pending())
                }
                Ok(FetchStep::Complete(Some(bytes))) => Ok(ArtworkUpdate {
                    publication: None,
                    warning: None,
                    job: Some(ArtworkJob(ArtworkJobKind::Store {
                        generation: fetch.generation,
                        key: fetch.key,
                        cache: self.cache.clone(),
                        bytes,
                    })),
                }),
                Ok(FetchStep::Complete(None)) => {
                    Ok(self.apply_no_art(fetch.generation, &fetch.key))
                }
                Err(error) if error.is_transport() => Err(error),
                Err(error) => Ok(self.apply_failure(fetch.generation, &fetch.key, error)),
            },
        }
    }

    /// Apply one background cache/decode result through the media-generation guard.
    pub fn complete_job(&mut self, completed: ArtworkJobResult) -> ArtworkUpdate {
        if !self.is_current(completed.generation, &completed.key) {
            return ArtworkUpdate::pending();
        }
        match completed.result {
            ArtworkJobResultKind::Lookup(Ok(Some(artwork)))
            | ArtworkJobResultKind::Store(Ok(artwork)) => {
                self.apply_artwork(completed.generation, &completed.key, artwork)
            }
            ArtworkJobResultKind::Lookup(Ok(None)) => {
                self.pending = Some(PendingWork::Fetch(ArtworkFetch::new(
                    completed.generation,
                    completed.key,
                )));
                ArtworkUpdate::pending()
            }
            ArtworkJobResultKind::Lookup(Err(error)) => {
                self.pending = Some(PendingWork::Fetch(ArtworkFetch::new(
                    completed.generation,
                    completed.key,
                )));
                ArtworkUpdate {
                    publication: None,
                    warning: Some(format!("cached artwork rejected: {error}; refetching")),
                    job: None,
                }
            }
            ArtworkJobResultKind::Store(Err(error)) => {
                self.apply_failure(completed.generation, &completed.key, error)
            }
        }
    }

    fn apply_artwork(
        &mut self,
        generation: u64,
        key: &MediaKey,
        artwork: CachedArtwork,
    ) -> ArtworkUpdate {
        if !self.is_current(generation, key) {
            return ArtworkUpdate::pending();
        }
        let warning = self
            .cache
            .activate(&artwork)
            .err()
            .map(|error| format!("artwork retention cleanup failed: {error}"));
        self.current_cover_url = Some(artwork.url);
        ArtworkUpdate {
            publication: Some(self.publication()),
            warning,
            job: None,
        }
    }

    fn apply_no_art(&self, generation: u64, key: &MediaKey) -> ArtworkUpdate {
        if !self.is_current(generation, key) {
            return ArtworkUpdate::pending();
        }
        ArtworkUpdate {
            publication: Some(self.publication()),
            warning: None,
            job: None,
        }
    }

    fn apply_failure(&self, generation: u64, key: &MediaKey, error: ArtworkError) -> ArtworkUpdate {
        if !self.is_current(generation, key) {
            return ArtworkUpdate::pending();
        }
        ArtworkUpdate {
            publication: Some(self.publication()),
            warning: Some(error.to_string()),
            job: None,
        }
    }

    fn is_current(&self, generation: u64, key: &MediaKey) -> bool {
        self.generation == generation && self.current_key.as_ref() == Some(key)
    }

    fn publication(&self) -> ArtworkPublication {
        ArtworkPublication {
            state: self.latest_state.clone(),
            cover_url: self.current_cover_url.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use image::{DynamicImage, ImageFormat};
    use tempfile::TempDir;

    use super::*;
    use crate::state::{PlaybackState, SongMetadata};

    struct FakeSource {
        responses: VecDeque<Result<BinaryResponse, MpdError>>,
        requests: Vec<(BinaryCommand, String, usize)>,
    }

    impl BinaryChunkSource for FakeSource {
        async fn read_binary(
            &mut self,
            kind: BinaryCommand,
            uri: &str,
            offset: usize,
        ) -> Result<BinaryResponse, MpdError> {
            self.requests.push((kind, uri.into(), offset));
            self.responses.pop_front().unwrap()
        }
    }

    fn state(key: &str, elapsed: u64) -> PlayerState {
        PlayerState {
            media_key: Some(MediaKey(key.into())),
            metadata: SongMetadata {
                title: Some(key.into()),
                ..SongMetadata::default()
            },
            playback: PlaybackState::Playing,
            elapsed: Some(std::time::Duration::from_secs(elapsed)),
            ..PlayerState::default()
        }
    }

    fn encoded(format: ImageFormat, width: u32, height: u32) -> Vec<u8> {
        let image = DynamicImage::new_rgba8(width, height);
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, format).unwrap();
        bytes.into_inner()
    }

    async fn resolve_lookup(coordinator: &mut ArtworkCoordinator, source: &mut FakeSource) {
        let scheduled = coordinator.step(source).await.unwrap();
        let completed = coordinator.complete_job(scheduled.job.unwrap().run());
        assert!(completed.publication.is_none());
    }

    fn resolve_store(
        coordinator: &mut ArtworkCoordinator,
        scheduled: ArtworkUpdate,
    ) -> ArtworkUpdate {
        coordinator.complete_job(scheduled.job.unwrap().run())
    }

    #[tokio::test]
    async fn each_step_reads_exactly_one_chunk_and_latest_state_reaches_art_publish() {
        let temp = TempDir::new().unwrap();
        let png = encoded(ImageFormat::Png, 2, 2);
        let split = png.len() / 2;
        let mut source = FakeSource {
            responses: VecDeque::from([
                Ok(BinaryResponse {
                    total_size: Some(png.len()),
                    bytes: png[..split].to_vec(),
                }),
                Ok(BinaryResponse {
                    total_size: Some(png.len()),
                    bytes: png[split..].to_vec(),
                }),
            ]),
            requests: Vec::new(),
        };
        let mut coordinator = ArtworkCoordinator::new(ArtworkCache::new(temp.path().into()));

        let first = coordinator.observe_state(state("a.flac", 1));
        assert_eq!(first.publication.unwrap().cover_url, None);
        resolve_lookup(&mut coordinator, &mut source).await;
        assert!(source.requests.is_empty(), "cache lookup never reads MPD");
        assert!(
            coordinator
                .step(&mut source)
                .await
                .unwrap()
                .publication
                .is_none()
        );
        assert_eq!(
            source.requests.len(),
            1,
            "one scheduler turn reads one chunk"
        );

        coordinator.observe_state(state("a.flac", 9));
        let scheduled = coordinator.step(&mut source).await.unwrap();
        let completed = resolve_store(&mut coordinator, scheduled);
        let publication = completed.publication.unwrap();
        assert_eq!(
            publication.state.elapsed,
            Some(std::time::Duration::from_secs(9))
        );
        assert!(publication.cover_url.unwrap().ends_with(".png"));
        assert_eq!(source.requests.len(), 2);
    }

    #[tokio::test]
    async fn media_change_discards_old_fetch_and_publishes_new_key_artless_first() {
        let temp = TempDir::new().unwrap();
        let png = encoded(ImageFormat::Png, 1, 1);
        let mut source = FakeSource {
            responses: VecDeque::from([Ok(BinaryResponse {
                total_size: Some(png.len()),
                bytes: png,
            })]),
            requests: Vec::new(),
        };
        let mut coordinator = ArtworkCoordinator::new(ArtworkCache::new(temp.path().into()));
        coordinator.observe_state(state("a.flac", 0));

        let b = coordinator.observe_state(state("b.flac", 0));
        assert_eq!(b.publication.unwrap().cover_url, None);
        resolve_lookup(&mut coordinator, &mut source).await;
        let scheduled = coordinator.step(&mut source).await.unwrap();
        let completed = resolve_store(&mut coordinator, scheduled);
        assert_eq!(source.requests[0].1, "b.flac");
        assert_eq!(
            completed.publication.unwrap().state.media_key,
            Some(MediaKey("b.flac".into()))
        );
    }

    #[tokio::test]
    async fn disabled_cache_failure_is_nonfatal_and_does_not_refetch_forever() {
        let png = encoded(ImageFormat::Png, 1, 1);
        let mut source = FakeSource {
            responses: VecDeque::from([Ok(BinaryResponse {
                total_size: Some(png.len()),
                bytes: png,
            })]),
            requests: Vec::new(),
        };
        let mut coordinator = ArtworkCoordinator::new(ArtworkCache::disabled());
        coordinator.observe_state(state("a.flac", 0));
        resolve_lookup(&mut coordinator, &mut source).await;

        let scheduled = coordinator.step(&mut source).await.unwrap();
        let failed = resolve_store(&mut coordinator, scheduled);

        assert!(failed.warning.unwrap().contains("cache directory"));
        assert_eq!(failed.publication.unwrap().cover_url, None);
        assert!(!coordinator.has_pending_work());
        assert_eq!(source.requests.len(), 1);
    }

    #[tokio::test]
    async fn new_media_publishes_an_explicit_clear_after_prior_art() {
        let temp = TempDir::new().unwrap();
        let png = encoded(ImageFormat::Png, 1, 1);
        let mut source = FakeSource {
            responses: VecDeque::from([Ok(BinaryResponse {
                total_size: Some(png.len()),
                bytes: png,
            })]),
            requests: Vec::new(),
        };
        let mut coordinator = ArtworkCoordinator::new(ArtworkCache::new(temp.path().into()));
        coordinator.observe_state(state("a.flac", 0));
        resolve_lookup(&mut coordinator, &mut source).await;
        let scheduled = coordinator.step(&mut source).await.unwrap();
        assert!(
            resolve_store(&mut coordinator, scheduled)
                .publication
                .unwrap()
                .cover_url
                .is_some()
        );

        let clear = coordinator.observe_state(state("b.flac", 0));

        assert_eq!(clear.publication.unwrap().cover_url, None);
    }

    #[test]
    fn stale_completion_after_media_change_is_rejected_by_application_generation() {
        let temp = TempDir::new().unwrap();
        let mut coordinator = ArtworkCoordinator::new(ArtworkCache::new(temp.path().into()));
        coordinator.observe_state(state("a.flac", 0));
        let old_generation = coordinator.generation;
        let old_key = MediaKey("a.flac".into());
        let old_art = coordinator
            .cache
            .store(&old_key, &encoded(ImageFormat::Png, 1, 1))
            .unwrap();

        coordinator.observe_state(state("b.flac", 0));
        let stale = coordinator.apply_artwork(old_generation, &old_key, old_art);

        assert!(stale.publication.is_none());
        assert_eq!(coordinator.current_key, Some(MediaKey("b.flac".into())));
        assert_eq!(coordinator.current_cover_url, None);
    }

    #[test]
    fn validation_accepts_only_bounded_jpeg_and_png() {
        assert_eq!(
            validate_image(&encoded(ImageFormat::Jpeg, 2, 2)).unwrap(),
            ValidatedFormat::Jpeg
        );
        assert_eq!(
            validate_image(&encoded(ImageFormat::Png, 2, 2)).unwrap(),
            ValidatedFormat::Png
        );
        assert!(matches!(
            validate_image(b"not an image"),
            Err(ArtworkError::UnsupportedFormat)
        ));
        assert!(matches!(
            validate_image(&encoded(ImageFormat::Png, MAX_ARTWORK_DIMENSION + 1, 1)),
            Err(ArtworkError::Dimensions { .. })
        ));
    }

    #[test]
    fn cache_uses_digest_file_url_and_keeps_current_plus_previous() {
        let temp = TempDir::new().unwrap();
        let mut cache = ArtworkCache::new(temp.path().join("cache with space"));
        let bytes = encoded(ImageFormat::Png, 1, 1);
        let a = cache.store(&MediaKey("a.flac".into()), &bytes).unwrap();
        cache.activate(&a).unwrap();
        let b = cache.store(&MediaKey("b.flac".into()), &bytes).unwrap();
        cache.activate(&b).unwrap();
        let c = cache.store(&MediaKey("c.flac".into()), &bytes).unwrap();
        cache.activate(&c).unwrap();

        assert!(!a.path.exists());
        assert!(b.path.exists());
        assert!(c.path.exists());
        assert_eq!(fs::read_dir(&cache.root).unwrap().count(), 2);
        assert!(c.url.contains("cache%20with%20space"));
        assert_eq!(c.path.file_stem().unwrap().to_string_lossy().len(), 64);
    }

    #[test]
    fn invalid_cached_file_is_rejected_for_refetch_without_partial_publish_path() {
        let temp = TempDir::new().unwrap();
        let cache = ArtworkCache::new(temp.path().into());
        let key = MediaKey("broken.flac".into());
        fs::create_dir_all(temp.path()).unwrap();
        let path = temp.path().join(format!("{}.png", media_digest(&key)));
        fs::write(&path, b"broken").unwrap();

        assert!(cache.lookup(&key).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"broken");
    }
}
