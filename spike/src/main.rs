use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use signal_hook::{
    consts::{SIGINT, SIGTERM},
    iterator::Signals,
};
use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
};
use url::Url;
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
    window::WindowId,
};

const LOG_PATH: &str = "/tmp/nowplayd-spike.log";

#[derive(Debug)]
enum ProbeEvent {
    Remote(MediaControlEvent),
    Shutdown(i32),
}

struct Probe {
    controls: Option<MediaControls>,
    cleared: bool,
    is_playing: bool,
    proxy: EventLoopProxy<ProbeEvent>,
}

impl Probe {
    fn new(proxy: EventLoopProxy<ProbeEvent>) -> Self {
        Self {
            controls: None,
            cleared: false,
            is_playing: true,
            proxy,
        }
    }

    fn start(&mut self) {
        if self.controls.is_some() {
            return;
        }

        let artwork_url = fixture_url().expect("locate fixture artwork");
        let mut controls = MediaControls::new(PlatformConfig {
            dbus_name: "org.nowplayd.Spike",
            display_name: "nowplayd spike",
            hwnd: None,
        })
        .expect("create media controls");

        let proxy = self.proxy.clone();
        controls
            .attach(move |event| {
                if proxy.send_event(ProbeEvent::Remote(event)).is_err() {
                    log_line("remote event dropped: event loop closed");
                }
            })
            .expect("attach media controls");
        controls
            .set_metadata(MediaMetadata {
                title: Some("Windowless Now Playing Spike"),
                artist: Some("nowplayd"),
                album: Some("AI-IMP-001 Architecture Gate"),
                duration: Some(Duration::from_secs(300)),
                cover_url: Some(artwork_url.as_str()),
            })
            .expect("publish static metadata and artwork");
        controls
            .set_playback(MediaPlayback::Playing {
                progress: Some(MediaPosition(Duration::from_secs(42))),
            })
            .expect("publish playback state");

        let scrubber_disabled = native::disable_change_playback_position();
        log_line(&format!(
            "probe active: window=false artwork={} scrubber_disabled={scrubber_disabled}",
            artwork_url
        ));
        assert!(scrubber_disabled, "native scrubber disable probe failed");
        self.controls = Some(controls);
    }

    fn handle_remote_event(&mut self, event: MediaControlEvent) {
        log_remote_event(&event);
        let Some(is_playing) = playback_state_after_event(self.is_playing, &event) else {
            return;
        };
        self.is_playing = is_playing;

        let progress = Some(MediaPosition(Duration::from_secs(42)));
        let playback = if is_playing {
            MediaPlayback::Playing { progress }
        } else {
            MediaPlayback::Paused { progress }
        };
        self.controls
            .as_mut()
            .expect("controls exist while callbacks are attached")
            .set_playback(playback)
            .expect("publish playback state after remote event");
    }

    fn clear(&mut self) {
        if self.cleared {
            return;
        }

        if let Some(controls) = self.controls.as_mut() {
            // Advance souvlaki's artwork generation before the native nil assignment.
            // Otherwise its asynchronous artwork loader can resurrect cleared metadata.
            controls
                .set_metadata(MediaMetadata {
                    title: None,
                    artist: None,
                    album: None,
                    duration: None,
                    cover_url: None,
                })
                .expect("advance metadata generation before clear");
            controls.detach().expect("detach media controls");
        }

        let cleared = native::clear_now_playing_info();
        log_line(&format!("probe clearing: now_playing_info_nil={cleared}"));
        assert!(cleared, "native Now Playing clear probe failed");
        self.cleared = true;
    }
}

