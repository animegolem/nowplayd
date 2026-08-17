use nowplayd::{
    artwork::{
        ArtworkCache, ArtworkCoordinator, ArtworkJobResult, ArtworkUpdate, default_cache_dir,
    },
    mpd::{ConnectionConfig, IdleConnection, LiveCommandConnection, Subsystem},
    platform::{
        CommandOutcome, RemoteCommand, RemoteCommandTarget, WorkerEvent, handle_remote_command,
    },
    state::PlayerState,
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

enum WorkerInput {
    Remote(Option<RemoteCommand>),
    Idle(Option<Result<Vec<Subsystem>, String>>),
    ArtworkJob(Option<ArtworkJobResult>),
    Artwork,
}

async fn next_worker_input(
    remote_commands: &mut UnboundedReceiver<RemoteCommand>,
    idle_events: &mut UnboundedReceiver<Result<Vec<Subsystem>, String>>,
    artwork_jobs: &mut UnboundedReceiver<ArtworkJobResult>,
    artwork_pending: bool,
) -> WorkerInput {
    tokio::select! {
        biased;
        remote = remote_commands.recv() => WorkerInput::Remote(remote),
        idle = idle_events.recv() => WorkerInput::Idle(idle),
        completed = artwork_jobs.recv() => WorkerInput::ArtworkJob(completed),
        () = std::future::ready(()), if artwork_pending => WorkerInput::Artwork,
    }
}

fn handle_artwork_update<F>(
    update: ArtworkUpdate,
    emit: &mut F,
    job_tx: &UnboundedSender<ArtworkJobResult>,
) -> Result<(), String>
where
    F: FnMut(WorkerEvent) -> Result<(), String>,
{
    if let Some(warning) = update.warning {
        eprintln!("artwork failure: {warning}");
    }
    if let Some(publication) = update.publication {
        emit(WorkerEvent::Publish(publication))?;
    }
    if let Some(job) = update.job {
        let completion_tx = job_tx.clone();
        tokio::task::spawn_blocking(move || {
            let _ = completion_tx.send(job.run());
        });
    }
    Ok(())
}

async fn handle_remote_with_artwork<T, F>(
    target: &mut T,
    artwork: &mut ArtworkCoordinator,
    command: RemoteCommand,
    emit: &mut F,
    job_tx: &UnboundedSender<ArtworkJobResult>,
) -> Result<Option<PlayerState>, String>
where
    T: RemoteCommandTarget,
    F: FnMut(WorkerEvent) -> Result<(), String>,
{
    let Some(refreshed) = handle_remote_command(target, command, emit).await? else {
        return Ok(None);
    };
    handle_artwork_update(artwork.observe_state(refreshed.clone()), emit, job_tx)?;
    Ok(Some(refreshed))
}

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
    let cache = match default_cache_dir() {
        Ok(cache_root) => ArtworkCache::new(cache_root),
        Err(error) => {
            eprintln!("artwork failure: {error}; artwork cache disabled");
            ArtworkCache::disabled()
        }
    };
    let mut artwork = ArtworkCoordinator::new(cache);
    let (artwork_job_tx, mut artwork_job_rx) = unbounded_channel();
    handle_artwork_update(
        artwork.observe_state(state.clone()),
        &mut emit,
        &artwork_job_tx,
    )?;

    loop {
        match next_worker_input(
            &mut remote_commands,
            &mut idle_rx,
            &mut artwork_job_rx,
            artwork.has_pending_work(),
        )
        .await
        {
            WorkerInput::Remote(remote) => {
                let command =
                    remote.ok_or_else(|| "platform command channel closed".to_string())?;
                if let Some(refreshed) = handle_remote_with_artwork(
                    &mut commands,
                    &mut artwork,
                    command,
                    &mut emit,
                    &artwork_job_tx,
                )
                .await?
                {
                    state = refreshed;
                }
            }
            WorkerInput::Idle(idle_result) => {
                let _subsystems =
                    idle_result.ok_or_else(|| "MPD idle event channel closed".to_string())??;
                let newer = commands
                    .refresh()
                    .await
                    .map_err(|error| error.to_string())?;
                if state.diff(&newer).any() {
                    handle_artwork_update(
                        artwork.observe_state(newer.clone()),
                        &mut emit,
                        &artwork_job_tx,
                    )?;
                }
                state = newer;
            }
            WorkerInput::ArtworkJob(completed) => {
                let completed =
                    completed.ok_or_else(|| "artwork job completion channel closed".to_string())?;
                handle_artwork_update(artwork.complete_job(completed), &mut emit, &artwork_job_tx)?;
            }
            WorkerInput::Artwork => {
                let update = artwork
                    .step(&mut commands)
                    .await
                    .map_err(|error| error.to_string())?;
                handle_artwork_update(update, &mut emit, &artwork_job_tx)?;
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
        artwork::ArtworkPublication,
        platform::{PlatformAdapter, SystemPlatform},
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
        pending_publish: Option<ArtworkPublication>,
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

        fn publish_or_exit(
            &mut self,
            event_loop: &ActiveEventLoop,
            publication: ArtworkPublication,
        ) {
            if self.test_clear_latched {
                return;
            }
            let Some(adapter) = self.adapter.as_mut() else {
                self.pending_publish = Some(publication);
                return;
            };
            if let Err(error) = adapter.publish(&publication) {
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

            if let Some(publication) = self.pending_publish.take() {
                self.publish_or_exit(event_loop, publication);
            }
            if std::mem::take(&mut self.pending_clear) {
                self.clear_or_exit(event_loop);
            }
        }

        fn user_event(&mut self, event_loop: &ActiveEventLoop, event: WorkerEvent) {
            match event {
                WorkerEvent::Publish(publication) => self.publish_or_exit(event_loop, publication),
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
        WorkerEvent::Publish(publication) => platform
            .publish(&publication)
            .map_err(|error| error.to_string()),
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

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;
    use nowplayd::{
        artwork::{ArtworkPublication, BinaryChunkSource},
        mpd::{BinaryCommand, BinaryResponse, MpdError},
        state::MediaKey,
    };
    use tempfile::TempDir;

    struct FakeTarget {
        calls: Vec<&'static str>,
        results: VecDeque<Result<(), &'static str>>,
        refreshed: PlayerState,
    }

    impl RemoteCommandTarget for FakeTarget {
        type Error = &'static str;

        async fn execute(&mut self, _command: RemoteCommand) -> Result<(), Self::Error> {
            self.calls.push("execute");
            self.results.pop_front().unwrap_or(Ok(()))
        }

        async fn refresh(&mut self) -> Result<PlayerState, Self::Error> {
            self.calls.push("refresh");
            Ok(self.refreshed.clone())
        }
    }

    struct FakeArtworkSource {
        responses: VecDeque<BinaryResponse>,
        requests: Vec<usize>,
    }

    impl BinaryChunkSource for FakeArtworkSource {
        async fn read_binary(
            &mut self,
            _kind: BinaryCommand,
            _uri: &str,
            offset: usize,
        ) -> Result<BinaryResponse, MpdError> {
            self.requests.push(offset);
            Ok(self.responses.pop_front().unwrap())
        }
    }

    #[tokio::test]
    async fn successful_remote_command_refreshes_then_emits_art_aware_full_publish() {
        let refreshed = PlayerState {
            media_key: Some(nowplayd::state::MediaKey("track.flac".into())),
            ..PlayerState::default()
        };
        let mut target = FakeTarget {
            calls: Vec::new(),
            results: VecDeque::from([Ok(())]),
            refreshed: refreshed.clone(),
        };
        let temp = TempDir::new().unwrap();
        let mut artwork = ArtworkCoordinator::new(ArtworkCache::new(temp.path().into()));
        let (job_tx, _job_rx) = unbounded_channel();
        let mut events = Vec::new();

        let returned = handle_remote_with_artwork(
            &mut target,
            &mut artwork,
            RemoteCommand::Play,
            &mut |event| {
                events.push(event);
                Ok(())
            },
            &job_tx,
        )
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
                WorkerEvent::Publish(ArtworkPublication {
                    state: refreshed,
                    cover_url: None,
                    intent: nowplayd::platform::PublicationIntent::FullMetadata,
                }),
            ]
        );
    }

    #[tokio::test]
    async fn remote_and_refresh_issued_mid_fetch_precede_the_next_artwork_chunk() {
        let (remote_tx, mut remote_rx) = unbounded_channel();
        let (idle_tx, mut idle_rx) = unbounded_channel();
        let (_job_tx, mut job_rx) = unbounded_channel();
        let temp = TempDir::new().unwrap();
        let mut artwork = ArtworkCoordinator::new(ArtworkCache::new(temp.path().into()));
        artwork.observe_state(PlayerState {
            media_key: Some(MediaKey("track.flac".into())),
            ..PlayerState::default()
        });
        let mut source = FakeArtworkSource {
            responses: VecDeque::from([
                BinaryResponse {
                    total_size: Some(4),
                    bytes: vec![1, 2],
                },
                BinaryResponse {
                    total_size: Some(4),
                    bytes: vec![3, 4],
                },
            ]),
            requests: Vec::new(),
        };

        let lookup = artwork.step(&mut source).await.unwrap();
        artwork.complete_job(lookup.job.unwrap().run());

        let mut order = Vec::new();
        match next_worker_input(&mut remote_rx, &mut idle_rx, &mut job_rx, true).await {
            WorkerInput::Artwork => {
                artwork.step(&mut source).await.unwrap();
                order.push("chunk N");
            }
            _ => panic!("artwork must begin when no command or refresh is queued"),
        }
        assert_eq!(source.requests, [0]);

        remote_tx.send(RemoteCommand::Next).unwrap();
        idle_tx.send(Ok(vec![Subsystem::Player])).unwrap();
        match next_worker_input(&mut remote_rx, &mut idle_rx, &mut job_rx, true).await {
            WorkerInput::Remote(Some(RemoteCommand::Next)) => {
                order.extend(["remote command", "coherent refresh", "full publish"]);
            }
            _ => panic!("remote command must have first priority"),
        }
        match next_worker_input(&mut remote_rx, &mut idle_rx, &mut job_rx, true).await {
            WorkerInput::Idle(Some(Ok(_))) => order.push("pending state refresh"),
            _ => panic!("pending refresh must precede artwork"),
        }
        match next_worker_input(&mut remote_rx, &mut idle_rx, &mut job_rx, true).await {
            WorkerInput::Artwork => {
                artwork.step(&mut source).await.unwrap();
                order.push("chunk N+1");
            }
            _ => panic!("artwork must run after queued command and refresh work"),
        }
        assert_eq!(source.requests, [0, 2]);

        assert_eq!(
            order,
            [
                "chunk N",
                "remote command",
                "coherent refresh",
                "full publish",
                "pending state refresh",
                "chunk N+1",
            ]
        );
    }
}
