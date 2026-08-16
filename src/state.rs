//! Platform-independent representation of MPD playback state.

use std::time::Duration;

/// Identity of one occurrence in MPD's current queue.
///
/// MPD does not preserve this value across daemon restarts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct OccurrenceId(pub u64);

/// Durable identity of the media resource, currently its MPD URI.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MediaKey(pub String);

/// Playback state reported by MPD.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlaybackState {
    Playing,
    Paused,
    #[default]
    Stopped,
}

/// Display metadata for the current song.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SongMetadata {
    pub title: Option<String>,
    pub artists: Vec<String>,
    pub album: Option<String>,
}

/// A coherent snapshot of the current MPD player state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlayerState {
    pub occurrence: Option<OccurrenceId>,
    pub media_key: Option<MediaKey>,
    pub metadata: SongMetadata,
    pub playback: PlaybackState,
    pub elapsed: Option<Duration>,
    pub duration: Option<Duration>,
}

/// Categories that changed between two coherent snapshots.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StateChange {
    pub metadata: bool,
    pub playback: bool,
    pub occurrence: bool,
    pub media_key: bool,
}

impl StateChange {
    pub fn any(self) -> bool {
        self.metadata || self.playback || self.occurrence || self.media_key
    }
}

impl PlayerState {
    /// Compare this snapshot with a newer coherent snapshot.
    pub fn diff(&self, newer: &Self) -> StateChange {
        StateChange {
            metadata: self.metadata != newer.metadata || self.duration != newer.duration,
            playback: self.playback != newer.playback || self.elapsed != newer.elapsed,
            occurrence: self.occurrence != newer.occurrence,
            media_key: self.media_key != newer.media_key,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> PlayerState {
        PlayerState {
            occurrence: Some(OccurrenceId(4)),
            media_key: Some(MediaKey("album/track.flac".into())),
            metadata: SongMetadata {
                title: Some("Track".into()),
                artists: vec!["Artist".into()],
                album: Some("Album".into()),
            },
            playback: PlaybackState::Playing,
            elapsed: Some(Duration::from_secs(10)),
            duration: Some(Duration::from_secs(180)),
        }
    }

    #[test]
    fn metadata_only_change_is_classified() {
        let before = sample();
        let mut after = before.clone();
        after.metadata.title = Some("Retagged".into());

        assert_eq!(
            before.diff(&after),
            StateChange {
                metadata: true,
                ..StateChange::default()
            }
        );
    }

    #[test]
    fn playback_only_change_is_classified() {
        let before = sample();
        let mut after = before.clone();
        after.playback = PlaybackState::Paused;
        after.elapsed = Some(Duration::from_secs(11));

        assert_eq!(
            before.diff(&after),
            StateChange {
                playback: true,
                ..StateChange::default()
            }
        );
    }

    #[test]
    fn duplicate_uri_changes_only_occurrence() {
        let before = sample();
        let mut after = before.clone();
        after.occurrence = Some(OccurrenceId(5));

        assert_eq!(
            before.diff(&after),
            StateChange {
                occurrence: true,
                ..StateChange::default()
            }
        );
    }

    #[test]
    fn restart_id_change_preserves_durable_media_key() {
        let before = sample();
        let mut after_restart = before.clone();
        after_restart.occurrence = Some(OccurrenceId(9001));

        let change = before.diff(&after_restart);
        assert!(change.occurrence);
        assert!(!change.media_key);
        assert_eq!(before.media_key, after_restart.media_key);
    }

    #[test]
    fn media_change_is_independent_of_occurrence() {
        let before = sample();
        let mut after = before.clone();
        after.media_key = Some(MediaKey("album/other.flac".into()));

        assert_eq!(
            before.diff(&after),
            StateChange {
                media_key: true,
                ..StateChange::default()
            }
        );
    }

    #[test]
    fn duration_is_metadata_but_elapsed_is_playback() {
        let before = sample();
        let mut after = before.clone();
        after.duration = Some(Duration::from_secs(181));
        after.elapsed = Some(Duration::from_secs(12));

        assert_eq!(
            before.diff(&after),
            StateChange {
                metadata: true,
                playback: true,
                ..StateChange::default()
            }
        );
    }
}
