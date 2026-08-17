//! Platform-neutral publication and remote-command seams.

use std::{error::Error, fmt, future::Future};

use souvlaki::{MediaControlEvent, MediaMetadata, MediaPlayback, MediaPosition};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::{
    artwork::ArtworkPublication,
    mpd::{CommandConnection, LiveCommandConnection, LivenessClock, MpdError},
    state::{PlaybackState, PlayerState},
};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "linux")]
pub use linux::LinuxPlatform as SystemPlatform;
#[cfg(target_os = "macos")]
pub use macos::MacPlatform as SystemPlatform;

/// The only remote commands accepted by the v1 MPD bridge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteCommand {
    Toggle,
    Play,
    Pause,
    Next,
    Previous,
}

impl fmt::Display for RemoteCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Toggle => "toggle",
            Self::Play => "play",
            Self::Pause => "pause",
            Self::Next => "next",
            Self::Previous => "previous",
        })
    }
}

/// Semantic command events. IMP-006 replaces only the stderr sink.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandOutcome {
    Received(RemoteCommand),
    Succeeded(RemoteCommand),
    Failed {
        command: RemoteCommand,
        error: String,
    },
}

/// Events sent from the Tokio worker to the platform owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkerEvent {
    Publish(ArtworkPublication),
    Command(CommandOutcome),
    ClearForTest,
    Fatal(String),
}

/// Owned projection used by every platform backend.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlatformMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration: Option<std::time::Duration>,
    pub cover_url: Option<String>,
}

impl PlatformMetadata {
    pub fn as_souvlaki(&self) -> MediaMetadata<'_> {
        MediaMetadata {
            title: self.title.as_deref(),
            artist: self.artist.as_deref(),
            album: self.album.as_deref(),
            duration: self.duration,
            cover_url: self.cover_url.as_deref(),
        }
    }
}

/// A full platform projection. Publication never accepts a partial update.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlatformSnapshot {
    pub metadata: PlatformMetadata,
    pub playback: MediaPlayback,
}

impl From<&ArtworkPublication> for PlatformSnapshot {
    fn from(publication: &ArtworkPublication) -> Self {
        let state = &publication.state;
        let artist =
            (!state.metadata.artists.is_empty()).then(|| state.metadata.artists.join("; "));
        let progress = state.elapsed.map(MediaPosition);
        let playback = match state.playback {
            PlaybackState::Playing => MediaPlayback::Playing { progress },
            PlaybackState::Paused => MediaPlayback::Paused { progress },
            PlaybackState::Stopped => MediaPlayback::Stopped,
        };

        Self {
            metadata: PlatformMetadata {
                title: state.metadata.title.clone(),
                artist,
                album: state.metadata.album.clone(),
                duration: state.duration,
                cover_url: publication.cover_url.clone(),
            },
            playback,
        }
    }
}

/// Fallible platform operation. Native probe assertions never enter production.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlatformError {
    Backend(String),
    PositionCapabilityStillEnabled,
    NowPlayingInfoNotCleared,
}

impl fmt::Display for PlatformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Backend(message) => write!(f, "platform backend error: {message}"),
            Self::PositionCapabilityStillEnabled => {
                f.write_str("playback-position capability remained enabled")
            }
            Self::NowPlayingInfoNotCleared => f.write_str("Now Playing info remained present"),
        }
    }
}

impl Error for PlatformError {}

impl From<souvlaki::Error> for PlatformError {
    fn from(error: souvlaki::Error) -> Self {
        Self::Backend(error.to_string())
    }
}

/// Main-thread/system-service-owned publication boundary.
pub trait PlatformAdapter {
    fn publish(&mut self, publication: &ArtworkPublication) -> Result<(), PlatformError>;
    fn clear(&mut self) -> Result<(), PlatformError>;
}

/// Async command target used to test the command-to-refresh transaction.
pub trait RemoteCommandTarget {
    type Error: fmt::Display;

