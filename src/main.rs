use std::{path::PathBuf, sync::Arc};

use nowplayd::{
    artwork::{
        ArtworkCache, ArtworkCoordinator, ArtworkError, ArtworkJobResult, ArtworkUpdate,
        default_cache_dir,
    },
    config::AppConfig,
    lifecycle::{
        BackoffPolicy, FailureDisposition, JitterRng, LifecycleClock, LifecycleEvent, LifecycleLog,
        LifecycleSleeper, StderrLifecycleLog, SupervisorState, SystemJitter, SystemLifecycleClock,
        TokioSleeper, classify_failure,
    },
    logging::{self, TracingLifecycleLog},
    mpd::{ConnectionConfig, IdleConnection, LiveCommandConnection, MpdError, Subsystem},
    platform::{
        CommandOutcome, RemoteCommand, RemoteCommandError, WorkerEvent, handle_remote_command,
    },
    state::PlayerState,
};
use tokio::{
    sync::{
        mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel},
        oneshot, watch,
    },
    task::JoinHandle,
};

struct RuntimeInputs {
    connection: ConnectionConfig,
    cache_root: Option<PathBuf>,
    log: Arc<dyn LifecycleLog>,
}

impl Default for RuntimeInputs {
    fn default() -> Self {
        let cache_root = match default_cache_dir() {
            Ok(root) => Some(root),
            Err(error) => {
                eprintln!("artwork failure: {error}; artwork cache disabled");
                None
            }
        };
        Self {
            connection: ConnectionConfig::default(),
            cache_root,
            log: Arc::new(StderrLifecycleLog),
        }
    }
}

impl From<AppConfig> for RuntimeInputs {
    fn from(config: AppConfig) -> Self {
        Self {
            connection: config.connection,
            cache_root: Some(config.cache_dir),
            log: Arc::new(TracingLifecycleLog),
        }
    }
}

fn configured_runtime() -> Result<(RuntimeInputs, bool), Box<dyn std::error::Error>> {
    let check_only = match std::env::args().skip(1).collect::<Vec<_>>().as_slice() {
        [] => false,
        [argument] if argument == "--check-config" => true,
        _ => return Err("usage: nowplayd [--check-config]".into()),
    };
    let config = AppConfig::load()?;
    logging::init(&config.log_level)?;
    logging::log_startup_config(&config);
    if check_only {
        tracing::info!("configuration preflight passed");
    }
    Ok((config.into(), check_only))
}

enum WorkerInput {
    Shutdown,
    Remote(Option<RemoteCommand>),
    Idle(Option<Result<Vec<Subsystem>, MpdError>>),
    ArtworkJob(Option<ArtworkJobResult>),
    Artwork,
}

struct LivePair {
    commands: LiveCommandConnection,
    idle_events: UnboundedReceiver<Result<Vec<Subsystem>, MpdError>>,
    idle_task: JoinHandle<()>,
}

enum SessionEnd {
    Shutdown,
    Fault(MpdError),
    Terminal(String),
}

struct ConnectedContext<'a, C, R, F> {
    artwork: &'a mut ArtworkCoordinator,
    remote_commands: &'a mut UnboundedReceiver<RemoteCommand>,
    shutdown: &'a mut watch::Receiver<bool>,
    emit: &'a mut F,
    backoff: &'a mut BackoffPolicy<C, R>,
    log: &'a dyn LifecycleLog,
    platform_may_be_present: &'a mut bool,
}

