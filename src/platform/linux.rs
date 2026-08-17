use souvlaki::{MediaControls, MediaPlayback, PlatformConfig};
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    artwork::ArtworkPublication,
    platform::{
        PlatformAdapter, PlatformError, PlatformMetadata, PlatformSnapshot, PublicationIntent,
        RemoteCommand, command_from_media_event,
    },
};

/// Linux MPRIS pass-through. The macOS-only shim is intentionally absent.
pub struct LinuxPlatform {
    controls: MediaControls,
}

impl LinuxPlatform {
    pub fn new(commands: UnboundedSender<RemoteCommand>) -> Result<Self, PlatformError> {
        let mut controls = MediaControls::new(PlatformConfig {
            dbus_name: "nowplayd",
            display_name: "nowplayd",
            hwnd: None,
        })?;
        controls.attach(move |event| {
            if let Some(command) = command_from_media_event(&event) {
                let _ = commands.send(command);
            }
        })?;
        Ok(Self { controls })
    }
}

impl PlatformAdapter for LinuxPlatform {
    fn publish(&mut self, publication: &ArtworkPublication) -> Result<(), PlatformError> {
        let snapshot = PlatformSnapshot::from(publication);
        if publication.intent == PublicationIntent::FullMetadata {
            self.controls
                .set_metadata(snapshot.metadata.as_souvlaki())?;
        }
        self.controls.set_playback(snapshot.playback)?;
        Ok(())
    }

    fn clear(&mut self) -> Result<(), PlatformError> {
        self.controls
            .set_metadata(PlatformMetadata::default().as_souvlaki())?;
        self.controls.set_playback(MediaPlayback::Stopped)?;
        Ok(())
    }
}
