use souvlaki::{MediaControlEvent, MediaControls, MediaPlayback, PlatformConfig};
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    platform::{
        PlatformAdapter, PlatformError, PlatformMetadata, PlatformSnapshot, RemoteCommand,
        command_from_media_event,
    },
    state::PlayerState,
};

use self::shim::{NativeShim, SystemShim};

mod shim;

type EventHandler = Box<dyn Fn(MediaControlEvent) + Send + 'static>;

trait ControlsBackend {
    fn attach(&mut self, handler: EventHandler) -> Result<(), PlatformError>;
    fn detach(&mut self) -> Result<(), PlatformError>;
    fn set_metadata(&mut self, metadata: &PlatformMetadata) -> Result<(), PlatformError>;
    fn set_playback(&mut self, playback: MediaPlayback) -> Result<(), PlatformError>;
}

struct SouvlakiControls(MediaControls);

impl SouvlakiControls {
    fn new() -> Result<Self, PlatformError> {
        Ok(Self(MediaControls::new(PlatformConfig {
            dbus_name: "nowplayd",
            display_name: "nowplayd",
            hwnd: None,
        })?))
    }
}

impl ControlsBackend for SouvlakiControls {
    fn attach(&mut self, handler: EventHandler) -> Result<(), PlatformError> {
        self.0.attach(handler).map_err(Into::into)
    }

    fn detach(&mut self) -> Result<(), PlatformError> {
        self.0.detach().map_err(Into::into)
    }

    fn set_metadata(&mut self, metadata: &PlatformMetadata) -> Result<(), PlatformError> {
        self.0
            .set_metadata(metadata.as_souvlaki())
            .map_err(Into::into)
    }

    fn set_playback(&mut self, playback: MediaPlayback) -> Result<(), PlatformError> {
        self.0.set_playback(playback).map_err(Into::into)
    }
}

struct Adapter<C, N> {
    controls: C,
    native: N,
    attached: bool,
}

impl<C, N> Adapter<C, N>
where
    C: ControlsBackend,
    N: NativeShim,
{
    fn new(controls: C, native: N) -> Self {
        Self {
            controls,
            native,
            attached: false,
        }
    }

    fn attach(&mut self, commands: UnboundedSender<RemoteCommand>) -> Result<(), PlatformError> {
        if self.attached {
            self.controls.detach()?;
            self.attached = false;
        }

        self.controls.attach(Box::new(move |event| {
            if let Some(command) = command_from_media_event(&event) {
                let _ = commands.send(command);
            }
        }))?;

        if let Err(error) = self.native.disable_change_playback_position() {
            let _ = self.controls.detach();
            return Err(error);
        }

        self.attached = true;
        Ok(())
    }

    fn publish(&mut self, state: &PlayerState) -> Result<(), PlatformError> {
        let snapshot = PlatformSnapshot::from(state);
        // set_metadata replaces the full dictionary; playback and progress must
        // always be restored after it (SPEC §5.2).
        self.controls.set_metadata(&snapshot.metadata)?;
        self.controls.set_playback(snapshot.playback)
    }

    fn clear(&mut self) -> Result<(), PlatformError> {
        // Advance souvlaki's metadata/art generation before assigning native nil.
        // Attachment lifetime is intentionally independent: no detach here.
        self.controls.set_metadata(&PlatformMetadata::default())?;
        self.native.clear_now_playing_info()
    }
}

/// macOS platform owner. Construct and use only on winit's main thread.
pub struct MacPlatform {
    adapter: Adapter<SouvlakiControls, SystemShim>,
}

impl MacPlatform {
    pub fn new(commands: UnboundedSender<RemoteCommand>) -> Result<Self, PlatformError> {
        let mut adapter = Adapter::new(SouvlakiControls::new()?, SystemShim);
        adapter.attach(commands)?;
        Ok(Self { adapter })
    }
}

impl PlatformAdapter for MacPlatform {
    fn publish(&mut self, state: &PlayerState) -> Result<(), PlatformError> {
        self.adapter.publish(state)
    }