async fn next_worker_input(
    shutdown: &mut watch::Receiver<bool>,
    remote_commands: &mut UnboundedReceiver<RemoteCommand>,
    idle_events: &mut UnboundedReceiver<Result<Vec<Subsystem>, MpdError>>,
    artwork_jobs: &mut UnboundedReceiver<ArtworkJobResult>,
    artwork_pending: bool,
) -> WorkerInput {
    if *shutdown.borrow() {
        return WorkerInput::Shutdown;
    }
    tokio::select! {
        biased;
        _ = shutdown.changed() => WorkerInput::Shutdown,
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
        tracing::warn!(reason = %warning, "artwork failure");
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

async fn establish_pair(config: &ConnectionConfig) -> Result<(LivePair, PlayerState), MpdError> {
    let mut commands = LiveCommandConnection::connect(config.clone()).await?;
    let mut idle = IdleConnection::connect(config).await?;
    let (idle_tx, idle_events) = unbounded_channel();
    let idle_task = tokio::spawn(async move {
        loop {
            let event = idle.next_event().await;
            let failed = event
                .as_ref()
                .is_err_and(|error| classify_failure(error) == FailureDisposition::TearDownPair);
            if idle_tx.send(event).is_err() || failed {
                return;
            }
        }
    });
    match commands.refresh().await {
        Ok(state) => Ok((
            LivePair {
                commands,
                idle_events,
                idle_task,
            },
            state,
        )),
        Err(error) => {
            abort_idle_task(idle_task).await;
            Err(error)
        }
    }
}

async fn teardown_pair(pair: LivePair) {
    abort_idle_task(pair.idle_task).await;
}

async fn abort_idle_task(idle_task: JoinHandle<()>) {
    idle_task.abort();
    let _ = idle_task.await;
}

async fn request_clear<F>(emit: &mut F, log: &dyn LifecycleLog, reason: &str) -> Result<(), String>
where
    F: FnMut(WorkerEvent) -> Result<(), String>,
{
    log.record(&LifecycleEvent::ClearRequested {
        reason: reason.into(),
    });
    let (acknowledgement, response) = oneshot::channel();
    emit(WorkerEvent::Clear { acknowledgement })?;
    response
        .await
        .map_err(|_| "platform dropped clear acknowledgement".to_string())??;
    log.record(&LifecycleEvent::ClearAcknowledged {
        reason: reason.into(),
    });
    Ok(())
}

fn note_successful_refresh<C, R>(backoff: &mut BackoffPolicy<C, R>, log: &dyn LifecycleLog)
where
    C: LifecycleClock,
    R: JitterRng,
{
    if backoff.successful_refresh() {
        log.record(&LifecycleEvent::HealthyBackoffReset);
    }
}

async fn apply_refreshed_state<C, R, F>(
    newer: PlayerState,
    state: &mut PlayerState,
    artwork_job_tx: &UnboundedSender<ArtworkJobResult>,
    force_no_song_clear: bool,
    context: &mut ConnectedContext<'_, C, R, F>,
) -> Result<(), String>
where
    C: LifecycleClock,
    R: JitterRng,
    F: FnMut(WorkerEvent) -> Result<(), String>,
{
    if newer.media_key.is_none() {
        let transitioned_from_song = state.media_key.is_some();
        context.artwork.invalidate_epoch();
        if (force_no_song_clear || transitioned_from_song) && *context.platform_may_be_present {
            request_clear(context.emit, context.log, "no current song").await?;
            *context.platform_may_be_present = false;
        }
    } else if state.diff(&newer).any() || state.media_key.is_none() {
        handle_artwork_update(
            context.artwork.observe_state(newer.clone()),
            context.emit,
            artwork_job_tx,
        )?;
        *context.platform_may_be_present = true;
    }
    *state = newer;
    Ok(())
}

async fn run_connected<C, R, F>(
    pair: &mut LivePair,
    initial_state: PlayerState,
    context: &mut ConnectedContext<'_, C, R, F>,
) -> SessionEnd
where
    C: LifecycleClock,
    R: JitterRng,
    F: FnMut(WorkerEvent) -> Result<(), String>,
{
    let (artwork_job_tx, mut artwork_job_rx) = unbounded_channel();
    let mut state = PlayerState::default();
    if let Err(error) =
        apply_refreshed_state(initial_state, &mut state, &artwork_job_tx, true, context).await
    {
        return SessionEnd::Terminal(error);
    }
    note_successful_refresh(context.backoff, context.log);

    loop {
        match next_worker_input(
            context.shutdown,
            context.remote_commands,
            &mut pair.idle_events,
            &mut artwork_job_rx,
            context.artwork.has_pending_work(),
        )
        .await
        {
            WorkerInput::Shutdown => return SessionEnd::Shutdown,
            WorkerInput::Remote(Some(command)) => {
                match handle_remote_command(&mut pair.commands, command, context.emit).await {
                    Ok(Some(newer)) => {
                        note_successful_refresh(context.backoff, context.log);
                        if let Err(error) = apply_refreshed_state(
                            newer,
                            &mut state,
                            &artwork_job_tx,
                            false,
                            context,
                        )
                        .await
                        {
                            return SessionEnd::Terminal(error);
                        }
                    }
                    Ok(None) => {}
                    Err(RemoteCommandError::Mpd(error)) => match classify_failure(&error) {
                        FailureDisposition::KeepPair => {
                            tracing::warn!(reason = %error, "remote refresh command rejected");
                        }
                        FailureDisposition::TearDownPair => return SessionEnd::Fault(error),
                    },
                    Err(RemoteCommandError::Emit(error)) => return SessionEnd::Terminal(error),
                }
            }
            WorkerInput::Remote(None) => {
                return SessionEnd::Terminal("platform command channel closed".into());
            }
            WorkerInput::Idle(Some(Ok(_subsystems))) => match pair.commands.refresh().await {
                Ok(newer) => {
                    note_successful_refresh(context.backoff, context.log);
                    if let Err(error) =
                        apply_refreshed_state(newer, &mut state, &artwork_job_tx, false, context)
                            .await
                    {
                        return SessionEnd::Terminal(error);
                    }
                }
                Err(error) => match classify_failure(&error) {
                    FailureDisposition::KeepPair => {
                        tracing::warn!(reason = %error, "state refresh command rejected");
                    }
                    FailureDisposition::TearDownPair => return SessionEnd::Fault(error),
                },
            },
            WorkerInput::Idle(Some(Err(error))) => match classify_failure(&error) {
                FailureDisposition::KeepPair => {
                    tracing::warn!(reason = %error, "idle command rejected");
                }
                FailureDisposition::TearDownPair => return SessionEnd::Fault(error),
            },
            WorkerInput::Idle(None) => {
                return SessionEnd::Fault(
                    std::io::Error::new(
                        std::io::ErrorKind::BrokenPipe,
                        "MPD idle task ended without a typed result",
                    )
                    .into(),
                );
            }
            WorkerInput::ArtworkJob(Some(completed)) => {
                if let Err(error) = handle_artwork_update(
                    context.artwork.complete_job(completed),
                    context.emit,
                    &artwork_job_tx,
                ) {
                    return SessionEnd::Terminal(error);
                }
            }
            WorkerInput::ArtworkJob(None) => {
                return SessionEnd::Terminal("artwork job channel closed".into());
            }
            WorkerInput::Artwork => match context.artwork.step(&mut pair.commands).await {
                Ok(update) => {
                    if let Err(error) = handle_artwork_update(update, context.emit, &artwork_job_tx)
                    {
                        return SessionEnd::Terminal(error);
                    }
                }
                Err(ArtworkError::Mpd(error)) => return SessionEnd::Fault(error),
                Err(error) => return SessionEnd::Terminal(error.to_string()),
            },
        }
    }
}

async fn shutdown_requested(shutdown: &mut watch::Receiver<bool>) {
    if *shutdown.borrow() {
        return;
    }
    let _ = shutdown.changed().await;
}

async fn run_worker_with<C, R, S, F>(
    inputs: RuntimeInputs,
    mut remote_commands: UnboundedReceiver<RemoteCommand>,
    mut shutdown: watch::Receiver<bool>,
    mut emit: F,
    clock: C,
    rng: R,
    sleeper: S,
) -> Result<(), String>
where
    C: LifecycleClock,
    R: JitterRng,
    S: LifecycleSleeper,
    F: FnMut(WorkerEvent) -> Result<(), String>,
{
    let cache = inputs
        .cache_root
        .map_or_else(ArtworkCache::disabled, ArtworkCache::new);
    let mut artwork = ArtworkCoordinator::new(cache);
    let mut backoff = BackoffPolicy::new(clock, rng);
    let mut state = SupervisorState::Reconnecting;
    let mut platform_may_be_present = true;

    loop {
        debug_assert_eq!(state, SupervisorState::Reconnecting);
        inputs.log.record(&LifecycleEvent::Connecting);
        let established = tokio::select! {
            _ = shutdown_requested(&mut shutdown) => {
                inputs.log.record(&LifecycleEvent::ShuttingDown);
                artwork.invalidate_epoch();
                if platform_may_be_present {
                    request_clear(&mut emit, inputs.log.as_ref(), "shutdown").await?;
                }
                return Ok(());
            }
            established = establish_pair(&inputs.connection) => established,
        };

        match established {
            Ok((mut pair, initial_state)) => {
                state = SupervisorState::Connected;
                debug_assert_eq!(state, SupervisorState::Connected);
                backoff.entered_connected();
                inputs.log.record(&LifecycleEvent::Connected);
                let mut context = ConnectedContext {
                    artwork: &mut artwork,
                    remote_commands: &mut remote_commands,
                    shutdown: &mut shutdown,
                    emit: &mut emit,
                    backoff: &mut backoff,
                    log: inputs.log.as_ref(),
                    platform_may_be_present: &mut platform_may_be_present,
                };
                let end = run_connected(&mut pair, initial_state, &mut context).await;
                teardown_pair(pair).await;

                match end {
                    SessionEnd::Shutdown => {
                        state = SupervisorState::ShuttingDown;
                        inputs.log.record(&LifecycleEvent::ShuttingDown);
                        artwork.invalidate_epoch();
                        if platform_may_be_present {
                            request_clear(&mut emit, inputs.log.as_ref(), "shutdown").await?;
                        }
                        debug_assert_eq!(state, SupervisorState::ShuttingDown);
                        return Ok(());
                    }
                    SessionEnd::Terminal(error) => return Err(error),
                    SessionEnd::Fault(error) => {
                        inputs.log.record(&LifecycleEvent::PairFault {
                            reason: error.to_string(),
                        });
                    }
                }
            }
            Err(error) => {
                inputs.log.record(&LifecycleEvent::PairFault {
                    reason: error.to_string(),
                });
            }
        }

        state = SupervisorState::Reconnecting;
        backoff.disconnected();
        artwork.invalidate_epoch();
        if platform_may_be_present {
            request_clear(&mut emit, inputs.log.as_ref(), "MPD connection fault").await?;
            platform_may_be_present = false;
        }
        let delay = backoff.next_delay();
        inputs.log.record(&LifecycleEvent::Backoff {
            attempt: delay.attempt,
            delay: delay.actual,
        });
        tokio::select! {
            _ = shutdown_requested(&mut shutdown) => {
                inputs.log.record(&LifecycleEvent::ShuttingDown);
                return Ok(());
            }
            () = sleeper.sleep(delay.actual) => {}
        }
    }
}

async fn run_worker<F>(
    inputs: RuntimeInputs,
    remote_commands: UnboundedReceiver<RemoteCommand>,
    shutdown: watch::Receiver<bool>,
    emit: F,
) -> Result<(), String>
where
    F: FnMut(WorkerEvent) -> Result<(), String>,
{
    run_worker_with(
        inputs,
        remote_commands,
        shutdown,
        emit,
        SystemLifecycleClock,
        SystemJitter::default(),
        TokioSleeper,
    )
    .await
}

async fn wait_for_shutdown_signal(shutdown: watch::Sender<bool>) {
    use tokio::signal::unix::{SignalKind, signal};

    let mut interrupt = match signal(SignalKind::interrupt()) {
        Ok(signal) => signal,
        Err(error) => {
            tracing::error!(reason = %error, "install SIGINT handler failed");
            return;
        }
    };
    let mut terminate = match signal(SignalKind::terminate()) {
        Ok(signal) => signal,
        Err(error) => {
            tracing::error!(reason = %error, "install SIGTERM handler failed");
            return;
        }
    };
    tokio::select! {
        _ = interrupt.recv() => {}
        _ = terminate.recv() => {}
    }
    let _ = shutdown.send(true);
}

fn log_command_outcome(outcome: &CommandOutcome) {
    match outcome {
        CommandOutcome::Received(command) => {
            tracing::info!(command = %command, "remote command received");
        }
        CommandOutcome::Succeeded(command) => {
            tracing::info!(command = %command, "remote command succeeded");
        }
        CommandOutcome::Failed {
            command,
            class,
            error,
        } => {
            tracing::warn!(command = %command, class = ?class, reason = %error, "remote command failed")
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

    pub fn run(inputs: RuntimeInputs) -> Result<(), Box<dyn Error>> {
        let event_loop = EventLoop::<WorkerEvent>::with_user_event().build()?;
        event_loop.set_control_flow(ControlFlow::Wait);
        let proxy = event_loop.create_proxy();
        let (command_tx, command_rx) = unbounded_channel();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        spawn_worker(
            proxy.clone(),
            command_rx,
            shutdown_tx.clone(),
            shutdown_rx,
            inputs,
        );
        install_clear_test_hook(proxy.clone())?;

        let mut app = App::new(command_tx, shutdown_tx);
        event_loop.run_app(&mut app)?;
        Ok(())
    }

    fn spawn_worker(
        proxy: EventLoopProxy<WorkerEvent>,
        command_rx: UnboundedReceiver<RemoteCommand>,
        shutdown_tx: watch::Sender<bool>,
        shutdown_rx: watch::Receiver<bool>,
        inputs: RuntimeInputs,
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
            let result = runtime.block_on(async move {
                let signal_task = tokio::spawn(wait_for_shutdown_signal(shutdown_tx));
                let result = run_worker(inputs, command_rx, shutdown_rx, move |event| {
                    event_proxy
                        .send_event(event)
                        .map_err(|_| "platform event loop closed".to_string())
                })
                .await;
                signal_task.abort();
                result
            });
            match result {
                Ok(()) => {
                    let _ = proxy.send_event(WorkerEvent::ShutdownComplete);
                }
                Err(error) => {
                    let _ = proxy.send_event(WorkerEvent::Fatal(error));
                }
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
        shutdown_tx: watch::Sender<bool>,
        pending_publish: Option<ArtworkPublication>,
        pending_clear: Option<oneshot::Sender<Result<(), String>>>,
        test_clear_latched: bool,
        now_playing_cleared: bool,
    }

    impl App {
        fn new(
            command_tx: tokio::sync::mpsc::UnboundedSender<RemoteCommand>,
            shutdown_tx: watch::Sender<bool>,
        ) -> Self {
            Self {
                adapter: None,
                command_tx,
                shutdown_tx,
                pending_publish: None,
                pending_clear: None,
                test_clear_latched: false,
                now_playing_cleared: true,
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
                tracing::error!(reason = %error, "platform publish failed");
                event_loop.exit();
            } else {
                self.now_playing_cleared = false;
            }
        }

        fn clear_now(&mut self) -> Result<(), String> {
            self.pending_publish = None;
            let adapter = self
                .adapter
                .as_mut()
                .ok_or_else(|| "platform adapter is not ready".to_string())?;
            adapter.clear().map_err(|error| error.to_string())?;
            self.now_playing_cleared = true;
            Ok(())
        }

        fn acknowledge_clear(&mut self, acknowledgement: oneshot::Sender<Result<(), String>>) {
            if self.adapter.is_none() {
                self.pending_publish = None;
                self.pending_clear = Some(acknowledgement);
                return;
            }
            let result = self.clear_now();
            let _ = acknowledgement.send(result);
        }

        fn clear_test_or_exit(&mut self, event_loop: &ActiveEventLoop) {
            // This is an M3-only owner test hook, not lifecycle policy. Once
            // exercised, keep later MPD events from obscuring whether native
            // nil truly removed the entry while the controls remain attached.
            self.test_clear_latched = true;
            if self.adapter.is_none() {
                return;
            }
            match self.clear_now() {
                Ok(()) => tracing::info!(
                    "M3 test hook: Now Playing cleared; controls remain attached; test publications latched"
                ),
                Err(error) => {
                    tracing::error!(reason = %error, "platform clear failed");
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
                    tracing::error!(reason = %error, "platform attach failed");
                    event_loop.exit();
                    return;
                }
            }

            if let Some(publication) = self.pending_publish.take() {
                self.publish_or_exit(event_loop, publication);
            }
            if let Some(acknowledgement) = self.pending_clear.take() {
                self.acknowledge_clear(acknowledgement);
            }
        }

        fn user_event(&mut self, event_loop: &ActiveEventLoop, event: WorkerEvent) {
            match event {
                WorkerEvent::Publish(publication) => self.publish_or_exit(event_loop, publication),
                WorkerEvent::Command(outcome) => log_command_outcome(&outcome),
                WorkerEvent::Clear { acknowledgement } => {
                    self.acknowledge_clear(acknowledgement);
                }
                WorkerEvent::ShutdownComplete => event_loop.exit(),
                WorkerEvent::ClearForTest => self.clear_test_or_exit(event_loop),
                WorkerEvent::Fatal(error) => {
                    tracing::error!(reason = %error, "worker failed");
                    if !self.now_playing_cleared
                        && let Err(clear_error) = self.clear_now()
                    {
                        tracing::error!(reason = %clear_error, "platform clear during fatal exit failed");
                    }
                    event_loop.exit();
                }
            }
        }

        fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
            let _ = self.shutdown_tx.send(true);
            if !self.now_playing_cleared
                && let Err(error) = self.clear_now()
            {
                tracing::error!(reason = %error, "platform clear during event-loop exit failed");
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
    let (inputs, check_only) = configured_runtime()?;
    if check_only {
        return Ok(());
    }
    macos_main::run(inputs)
}

#[cfg(target_os = "linux")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use nowplayd::platform::{PlatformAdapter, SystemPlatform};

    let (inputs, check_only) = configured_runtime()?;
    if check_only {
        return Ok(());
    }
    let (command_tx, command_rx) = unbounded_channel();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let signal_task = tokio::spawn(wait_for_shutdown_signal(shutdown_tx));
    let mut platform = SystemPlatform::new(command_tx)?;
    let result = run_worker(inputs, command_rx, shutdown_rx, move |event| match event {
        WorkerEvent::Publish(publication) => platform
            .publish(&publication)
            .map_err(|error| error.to_string()),
        WorkerEvent::Command(outcome) => {
            log_command_outcome(&outcome);
            Ok(())
        }
        WorkerEvent::Clear { acknowledgement } => {
            let result = platform.clear().map_err(|error| error.to_string());
            let _ = acknowledgement.send(result);
            Ok(())
        }
        WorkerEvent::ShutdownComplete => Ok(()),
        WorkerEvent::ClearForTest => platform.clear().map_err(|error| error.to_string()),
        WorkerEvent::Fatal(error) => Err(error),
    })
    .await;
    signal_task.abort();
    result.map_err(Into::into)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!("nowplayd currently supports macOS and Linux targets");

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        sync::{
            Mutex,
            atomic::{AtomicBool, Ordering},
        },
        time::{Duration, Instant},
    };

    use super::*;
    use nowplayd::{
        artwork::{ArtworkPublication, BinaryChunkSource},
        mpd::{BinaryCommand, BinaryResponse, MpdError},
        platform::RemoteCommandTarget,
        state::MediaKey,
    };
    use tempfile::TempDir;

    #[derive(Clone)]
    struct FixedClock(Instant);

    impl LifecycleClock for FixedClock {
        fn now(&self) -> Instant {
            self.0
        }
    }

    struct LowJitter;

    impl JitterRng for LowJitter {
        fn sample(&mut self) -> u64 {
            0
        }
    }

    #[derive(Clone)]
    struct ShutdownSleeper {
        invoked: Arc<AtomicBool>,
        order: Arc<Mutex<Vec<&'static str>>>,
        shutdown: watch::Sender<bool>,
    }

    impl LifecycleSleeper for ShutdownSleeper {
        async fn sleep(&self, _duration: Duration) {
            self.invoked.store(true, Ordering::SeqCst);
            self.order.lock().unwrap().push("sleep");
            let _ = self.shutdown.send(true);
        }
    }

    #[derive(Clone)]
    struct SecondAttemptShutdownSleeper {
        attempts: Arc<Mutex<usize>>,
        shutdown: watch::Sender<bool>,
    }

    impl LifecycleSleeper for SecondAttemptShutdownSleeper {
        async fn sleep(&self, _duration: Duration) {
            let mut attempts = self.attempts.lock().unwrap();
            *attempts += 1;
            if *attempts == 2 {
                let _ = self.shutdown.send(true);
            }
        }
    }

    #[derive(Default)]
    struct CaptureLog(Mutex<Vec<LifecycleEvent>>);

    impl LifecycleLog for CaptureLog {
        fn record(&self, event: &LifecycleEvent) {
            self.0.lock().unwrap().push(event.clone());
        }
    }

    fn refused_config() -> ConnectionConfig {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        ConnectionConfig {
            address: nowplayd::mpd::MpdAddress::Tcp(address.to_string()),
            password: None,
        }
    }

    #[tokio::test]
    async fn connection_fault_clear_is_acknowledged_before_first_backoff_sleep() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let invoked = Arc::new(AtomicBool::new(false));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let sleeper = ShutdownSleeper {
            invoked: invoked.clone(),
            order: order.clone(),
            shutdown: shutdown_tx,
        };
        let (_command_tx, command_rx) = unbounded_channel();
        let log = Arc::new(CaptureLog::default());
        let event_order = order.clone();

        run_worker_with(
            RuntimeInputs {
                connection: refused_config(),
                cache_root: None,
                log,
            },
            command_rx,
            shutdown_rx,
            move |event| match event {
                WorkerEvent::Clear { acknowledgement } => {
                    event_order.lock().unwrap().push("clear");
                    let _ = acknowledgement.send(Ok(()));
                    Ok(())
                }
                unexpected => Err(format!("unexpected event: {unexpected:?}")),
            },
            FixedClock(Instant::now()),
            LowJitter,
            sleeper,
        )
        .await
        .unwrap();

        assert!(invoked.load(Ordering::SeqCst));
        assert_eq!(*order.lock().unwrap(), ["clear", "sleep"]);
    }

    #[tokio::test]
    async fn clear_failure_is_terminal_and_never_enters_backoff() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let invoked = Arc::new(AtomicBool::new(false));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let sleeper = ShutdownSleeper {
            invoked: invoked.clone(),
            order,
            shutdown: shutdown_tx,
        };
        let (_command_tx, command_rx) = unbounded_channel();

        let error = run_worker_with(
            RuntimeInputs {
                connection: refused_config(),
                cache_root: None,
                log: Arc::new(CaptureLog::default()),
            },
            command_rx,
            shutdown_rx,
            move |event| match event {
                WorkerEvent::Clear { acknowledgement } => {
                    let _ = acknowledgement.send(Err("native clear failed".into()));
                    Ok(())
                }
                unexpected => Err(format!("unexpected event: {unexpected:?}")),
            },
            FixedClock(Instant::now()),
            LowJitter,
            sleeper,
        )
        .await
        .unwrap_err();

        assert_eq!(error, "native clear failed");
        assert!(!invoked.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn repeated_failed_reconnects_do_not_repeat_an_already_acknowledged_clear() {
        let (_command_tx, command_rx) = unbounded_channel();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let attempts = Arc::new(Mutex::new(0_usize));
        let clears = Arc::new(Mutex::new(0_usize));
        let observed = clears.clone();

        run_worker_with(
            RuntimeInputs {
                connection: refused_config(),
                cache_root: None,
                log: Arc::new(CaptureLog::default()),
            },
            command_rx,
            shutdown_rx,
            move |event| match event {
                WorkerEvent::Clear { acknowledgement } => {
                    *observed.lock().unwrap() += 1;
                    let _ = acknowledgement.send(Ok(()));
                    Ok(())
                }
                unexpected => Err(format!("unexpected event: {unexpected:?}")),
            },
            FixedClock(Instant::now()),
            LowJitter,
            SecondAttemptShutdownSleeper {
                attempts: attempts.clone(),
                shutdown: shutdown_tx,
            },
        )
        .await
        .unwrap();

        assert_eq!(*attempts.lock().unwrap(), 2);
        assert_eq!(*clears.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn shutdown_waits_for_clear_ack_and_returns_cleanly() {
        let (_command_tx, command_rx) = unbounded_channel();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        shutdown_tx.send(true).unwrap();
        let clears = Arc::new(Mutex::new(0_usize));
        let observed = clears.clone();

        run_worker_with(
            RuntimeInputs {
                connection: refused_config(),
                cache_root: None,
                log: Arc::new(CaptureLog::default()),
            },
            command_rx,
            shutdown_rx,
            move |event| match event {
                WorkerEvent::Clear { acknowledgement } => {
                    *observed.lock().unwrap() += 1;
                    let _ = acknowledgement.send(Ok(()));
                    Ok(())
                }
                unexpected => Err(format!("unexpected event: {unexpected:?}")),
            },
            FixedClock(Instant::now()),
            LowJitter,
            ShutdownSleeper {
                invoked: Arc::new(AtomicBool::new(false)),
                order: Arc::new(Mutex::new(Vec::new())),
                shutdown: shutdown_tx,
            },
        )
        .await
        .unwrap();

        assert_eq!(*clears.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn no_song_clears_once_and_next_song_republishes() {
        let temp = TempDir::new().unwrap();
        let mut artwork = ArtworkCoordinator::new(ArtworkCache::new(temp.path().into()));
        let mut state = PlayerState {
            media_key: Some(MediaKey("old.flac".into())),
            ..PlayerState::default()
        };
        artwork.observe_state(state.clone());
        let (job_tx, _job_rx) = unbounded_channel();
        let log = CaptureLog::default();
        let mut events = Vec::new();
        let mut platform_may_be_present = true;
        let (_remote_tx, mut remote_rx) = unbounded_channel();
        let (_shutdown_tx, mut shutdown_rx) = watch::channel(false);
        let mut backoff = BackoffPolicy::new(FixedClock(Instant::now()), LowJitter);
        let mut emit = |event| {
            if let WorkerEvent::Clear { acknowledgement } = event {
                let _ = acknowledgement.send(Ok(()));
                events.push("clear");
            } else if matches!(event, WorkerEvent::Publish(_)) {
                events.push("publish");
            }
            Ok(())
        };
        {
            let mut context = ConnectedContext {
                artwork: &mut artwork,
                remote_commands: &mut remote_rx,
                shutdown: &mut shutdown_rx,
                emit: &mut emit,
                backoff: &mut backoff,
                log: &log,
                platform_may_be_present: &mut platform_may_be_present,
            };

            apply_refreshed_state(
                PlayerState::default(),
                &mut state,
                &job_tx,
                false,
                &mut context,
            )
            .await
            .unwrap();
            apply_refreshed_state(
                PlayerState {
                    media_key: Some(MediaKey("new.flac".into())),
                    ..PlayerState::default()
                },
                &mut state,
                &job_tx,
                false,
                &mut context,
            )
            .await
            .unwrap();
        }

        assert_eq!(events, ["clear", "publish"]);
        assert!(artwork.has_pending_work());
    }

    #[tokio::test]
    async fn idle_task_is_aborted_and_awaited_on_teardown() {
        struct Dropped(Arc<AtomicBool>);
        impl Drop for Dropped {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }

        let dropped = Arc::new(AtomicBool::new(false));
        let task_dropped = dropped.clone();
        let task = tokio::spawn(async move {
            let _guard = Dropped(task_dropped);
            std::future::pending::<()>().await;
        });
        tokio::task::yield_now().await;

        abort_idle_task(task).await;
        assert!(dropped.load(Ordering::SeqCst));
    }

    struct FakeTarget {
        calls: Vec<&'static str>,
        results: VecDeque<Result<(), &'static str>>,
        refreshed: PlayerState,
    }

    impl RemoteCommandTarget for FakeTarget {
        async fn execute(&mut self, _command: RemoteCommand) -> Result<(), MpdError> {
            self.calls.push("execute");
            match self.results.pop_front().unwrap_or(Ok(())) {
                Ok(()) => Ok(()),
                Err(message) => Err(MpdError::InvalidField {
                    field: "fake command",
                    value: message.into(),
                }),
            }
        }

        async fn refresh(&mut self) -> Result<PlayerState, MpdError> {
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

        let returned = handle_remote_command(&mut target, RemoteCommand::Play, &mut |event| {
            events.push(event);
            Ok(())
        })
        .await
        .unwrap()
        .unwrap();
        handle_artwork_update(
            artwork.observe_state(returned.clone()),
            &mut |event| {
                events.push(event);
                Ok(())
            },
            &job_tx,
        )
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
        let (_shutdown_tx, mut shutdown_rx) = watch::channel(false);
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
        match next_worker_input(
            &mut shutdown_rx,
            &mut remote_rx,
            &mut idle_rx,
            &mut job_rx,
            true,
        )
        .await
        {
            WorkerInput::Artwork => {
                artwork.step(&mut source).await.unwrap();
                order.push("chunk N");
            }
            _ => panic!("artwork must begin when no command or refresh is queued"),
        }
        assert_eq!(source.requests, [0]);

        remote_tx.send(RemoteCommand::Next).unwrap();
        idle_tx.send(Ok(vec![Subsystem::Player])).unwrap();
        match next_worker_input(
            &mut shutdown_rx,
            &mut remote_rx,
            &mut idle_rx,
            &mut job_rx,
            true,
        )
        .await
        {
            WorkerInput::Remote(Some(RemoteCommand::Next)) => {
                order.extend(["remote command", "coherent refresh", "full publish"]);
            }
            _ => panic!("remote command must have first priority"),
        }
        match next_worker_input(
            &mut shutdown_rx,
            &mut remote_rx,
            &mut idle_rx,
            &mut job_rx,
            true,
        )
        .await
        {
            WorkerInput::Idle(Some(Ok(_))) => order.push("pending state refresh"),
            _ => panic!("pending refresh must precede artwork"),
        }
        match next_worker_input(
            &mut shutdown_rx,
            &mut remote_rx,
            &mut idle_rx,
            &mut job_rx,
            true,
        )
        .await
        {
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
