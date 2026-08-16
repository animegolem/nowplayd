use nowplayd::{
    mpd::{ConnectionConfig, IdleConnection, LiveCommandConnection},
    platform::{CommandOutcome, RemoteCommand, WorkerEvent, handle_remote_command},
};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

async fn run_worker<F>(
    mut remote_commands: UnboundedReceiver<RemoteCommand>,
    mut emit: F,
) -> Result<(), String>
where
    F: FnMut(WorkerEvent) -> Result<(), String>,
{
    let config = ConnectionConfig::default();
    let mut commands = LiveCommandConnection::connect(config.clone())
        .await
        .map_err(|error| error.to_string())?;
    let mut idle = IdleConnection::connect(&config)
        .await
        .map_err(|error| error.to_string())?;
    let (idle_tx, mut idle_rx) = unbounded_channel();
    tokio::spawn(async move {
        loop {
            let event = idle.next_event().await.map_err(|error| error.to_string());
            let failed = event.is_err();
            if idle_tx.send(event).is_err() || failed {
                return;
            }
        }
    });
    let mut state = commands
        .refresh()
        .await
        .map_err(|error| error.to_string())?;
    emit(WorkerEvent::Publish(state.clone()))?;

    loop {
        tokio::select! {
            idle_result = idle_rx.recv() => {
                let _subsystems = idle_result
                    .ok_or_else(|| "MPD idle event channel closed".to_string())??;
                let newer = commands.refresh().await.map_err(|error| error.to_string())?;
                if state.diff(&newer).any() {
                    emit(WorkerEvent::Publish(newer.clone()))?;
                }
                state = newer;
            }
            remote = remote_commands.recv() => {
                let command = remote.ok_or_else(|| "platform command channel closed".to_string())?;
                if let Some(refreshed) =
                    handle_remote_command(&mut commands, command, &mut emit).await?
                {
                    state = refreshed;
                }
            }
        }
    }
}

fn log_command_outcome(outcome: &CommandOutcome) {
    match outcome {
        CommandOutcome::Received(command) => eprintln!("remote command received: {command}"),
        CommandOutcome::Succeeded(command) => eprintln!("remote command succeeded: {command}"),
        CommandOutcome::Failed { command, error } => {
            eprintln!("remote command failed: {command}: {error}")
        }
    }
}

#[cfg(target_os = "macos")]
mod macos_main {
    use std::{error::Error, thread, time::Duration};

    use nowplayd::{
        platform::{PlatformAdapter, SystemPlatform},
        state::PlayerState,
    };
    use tokio::runtime::Builder;
    use winit::{
        application::ApplicationHandler,
        event::WindowEvent,
        event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
        window::WindowId,
    };

    use super::*;

    pub fn run() -> Result<(), Box<dyn Error>> {
        let event_loop = EventLoop::<WorkerEvent>::with_user_event().build()?;
        event_loop.set_control_flow(ControlFlow::Wait);
        let proxy = event_loop.create_proxy();
        let (command_tx, command_rx) = unbounded_channel();

        spawn_worker(proxy.clone(), command_rx);
        install_clear_test_hook(proxy.clone())?;

        let mut app = App::new(command_tx);
        event_loop.run_app(&mut app)?;
        Ok(())
    }

    fn spawn_worker(
        proxy: EventLoopProxy<WorkerEvent>,
        command_rx: UnboundedReceiver<RemoteCommand>,
    ) {
        thread::spawn(move || {
            let runtime = match Builder::new_multi_thread().enable_all().build() {
                Ok(runtime) => runtime,
                Err(error) => {
                    let _ = proxy.send_event(WorkerEvent::Fatal(format!(
                        "create Tokio worker runtime: {error}"
                    )));
                    return;
                }
            };

            let event_proxy = proxy.clone();
            let result = runtime.block_on(run_worker(command_rx, move |event| {
                event_proxy
                    .send_event(event)
                    .map_err(|_| "platform event loop closed".to_string())
            }));
            if let Err(error) = result {
                let _ = proxy.send_event(WorkerEvent::Fatal(error));
            }
        });
    }