    fn clear(&mut self) -> Result<(), PlatformError> {
        self.adapter.clear()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::state::{PlaybackState, SongMetadata};

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Call {
        Attach,
        Detach,
        Metadata { empty: bool },
        Playback,
        DisablePosition,
        NativeClear,
    }

    #[derive(Default)]
    struct FakeState {
        calls: Vec<Call>,
        generation: usize,
        delayed_art_generation: Option<usize>,
        now_playing_present: bool,
    }

    #[derive(Clone, Default)]
    struct FakeControls(Arc<Mutex<FakeState>>);

    impl FakeControls {
        fn schedule_delayed_art(&self) {
            let mut state = self.0.lock().unwrap();
            state.delayed_art_generation = Some(state.generation);
            state.now_playing_present = true;
        }

        fn complete_delayed_art(&self) {
            let mut state = self.0.lock().unwrap();
            if state.delayed_art_generation == Some(state.generation) {
                state.now_playing_present = true;
            }
        }
    }

    impl ControlsBackend for FakeControls {
        fn attach(&mut self, _handler: EventHandler) -> Result<(), PlatformError> {
            self.0.lock().unwrap().calls.push(Call::Attach);
            Ok(())
        }

        fn detach(&mut self) -> Result<(), PlatformError> {
            self.0.lock().unwrap().calls.push(Call::Detach);
            Ok(())
        }

        fn set_metadata(&mut self, metadata: &PlatformMetadata) -> Result<(), PlatformError> {
            let mut state = self.0.lock().unwrap();
            state.generation += 1;
            state.calls.push(Call::Metadata {
                empty: metadata == &PlatformMetadata::default(),
            });
            state.now_playing_present = true;
            Ok(())
        }

        fn set_playback(&mut self, _playback: MediaPlayback) -> Result<(), PlatformError> {
            self.0.lock().unwrap().calls.push(Call::Playback);
            Ok(())
        }
    }

    #[derive(Clone)]
    struct FakeShim(Arc<Mutex<FakeState>>);

    impl NativeShim for FakeShim {
        fn disable_change_playback_position(&mut self) -> Result<(), PlatformError> {
            self.0.lock().unwrap().calls.push(Call::DisablePosition);
            Ok(())
        }

        fn clear_now_playing_info(&mut self) -> Result<(), PlatformError> {
            let mut state = self.0.lock().unwrap();
            state.calls.push(Call::NativeClear);
            state.now_playing_present = false;
            Ok(())
        }
    }

    fn test_adapter() -> (
        Adapter<FakeControls, FakeShim>,
        FakeControls,
        Arc<Mutex<FakeState>>,
    ) {
        let state = Arc::new(Mutex::new(FakeState::default()));
        let controls = FakeControls(state.clone());
        (
            Adapter::new(controls.clone(), FakeShim(state.clone())),
            controls,
            state,
        )
    }

    #[test]
    fn attach_always_disables_position_after_souvlaki_attach() {
        let (mut adapter, _controls, state) = test_adapter();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        adapter.attach(tx.clone()).unwrap();
        assert_eq!(
            state.lock().unwrap().calls,
            [Call::Attach, Call::DisablePosition]
        );

        state.lock().unwrap().calls.clear();
        adapter.attach(tx).unwrap();
        assert_eq!(
            state.lock().unwrap().calls,
            [Call::Detach, Call::Attach, Call::DisablePosition]
        );
    }

    #[test]
    fn full_publish_restores_playback_after_metadata() {
        let (mut adapter, _controls, state) = test_adapter();
        let player = PlayerState {
            metadata: SongMetadata {
                title: Some("Track".into()),
                artists: vec!["Artist".into()],
                album: Some("Album".into()),
            },
            playback: PlaybackState::Playing,
            ..PlayerState::default()
        };

        adapter.publish(&player).unwrap();

        assert_eq!(
            state.lock().unwrap().calls,
            [Call::Metadata { empty: false }, Call::Playback]
        );
    }

    #[test]
    fn clear_advances_generation_then_nil_without_detaching_or_stale_art() {
        let (mut adapter, controls, state) = test_adapter();
        controls.schedule_delayed_art();

        adapter.clear().unwrap();
        controls.complete_delayed_art();

        let state = state.lock().unwrap();
        assert_eq!(
            state.calls,
            [Call::Metadata { empty: true }, Call::NativeClear]
        );
        assert!(!state.now_playing_present);
    }
}
