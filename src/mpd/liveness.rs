use std::time::{Duration, Instant};

use crate::state::PlayerState;

use super::{BinaryCommand, BinaryResponse, CommandConnection, ConnectionConfig, MpdIo, Result};

const COMMAND_STALE_AFTER: Duration = Duration::from_secs(50);

/// Monotonic time source for the command-role at-use liveness check.
pub trait LivenessClock: Clone {
    fn now(&self) -> Instant;
}

/// Production monotonic clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl LivenessClock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// The production command role, with rev-0.10 per-use socket liveness.
///
/// It never wakes on a timer. Before each use it validates a socket that has
/// been quiet for 50 seconds, reconnecting before the requested operation if
/// validation fails. Non-mutating operations retry once after reconnect;
/// mutating commands are issued at most once.
#[derive(Debug)]
pub struct LiveCommandConnection<C = SystemClock> {
    config: ConnectionConfig,
    connection: Option<CommandConnection<MpdIo>>,
    clock: C,
    last_use: Instant,
}

impl LiveCommandConnection<SystemClock> {
    pub async fn connect(config: ConnectionConfig) -> Result<Self> {
        Self::connect_with_clock(config, SystemClock).await
    }
}

impl<C> LiveCommandConnection<C>
where
    C: LivenessClock,
{
    pub async fn connect_with_clock(config: ConnectionConfig, clock: C) -> Result<Self> {
        let connection = CommandConnection::connect(&config).await?;
        let last_use = clock.now();
        Ok(Self {
            config,
            connection: Some(connection),
            clock,
            last_use,
        })
    }

    pub async fn refresh(&mut self) -> Result<PlayerState> {
        self.ensure_live().await?;
        let result = self.connection_mut()?.refresh().await;
        match result {
            Ok(state) => {
                self.mark_used();
                Ok(state)
            }
            Err(error) if error.is_transport() => {
                self.connection = None;
                self.reconnect().await?;
                let retry = self.connection_mut()?.refresh().await;
                self.finish_non_mutating(retry)
            }
            Err(error) => {
                self.mark_used();
                Err(error)
            }
        }
    }

    pub async fn read_binary(
        &mut self,
        kind: BinaryCommand,
        uri: &str,
        offset: usize,
    ) -> Result<BinaryResponse> {
        self.ensure_live().await?;
        let result = self.connection_mut()?.read_binary(kind, uri, offset).await;
        match result {
            Ok(response) => {
                self.mark_used();
                Ok(response)
            }
            Err(error) if error.is_transport() => {
                self.connection = None;
                self.reconnect().await?;
                let retry = self.connection_mut()?.read_binary(kind, uri, offset).await;
                self.finish_non_mutating(retry)
            }
            Err(error) => {
                self.mark_used();
                Err(error)
            }
        }
    }

    pub async fn toggle(&mut self) -> Result<()> {
        self.run_mutating(RemoteMutation::Toggle).await
    }

    pub async fn play(&mut self) -> Result<()> {
        self.run_mutating(RemoteMutation::Play).await
    }

    pub async fn pause(&mut self) -> Result<()> {
        self.run_mutating(RemoteMutation::Pause).await
    }

    pub async fn next(&mut self) -> Result<()> {
        self.run_mutating(RemoteMutation::Next).await
    }

    pub async fn previous(&mut self) -> Result<()> {
        self.run_mutating(RemoteMutation::Previous).await
    }

    async fn run_mutating(&mut self, mutation: RemoteMutation) -> Result<()> {
        self.ensure_live().await?;
        let result = match mutation {
            RemoteMutation::Toggle => self.connection_mut()?.toggle().await,
            RemoteMutation::Play => self.connection_mut()?.play().await,
            RemoteMutation::Pause => self.connection_mut()?.pause().await,
            RemoteMutation::Next => self.connection_mut()?.next().await,
            RemoteMutation::Previous => self.connection_mut()?.previous().await,
        };
        if result.as_ref().is_err_and(|error| error.is_transport()) {
            // The write may have landed. Invalidate the role for the next use,
            // but never reconnect and reissue this mutation.
            self.connection = None;
        } else {
            self.mark_used();
        }
        result
    }

    async fn ensure_live(&mut self) -> Result<()> {
        if self.connection.is_none() {
            return self.reconnect().await;
        }
        if self
            .clock
            .now()
            .checked_duration_since(self.last_use)
            .unwrap_or_default()
            <= COMMAND_STALE_AFTER
        {
            return Ok(());
        }

        match self.connection_mut()?.ping().await {
            Ok(()) => {
                self.mark_used();
                return Ok(());
            }
            Err(error) if !error.is_transport() => {
                self.mark_used();
                return Err(error);
            }
            Err(_) => {}
        }

        self.connection = None;
        self.reconnect().await
    }

    async fn reconnect(&mut self) -> Result<()> {
        let connection = CommandConnection::connect(&self.config).await?;
        self.connection = Some(connection);
        self.mark_used();
        Ok(())
    }

    fn connection_mut(&mut self) -> Result<&mut CommandConnection<MpdIo>> {
        self.connection.as_mut().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "MPD command role is not connected",
            )
            .into()
        })
    }

    fn finish_non_mutating<T>(&mut self, result: Result<T>) -> Result<T> {
        if result.as_ref().is_err_and(|error| error.is_transport()) {
            self.connection = None;
        } else {
            self.mark_used();
        }
        result
    }

    fn mark_used(&mut self) {
        self.last_use = self.clock.now();
    }
}

#[derive(Clone, Copy)]
enum RemoteMutation {
    Toggle,
    Play,
    Pause,
    Next,
    Previous,
}