impl ApplicationHandler<ProbeEvent> for Probe {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {
        self.start();
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: ProbeEvent) {
        match event {
            ProbeEvent::Remote(event) => self.handle_remote_event(event),
            ProbeEvent::Shutdown(signal) => {
                log_line(&format!("received signal {signal}; shutting down"));
                self.clear();
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        _event: WindowEvent,
    ) {
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.clear();
    }
}

fn main() {
    let event_loop = EventLoop::<ProbeEvent>::with_user_event()
        .build()
        .expect("create event loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    let proxy = event_loop.create_proxy();
    install_signal_forwarder(proxy.clone());
    event_loop
        .run_app(&mut Probe::new(proxy))
        .expect("run windowless event loop");
}

fn install_signal_forwarder(proxy: winit::event_loop::EventLoopProxy<ProbeEvent>) {
    let mut signals = Signals::new([SIGTERM, SIGINT]).expect("register termination signals");
    thread::spawn(move || {
        if let Some(signal) = signals.forever().next() {
            let _ = proxy.send_event(ProbeEvent::Shutdown(signal));
        }
    });
}

fn fixture_url() -> Result<Url, String> {
    let path = bundled_fixture_path().unwrap_or_else(|| {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixture.jpg")
            .to_path_buf()
    });
    Url::from_file_path(&path).map_err(|()| format!("invalid fixture path: {}", path.display()))
}

fn bundled_fixture_path() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let contents = executable.parent()?.parent()?;
    let candidate = contents.join("Resources/fixture.jpg");
    candidate.is_file().then_some(candidate)
}

fn log_remote_event(event: &MediaControlEvent) {
    let name = match event {
        MediaControlEvent::Toggle => "toggle",
        MediaControlEvent::Play => "play",
        MediaControlEvent::Pause => "pause",
        MediaControlEvent::Next => "next",
        MediaControlEvent::Previous => "previous",
        other => {
            log_line(&format!("remote event (unexpected): {other:?}"));
            return;
        }
    };
    log_line(&format!("remote event: {name}"));
}

fn playback_state_after_event(current: bool, event: &MediaControlEvent) -> Option<bool> {
    match event {
        MediaControlEvent::Toggle => Some(!current),
        MediaControlEvent::Play => Some(true),
        MediaControlEvent::Pause => Some(false),
        MediaControlEvent::Next | MediaControlEvent::Previous => None,
        _ => None,
    }
}

fn log_line(message: &str) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let timestamp = format!("{}.{:03}", now.as_secs(), now.subsec_millis());
    let line = format!("{timestamp} {message}");

    match OpenOptions::new().create(true).append(true).open(LOG_PATH) {
        Ok(mut file) => {
            let _ = writeln!(file, "{line}");
        }
        Err(error) => eprintln!("could not append {LOG_PATH}: {error}"),
    }
}

#[cfg(target_os = "macos")]
mod native {
    use objc2::{
        class, msg_send,
        runtime::{AnyObject, Bool},
    };

    pub fn disable_change_playback_position() -> bool {
        // SAFETY: MediaPlayer is linked by souvlaki. Both selectors are stable macOS
        // MediaPlayer API, return shared objects, and this runs on winit's main thread.
        unsafe {
            let center: &AnyObject = msg_send![class!(MPRemoteCommandCenter), sharedCommandCenter];
            let command: &AnyObject = msg_send![center, changePlaybackPositionCommand];
            let _: () = msg_send![command, setEnabled: Bool::NO];
            let enabled: Bool = msg_send![command, isEnabled];
            enabled.is_false()
        }
    }

    pub fn clear_now_playing_info() -> bool {
        // SAFETY: MPNowPlayingInfoCenter is a shared MediaPlayer singleton. Passing
        // Objective-C nil is the documented way to clear nowPlayingInfo.
        unsafe {
            let center: &AnyObject = msg_send![class!(MPNowPlayingInfoCenter), defaultCenter];
            let _: () = msg_send![center, setNowPlayingInfo: None::<&AnyObject>];
            let info: Option<&AnyObject> = msg_send![center, nowPlayingInfo];
            info.is_none()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_transport_events_update_the_published_playback_state() {
        assert_eq!(
            playback_state_after_event(true, &MediaControlEvent::Pause),
            Some(false)
        );
        assert_eq!(
            playback_state_after_event(false, &MediaControlEvent::Play),
            Some(true)
        );
        assert_eq!(
            playback_state_after_event(true, &MediaControlEvent::Toggle),
            Some(false)
        );
        assert_eq!(
            playback_state_after_event(false, &MediaControlEvent::Toggle),
            Some(true)
        );
        assert_eq!(
            playback_state_after_event(true, &MediaControlEvent::Next),
            None
        );
        assert_eq!(
            playback_state_after_event(true, &MediaControlEvent::Previous),
            None
        );
    }
}

#[cfg(not(target_os = "macos"))]
mod native {
    pub fn disable_change_playback_position() -> bool {
        false
    }

    pub fn clear_now_playing_info() -> bool {
        false
    }
}
