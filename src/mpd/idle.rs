use mpd_protocol::{AsyncConnection, Command};
use tokio::io::{AsyncRead, AsyncWrite};

use super::{ConnectionConfig, MpdError, MpdIo, Result, authenticate, exactly_one, open_io};

/// MPD subsystem reported by this ticket's filtered idle role.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Subsystem {
    Player,
    Mixer,
    Options,
}

/// The dedicated MPD role that sends only filtered `idle` commands.
#[derive(Debug)]
pub struct IdleConnection<IO = MpdIo> {
    inner: AsyncConnection<IO>,
}

impl IdleConnection<MpdIo> {
    pub async fn connect(config: &ConnectionConfig) -> Result<Self> {
        let io = open_io(&config.address).await?;
        let mut connection = Self::from_io(io).await?;
        if let Some(password) = &config.password {
            authenticate(&mut connection.inner, password).await?;
        }
        Ok(connection)
    }
}

impl<IO> IdleConnection<IO>
where
    IO: AsyncRead + AsyncWrite + Unpin,
{
    pub async fn from_io(io: IO) -> Result<Self> {
        Ok(Self {
            inner: AsyncConnection::connect(io).await?,
        })
    }

    /// Wait for the next filtered set of subsystem changes.
    pub async fn next_event(&mut self) -> Result<Vec<Subsystem>> {
        let response = self
            .inner
            .command(
                Command::new("idle")
                    .argument("player")
                    .argument("mixer")
                    .argument("options"),
            )
            .await?;
        let frame = exactly_one(response)?;

        frame
            .fields()
            .map(|(key, value)| {
                if key != "changed" {
                    return Err(MpdError::InvalidField {
                        field: "idle response key",
                        value: key.into(),
                    });
                }
                match value {
                    "player" => Ok(Subsystem::Player),
                    "mixer" => Ok(Subsystem::Mixer),
                    "options" => Ok(Subsystem::Options),
                    _ => Err(MpdError::InvalidField {
                        field: "changed",
                        value: value.into(),
                    }),
                }
            })
            .collect()
    }
}