    fn execute(&mut self, command: RemoteCommand) -> impl Future<Output = Result<(), Self::Error>>;

    fn refresh(&mut self) -> impl Future<Output = Result<PlayerState, Self::Error>>;
}

impl<IO> RemoteCommandTarget for CommandConnection<IO>
where
    IO: AsyncRead + AsyncWrite + Unpin,
{
    type Error = MpdError;

    async fn execute(&mut self, command: RemoteCommand) -> Result<(), Self::Error> {
        match command {
            RemoteCommand::Toggle => self.toggle().await,
            RemoteCommand::Play => self.play().await,
            RemoteCommand::Pause => self.pause().await,
            RemoteCommand::Next => self.next().await,
            RemoteCommand::Previous => self.previous().await,
        }
    }

    async fn refresh(&mut self) -> Result<PlayerState, Self::Error> {
        CommandConnection::refresh(self).await
    }
}

impl<C> RemoteCommandTarget for LiveCommandConnection<C>
where
    C: LivenessClock,
{
    type Error = MpdError;

    async fn execute(&mut self, command: RemoteCommand) -> Result<(), Self::Error> {
        match command {
            RemoteCommand::Toggle => self.toggle().await,
            RemoteCommand::Play => self.play().await,
            RemoteCommand::Pause => self.pause().await,
            RemoteCommand::Next => self.next().await,
            RemoteCommand::Previous => self.previous().await,
        }
    }

    async fn refresh(&mut self) -> Result<PlayerState, Self::Error> {
        LiveCommandConnection::refresh(self).await
    }
}

/// Execute one callback transaction and return the coherent refreshed state.
///
/// The worker owns the subsequent full publication because it must combine
/// this state with the application-owned artwork generation and current URL.
pub async fn handle_remote_command<T, F>(
    target: &mut T,
    command: RemoteCommand,
    emit: &mut F,
) -> Result<Option<PlayerState>, String>
where
    T: RemoteCommandTarget,
    F: FnMut(WorkerEvent) -> Result<(), String>,
{
    emit(WorkerEvent::Command(CommandOutcome::Received(command)))?;

    if let Err(error) = target.execute(command).await {
        emit(WorkerEvent::Command(CommandOutcome::Failed {
            command,
            error: error.to_string(),
        }))?;
        return Ok(None);
    }

    emit(WorkerEvent::Command(CommandOutcome::Succeeded(command)))?;
    let state = target
        .refresh()
        .await
        .map_err(|error| format!("refresh after {command} failed: {error}"))?;
    Ok(Some(state))
}