    fn install_clear_test_hook(proxy: EventLoopProxy<WorkerEvent>) -> Result<(), Box<dyn Error>> {
        let Some(delay) = std::env::var_os("NOWPLAYD_M3_CLEAR_AFTER_MS") else {
            return Ok(());
        };
        let delay = delay.to_string_lossy().parse::<u64>()?;
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(delay));
            let _ = proxy.send_event(WorkerEvent::ClearForTest);
        });
        Ok(())
    }

    struct App {
        adapter: Option<SystemPlatform>,
        command_tx: tokio::sync::mpsc::UnboundedSender<RemoteCommand>,
        pending_publish: Option<PlayerState>,
        pending_clear: bool,
        test_clear_latched: bool,
    }

    impl App {
        fn new(command_tx: tokio::sync::mpsc::UnboundedSender<RemoteCommand>) -> Self {
            Self {
                adapter: None,
                command_tx,
                pending_publish: None,
                pending_clear: false,
                test_clear_latched: false,
            }
        }

        fn publish_or_exit(&mut self, event_loop: &ActiveEventLoop, state: PlayerState) {
            if self.test_clear_latched {
                return;
            }
            let Some(adapter) = self.adapter.as_mut() else {
                self.pending_publish = Some(state);
                return;
            };
            if let Err(error) = adapter.publish(&state) {
                eprintln!("platform publish failed: {error}");
                event_loop.exit();
            }
        }

        fn clear_or_exit(&mut self, event_loop: &ActiveEventLoop) {
            // This is an M3-only owner test hook, not lifecycle policy. Once
            // exercised, keep later MPD events from obscuring whether native
            // nil truly removed the entry while the controls remain attached.
            self.test_clear_latched = true;
            self.pending_publish = None;
            let Some(adapter) = self.adapter.as_mut() else {
                self.pending_clear = true;
                return;
            };
            match adapter.clear() {
                Ok(()) => eprintln!(
                    "M3 test hook: Now Playing cleared; controls remain attached; test publications latched"
                ),
                Err(error) => {
                    eprintln!("platform clear failed: {error}");
                    event_loop.exit();
                }
            }
        }
    }

    impl ApplicationHandler<WorkerEvent> for App {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            if self.adapter.is_some() {
                return;
            }
            match SystemPlatform::new(self.command_tx.clone()) {
                Ok(adapter) => self.adapter = Some(adapter),
                Err(error) => {
                    eprintln!("platform attach failed: {error}");
                    event_loop.exit();
                    return;
                }
            }

            if let Some(state) = self.pending_publish.take() {
                self.publish_or_exit(event_loop, state);
            }
            if std::mem::take(&mut self.pending_clear) {
                self.clear_or_exit(event_loop);
            }
        }

        fn user_event(&mut self, event_loop: &ActiveEventLoop, event: WorkerEvent) {
            match event {
                WorkerEvent::Publish(state) => self.publish_or_exit(event_loop, state),
                WorkerEvent::Command(outcome) => log_command_outcome(&outcome),
                WorkerEvent::ClearForTest => self.clear_or_exit(event_loop),
                WorkerEvent::Fatal(error) => {
                    eprintln!("worker failed: {error}");
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
    }
}

#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    macos_main::run()
}

#[cfg(target_os = "linux")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use nowplayd::platform::{PlatformAdapter, SystemPlatform};

    let (command_tx, command_rx) = unbounded_channel();
    let mut platform = SystemPlatform::new(command_tx)?;
    run_worker(command_rx, move |event| match event {
        WorkerEvent::Publish(state) => platform.publish(&state).map_err(|error| error.to_string()),
        WorkerEvent::Command(outcome) => {
            log_command_outcome(&outcome);
            Ok(())
        }
        WorkerEvent::ClearForTest => platform.clear().map_err(|error| error.to_string()),
        WorkerEvent::Fatal(error) => Err(error),
    })
    .await
    .map_err(Into::into)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!("nowplayd currently supports macOS and Linux targets");
