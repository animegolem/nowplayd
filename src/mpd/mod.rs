//! MPD connection roles and response mapping.

mod command;
mod idle;
mod liveness;

use std::{
    error::Error,
    fmt, io,
    path::PathBuf,
    pin::Pin,
    task::{Context, Poll},
};

use mpd_protocol::{AsyncConnection, Command, MpdProtocolError, response};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpStream, UnixStream},
};

pub use command::{BinaryCommand, BinaryResponse, CommandConnection};
pub use idle::{IdleConnection, Subsystem};
pub use liveness::{LiveCommandConnection, LivenessClock, SystemClock};

/// Address of an MPD daemon.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MpdAddress {
    Tcp(String),
    Unix(PathBuf),
}

/// Plain connection inputs. Persistent configuration is owned by AI-IMP-006.
#[derive(Clone, PartialEq, Eq)]
pub struct ConnectionConfig {
    pub address: MpdAddress,
    pub password: Option<String>,
}

impl fmt::Debug for ConnectionConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectionConfig")
            .field("address", &self.address)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            address: MpdAddress::Tcp("localhost:6600".into()),
            password: None,
        }
    }
}

/// A command-level MPD rejection. The connection remains usable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandFailure {
    pub code: u64,
    pub command_index: u64,
    pub command: Option<String>,
    pub message: String,
}

impl From<response::Error> for CommandFailure {
    fn from(error: response::Error) -> Self {
        Self {
            code: error.code,
            command_index: error.command_index,
            command: error.current_command.map(Into::into),
            message: error.message.into(),
        }
    }
}

/// Errors surfaced by the transport and mapping boundary.
#[derive(Debug)]
pub enum MpdError {
    Transport(MpdProtocolError),
    Command(CommandFailure),
    UnexpectedFrameCount {
        expected: usize,
        actual: usize,
    },
    UnexpectedFields {
        actual: usize,
    },
    MissingField(&'static str),
    InvalidField {
        field: &'static str,
        value: String,
    },
    IncoherentSnapshot {
        status_song_id: Option<u64>,
        current_song_id: Option<u64>,
    },
}

impl fmt::Display for MpdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(error) => write!(f, "MPD transport error: {error}"),
            Self::Command(error) => write!(
                f,
                "MPD command rejected (code {}): {}",
                error.code, error.message
            ),
            Self::UnexpectedFrameCount { expected, actual } => {
                write!(f, "expected {expected} MPD response frames, got {actual}")
            }
            Self::UnexpectedFields { actual } => {
                write!(f, "expected an empty MPD response, got {actual} fields")
            }
            Self::MissingField(field) => write!(f, "MPD response omitted required field {field}"),
            Self::InvalidField { field, value } => {
                write!(f, "MPD response has invalid {field} value {value:?}")
            }
            Self::IncoherentSnapshot {
                status_song_id,
                current_song_id,
            } => write!(
                f,
                "MPD snapshot changed during refresh: status songid {status_song_id:?}, currentsong Id {current_song_id:?}"
            ),
        }
    }
}

impl Error for MpdError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Transport(error) => Some(error),
            _ => None,
        }
    }
}

impl MpdError {
    /// Whether the operation lost or could not establish its transport.
    pub fn is_transport(&self) -> bool {
        matches!(self, Self::Transport(_))
    }
}

impl From<MpdProtocolError> for MpdError {
    fn from(error: MpdProtocolError) -> Self {
        Self::Transport(error)
    }
}

impl From<io::Error> for MpdError {
    fn from(error: io::Error) -> Self {
        Self::Transport(MpdProtocolError::Io(error))
    }
}

pub type Result<T> = std::result::Result<T, MpdError>;

/// Concrete stream used by configured TCP and Unix-socket connections.
#[derive(Debug)]
pub enum MpdIo {
    Tcp(TcpStream),
    Unix(UnixStream),
}

impl AsyncRead for MpdIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match &mut *self {
            Self::Tcp(stream) => Pin::new(stream).poll_read(cx, buf),
            Self::Unix(stream) => Pin::new(stream).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for MpdIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match &mut *self {
            Self::Tcp(stream) => Pin::new(stream).poll_write(cx, buf),
            Self::Unix(stream) => Pin::new(stream).poll_write(cx, buf),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut *self {
            Self::Tcp(stream) => Pin::new(stream).poll_flush(cx),
            Self::Unix(stream) => Pin::new(stream).poll_flush(cx),
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut *self {
            Self::Tcp(stream) => Pin::new(stream).poll_shutdown(cx),
            Self::Unix(stream) => Pin::new(stream).poll_shutdown(cx),
        }
    }
}

async fn open_io(address: &MpdAddress) -> Result<MpdIo> {
    match address {
        MpdAddress::Tcp(address) => Ok(MpdIo::Tcp(TcpStream::connect(address).await?)),
        MpdAddress::Unix(path) => Ok(MpdIo::Unix(UnixStream::connect(path).await?)),
    }
}

fn collect_frames(response: mpd_protocol::response::Response) -> Result<Vec<response::Frame>> {
    response
        .into_iter()
        .map(|frame| frame.map_err(|error| MpdError::Command(error.into())))
        .collect()
}

fn exactly_one(response: mpd_protocol::response::Response) -> Result<response::Frame> {
    let mut frames = collect_frames(response)?;
    if frames.len() != 1 {
        return Err(MpdError::UnexpectedFrameCount {
            expected: 1,
            actual: frames.len(),
        });
    }
    Ok(frames.remove(0))
}

async fn authenticate<IO>(connection: &mut AsyncConnection<IO>, password: &str) -> Result<()>
where
    IO: AsyncRead + AsyncWrite + Unpin,
{
    // Do not add application logging here. SPEC §11 FR-7 permanently
    // disables the dependency's command-bearing tracing target in IMP-006.
    let response = connection
        .command(Command::new("password").argument(password))
        .await?;
    let frame = exactly_one(response)?;
    if !frame.is_empty() {
        return Err(MpdError::UnexpectedFields {
            actual: frame.fields_len(),
        });
    }
    Ok(())
}

fn parse_optional_u64(frame: &response::Frame, field: &'static str) -> Result<Option<u64>> {
    frame
        .find(field)
        .map(|value| {
            value.parse().map_err(|_| MpdError::InvalidField {
                field,
                value: value.into(),
            })
        })
        .transpose()
}

fn parse_optional_duration(
    frame: &response::Frame,
    field: &'static str,
) -> Result<Option<std::time::Duration>> {
    frame
        .find(field)
        .map(|value| {
            let seconds = value.parse::<f64>().map_err(|_| MpdError::InvalidField {
                field,
                value: value.into(),
            })?;
            std::time::Duration::try_from_secs_f64(seconds).map_err(|_| MpdError::InvalidField {
                field,
                value: value.into(),
            })
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_debug_redacts_password() {
        let config = ConnectionConfig {
            address: MpdAddress::Tcp("localhost:6600".into()),
            password: Some("sentinel-secret".into()),
        };

        let rendered = format!("{config:?}");
        assert!(rendered.contains("<redacted>"));
        assert!(!rendered.contains("sentinel-secret"));
    }
}
