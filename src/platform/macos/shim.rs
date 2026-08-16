//! The only project-local direct MediaPlayer calls (SPEC §4).

use objc2::{
    class, msg_send,
    runtime::{AnyObject, Bool},
};

use crate::platform::PlatformError;

pub(super) trait NativeShim {
    fn disable_change_playback_position(&mut self) -> Result<(), PlatformError>;
    fn clear_now_playing_info(&mut self) -> Result<(), PlatformError>;
}

#[derive(Debug, Default)]
pub(super) struct SystemShim;

impl NativeShim for SystemShim {
    fn disable_change_playback_position(&mut self) -> Result<(), PlatformError> {
        // SAFETY: MediaPlayer is linked by souvlaki. These stable selectors return
        // shared objects, and the adapter invokes this on winit's main thread.
        let disabled = unsafe {
            let center: &AnyObject = msg_send![class!(MPRemoteCommandCenter), sharedCommandCenter];
            let command: &AnyObject = msg_send![center, changePlaybackPositionCommand];
            let _: () = msg_send![command, setEnabled: Bool::NO];
            let enabled: Bool = msg_send![command, isEnabled];
            enabled.is_false()
        };

        disabled
            .then_some(())
            .ok_or(PlatformError::PositionCapabilityStillEnabled)
    }

    fn clear_now_playing_info(&mut self) -> Result<(), PlatformError> {
        // SAFETY: MPNowPlayingInfoCenter is a shared MediaPlayer singleton.
        // Assigning Objective-C nil is the documented true-clear operation.
        let cleared = unsafe {
            let center: &AnyObject = msg_send![class!(MPNowPlayingInfoCenter), defaultCenter];
            let _: () = msg_send![center, setNowPlayingInfo: None::<&AnyObject>];
            let info: Option<&AnyObject> = msg_send![center, nowPlayingInfo];
            info.is_none()
        };

        cleared
            .then_some(())
            .ok_or(PlatformError::NowPlayingInfoNotCleared)
    }
}