pub(crate) fn command_from_media_event(event: &MediaControlEvent) -> Option<RemoteCommand> {
    match event {
        MediaControlEvent::Toggle => Some(RemoteCommand::Toggle),
        MediaControlEvent::Play => Some(RemoteCommand::Play),
        MediaControlEvent::Pause => Some(RemoteCommand::Pause),
        MediaControlEvent::Next => Some(RemoteCommand::Next),
        MediaControlEvent::Previous => Some(RemoteCommand::Previous),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;
    use crate::state::{OccurrenceId, SongMetadata};

    #[test]
    fn only_the_five_ruled_events_cross_the_bridge() {
        assert_eq!(
            command_from_media_event(&MediaControlEvent::Toggle),
            Some(RemoteCommand::Toggle)
        );
        assert_eq!(
            command_from_media_event(&MediaControlEvent::Play),
            Some(RemoteCommand::Play)
        );
        assert_eq!(
            command_from_media_event(&MediaControlEvent::Pause),
            Some(RemoteCommand::Pause)
        );
        assert_eq!(
            command_from_media_event(&MediaControlEvent::Next),
            Some(RemoteCommand::Next)
        );
        assert_eq!(
            command_from_media_event(&MediaControlEvent::Previous),
            Some(RemoteCommand::Previous)
        );
        assert_eq!(
            command_from_media_event(&MediaControlEvent::SetPosition(MediaPosition(
                std::time::Duration::from_secs(10)
            ))),
            None
        );
        assert_eq!(command_from_media_event(&MediaControlEvent::Stop), None);
    }

    #[test]
    fn full_projection_joins_artists_in_source_order() {
        let state = PlayerState {
            metadata: SongMetadata {
                title: Some("Track".into()),
                artists: vec!["First Artist".into(), "Second Artist".into()],
                album: Some("Album".into()),
            },
            playback: PlaybackState::Playing,
            elapsed: Some(std::time::Duration::from_secs(12)),
            duration: Some(std::time::Duration::from_secs(180)),
            ..PlayerState::default()
        };

        let projected = PlatformSnapshot::from(&ArtworkPublication {
            state,
            cover_url: Some("file:///tmp/cover.png".into()),
        });
        assert_eq!(
            projected.metadata.artist.as_deref(),
            Some("First Artist; Second Artist")
        );
        assert_eq!(
            projected.metadata.cover_url.as_deref(),
            Some("file:///tmp/cover.png")
        );
        assert_eq!(
            projected.playback,
            MediaPlayback::Playing {
                progress: Some(MediaPosition(std::time::Duration::from_secs(12)))
            }
        );

        let empty = PlatformSnapshot::from(&ArtworkPublication {
            state: PlayerState::default(),
            cover_url: None,
        });
        assert_eq!(empty.metadata.artist, None);
    }

    struct FakeTarget {
        calls: Vec<&'static str>,
        execute_results: VecDeque<Result<(), &'static str>>,
        refreshed: PlayerState,
    }

    impl RemoteCommandTarget for FakeTarget {
        type Error = &'static str;

        async fn execute(&mut self, _command: RemoteCommand) -> Result<(), Self::Error> {
            self.calls.push("execute");
            self.execute_results.pop_front().unwrap_or(Ok(()))
        }

        async fn refresh(&mut self) -> Result<PlayerState, Self::Error> {
            self.calls.push("refresh");
            Ok(self.refreshed.clone())
        }
    }

    #[tokio::test]
    async fn success_emits_receipt_result_then_returns_refresh_for_full_publish() {
        let refreshed = PlayerState {
            occurrence: Some(OccurrenceId(9)),
            ..PlayerState::default()
        };
        let mut target = FakeTarget {
            calls: Vec::new(),
            execute_results: VecDeque::from([Ok(())]),
            refreshed: refreshed.clone(),
        };
        let mut events = Vec::new();

        let returned = handle_remote_command(&mut target, RemoteCommand::Play, &mut |event| {
            events.push(event);
            Ok(())
        })
        .await
        .unwrap()
        .unwrap();

        assert_eq!(target.calls, ["execute", "refresh"]);
        assert_eq!(returned, refreshed);
        assert_eq!(
            events,
            [
                WorkerEvent::Command(CommandOutcome::Received(RemoteCommand::Play)),
                WorkerEvent::Command(CommandOutcome::Succeeded(RemoteCommand::Play)),
            ]
        );
    }

    #[tokio::test]
    async fn command_failure_emits_no_refresh_or_optimistic_publish() {
        let mut target = FakeTarget {
            calls: Vec::new(),
            execute_results: VecDeque::from([Err("No next song")]),
            refreshed: PlayerState::default(),
        };
        let mut events = Vec::new();

        handle_remote_command(&mut target, RemoteCommand::Next, &mut |event| {
            events.push(event);
            Ok(())
        })
        .await
        .unwrap();

        assert_eq!(target.calls, ["execute"]);
        assert_eq!(
            events,
            [
                WorkerEvent::Command(CommandOutcome::Received(RemoteCommand::Next)),
                WorkerEvent::Command(CommandOutcome::Failed {
                    command: RemoteCommand::Next,
                    error: "No next song".into(),
                }),
            ]
        );
    }
}
