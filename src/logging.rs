//! Tracing setup with an immutable credential-bearing dependency fence.

use std::{error::Error, fmt};

use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use crate::{
    config::{AppConfig, SettingSource},
    lifecycle::{LifecycleEvent, LifecycleLog},
};

#[derive(Debug)]
pub struct LoggingError(String);

impl fmt::Display for LoggingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "initialize tracing: {}", self.0)
    }
}

impl Error for LoggingError {}

pub fn filter(log_level: &str) -> Result<EnvFilter, LoggingError> {
    let base = log_level
        .parse()
        .map_err(|error| LoggingError(format!("invalid log directive: {error}")))?;
    let fence = "mpd_protocol=off"
        .parse()
        .map_err(|error| LoggingError(format!("invalid credential fence: {error}")))?;
    Ok(EnvFilter::default()
        .add_directive(base)
        .add_directive(fence))
}

pub fn init(log_level: &str) -> Result<(), LoggingError> {
    tracing_subscriber::registry()
        .with(filter(log_level)?)
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_target(true)
                .with_writer(std::io::stderr),
        )
        .try_init()
        .map_err(|error| LoggingError(error.to_string()))
}

pub fn log_startup_config(config: &AppConfig) {
    log_setting(
        "mpd_address",
        &format!("{:?}", config.connection.address),
        config.sources.mpd_address,
    );
    log_setting("mpd_password", "<redacted>", config.sources.mpd_password);
    log_setting(
        "cache_dir",
        &config.cache_dir.display().to_string(),
        config.sources.cache_dir,
    );
    log_setting("log_level", &config.log_level, config.sources.log_level);
}

fn log_setting(name: &str, value: &str, source: SettingSource) {
    tracing::info!(setting = name, value, source = ?source, "configuration resolved");
}

#[derive(Debug, Default)]
pub struct TracingLifecycleLog;

impl LifecycleLog for TracingLifecycleLog {
    fn record(&self, event: &LifecycleEvent) {
        tracing::info!(event = ?event, "lifecycle transition");
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        io::{self, Write},
        sync::{Arc, Mutex},
    };

    use tracing_subscriber::prelude::*;

    use super::*;
    use crate::config::{ENV_MPD_ADDRESS, ENV_MPD_PASSWORD};
    use tempfile::TempDir;

    #[derive(Clone, Default)]
    struct Buffer(Arc<Mutex<Vec<u8>>>);

    struct BufferWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for BufferWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Buffer {
        type Writer = BufferWriter;

        fn make_writer(&'a self) -> Self::Writer {
            BufferWriter(self.0.clone())
        }
    }

    #[test]
    fn most_verbose_filter_cannot_expose_mpd_protocol_fields() {
        let output = Buffer::default();
        let subscriber = tracing_subscriber::registry()
            .with(filter("trace").unwrap())
            .with(
                tracing_subscriber::fmt::layer()
                    .without_time()
                    .with_writer(output.clone()),
            );

        tracing::subscriber::with_default(subscriber, || {
            tracing::trace!(
                target: "mpd_protocol",
                password = "sentinel-password",
                "credential-bearing protocol event"
            );
            tracing::info!(target: "nowplayd", "visible application event");
        });

        let rendered = String::from_utf8(output.0.lock().unwrap().clone()).unwrap();
        assert!(rendered.contains("visible application event"));
        assert!(!rendered.contains("sentinel-password"));
        assert!(!rendered.contains("credential-bearing protocol event"));
    }

    #[test]
    fn startup_capture_reports_every_source_and_redacts_password() {
        let temp = TempDir::new().unwrap();
        let environment = HashMap::from([
            (ENV_MPD_ADDRESS.into(), "tcp://example.test:6601".into()),
            (ENV_MPD_PASSWORD.into(), "sentinel-password".into()),
        ]);
        let config = AppConfig::load_from(temp.path(), &environment).unwrap();
        let output = Buffer::default();
        let subscriber = tracing_subscriber::registry()
            .with(filter("info").unwrap())
            .with(
                tracing_subscriber::fmt::layer()
                    .without_time()
                    .with_writer(output.clone()),
            );

        tracing::subscriber::with_default(subscriber, || log_startup_config(&config));

        let rendered = String::from_utf8(output.0.lock().unwrap().clone()).unwrap();
        for setting in ["mpd_address", "mpd_password", "cache_dir", "log_level"] {
            assert!(rendered.contains(setting));
        }
        assert!(rendered.contains("Environment"));
        assert!(rendered.contains("Default"));
        assert!(rendered.contains("example.test:6601"));
        assert!(rendered.contains("<redacted>"));
        assert!(!rendered.contains("sentinel-password"));
    }
}
