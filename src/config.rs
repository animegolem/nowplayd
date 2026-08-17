//! Strict zero-config defaults with TOML and environment overlays.

use std::{
    collections::HashMap,
    error::Error,
    fmt, fs, io,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use url::Url;

use crate::mpd::{ConnectionConfig, MpdAddress};

pub const ENV_MPD_ADDRESS: &str = "NOWPLAYD_MPD_ADDRESS";
pub const ENV_MPD_PASSWORD: &str = "NOWPLAYD_MPD_PASSWORD";
pub const ENV_CACHE_DIR: &str = "NOWPLAYD_CACHE_DIR";
pub const ENV_LOG_LEVEL: &str = "NOWPLAYD_LOG_LEVEL";

const DEFAULT_MPD_ADDRESS: &str = "tcp://localhost:6600";
const DEFAULT_LOG_LEVEL: &str = "info";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingSource {
    Default,
    ConfigFile,
    Environment,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ConfigSources {
    pub mpd_address: SettingSource,
    pub mpd_password: SettingSource,
    pub cache_dir: SettingSource,
    pub log_level: SettingSource,
}

impl fmt::Debug for ConfigSources {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConfigSources")
            .field("mpd_address", &self.mpd_address)
            .field("mpd_password", &self.mpd_password)
            .field("cache_dir", &self.cache_dir)
            .field("log_level", &self.log_level)
            .field("secret_values", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct AppConfig {
    pub connection: ConnectionConfig,
    pub cache_dir: PathBuf,
    pub log_level: String,
    pub sources: ConfigSources,
}

impl fmt::Debug for AppConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AppConfig")
            .field("connection", &self.connection)
            .field("cache_dir", &self.cache_dir)
            .field("log_level", &self.log_level)
            .field("sources", &self.sources)
            .finish()
    }
}

#[derive(Debug)]
pub enum ConfigError {
    MissingHome,
    Read {
        path: PathBuf,
        source: io::Error,
    },
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    InvalidAddress(String),
    InvalidCacheDir(PathBuf),
    InvalidLogLevel(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingHome => f.write_str("HOME is missing; cannot resolve nowplayd paths"),
            Self::Read { path, source } => {
                write!(f, "read config {}: {source}", path.display())
            }
            Self::Parse { path, source } => {
                write!(f, "parse config {}: {source}", path.display())
            }
            Self::InvalidAddress(value) => write!(
                f,
                "invalid MPD address {value:?}; use tcp://host:port or unix:///absolute/path"
            ),
            Self::InvalidCacheDir(path) => write!(
                f,
                "cache directory must be an absolute non-empty path: {}",
                path.display()
            ),
            Self::InvalidLogLevel(value) => write!(
                f,
                "invalid log level {value:?}; use trace, debug, info, warn, or error"
            ),
        }
    }
}

impl Error for ConfigError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            Self::Parse { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    mpd_address: Option<String>,
    mpd_password: Option<String>,
    cache_dir: Option<PathBuf>,
    log_level: Option<String>,
}

impl AppConfig {
    pub fn load() -> Result<Self, ConfigError> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or(ConfigError::MissingHome)?;
        let environment = std::env::vars().collect::<HashMap<_, _>>();
        Self::load_from(&home, &environment)
    }

    pub fn load_from(
        home: &Path,
        environment: &HashMap<String, String>,
    ) -> Result<Self, ConfigError> {
        if !home.is_absolute() || home.as_os_str().is_empty() || home == Path::new("/") {
            return Err(ConfigError::MissingHome);
        }
        let config_path = home.join(".config/nowplayd/config.toml");
        let file = match fs::read_to_string(&config_path) {
            Ok(contents) => {
                toml::from_str::<FileConfig>(&contents).map_err(|source| ConfigError::Parse {
                    path: config_path.clone(),
                    source,
                })?
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => FileConfig::default(),
            Err(source) => {
                return Err(ConfigError::Read {
                    path: config_path,
                    source,
                });
            }
        };

        let (address, mpd_address) = overlay(
            DEFAULT_MPD_ADDRESS.to_string(),
            file.mpd_address,
            environment.get(ENV_MPD_ADDRESS).cloned(),
        );
        let (password, mpd_password) = overlay_optional(
            file.mpd_password,
            environment.get(ENV_MPD_PASSWORD).cloned(),
        );
        let default_cache = home.join("Library/Caches/nowplayd");
        let (cache_dir, cache_source) = overlay(
            default_cache,
            file.cache_dir,
            environment.get(ENV_CACHE_DIR).map(PathBuf::from),
        );
        let (log_level, log_source) = overlay(
            DEFAULT_LOG_LEVEL.to_string(),
            file.log_level,
            environment.get(ENV_LOG_LEVEL).cloned(),
        );

        if cache_dir.as_os_str().is_empty() || !cache_dir.is_absolute() {
            return Err(ConfigError::InvalidCacheDir(cache_dir));
        }
        validate_log_level(&log_level)?;

        Ok(Self {
            connection: ConnectionConfig {
                address: parse_address(&address)?,
                password,
            },
            cache_dir,
            log_level: log_level.to_ascii_lowercase(),
            sources: ConfigSources {
                mpd_address,
                mpd_password,
                cache_dir: cache_source,
                log_level: log_source,
            },
        })
    }
}

fn overlay<T>(default: T, file: Option<T>, environment: Option<T>) -> (T, SettingSource) {
    if let Some(value) = environment {
        (value, SettingSource::Environment)
    } else if let Some(value) = file {
        (value, SettingSource::ConfigFile)
    } else {
        (default, SettingSource::Default)
    }
}

fn overlay_optional<T>(file: Option<T>, environment: Option<T>) -> (Option<T>, SettingSource) {
    if let Some(value) = environment {
        (Some(value), SettingSource::Environment)
    } else if file.is_some() {
        (file, SettingSource::ConfigFile)
    } else {
        (None, SettingSource::Default)
    }
}

fn parse_address(value: &str) -> Result<MpdAddress, ConfigError> {
    if let Some(path) = value.strip_prefix("unix://") {
        let path = PathBuf::from(path);
        if path.is_absolute() && !path.as_os_str().is_empty() {
            return Ok(MpdAddress::Unix(path));
        }
        return Err(ConfigError::InvalidAddress(value.into()));
    }

    let url = Url::parse(value).map_err(|_| ConfigError::InvalidAddress(value.into()))?;
    if url.scheme() != "tcp"
        || url.username() != ""
        || url.password().is_some()
        || url.path() != "" && url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ConfigError::InvalidAddress(value.into()));
    }
    let host = url
        .host_str()
        .ok_or_else(|| ConfigError::InvalidAddress(value.into()))?;
    let port = url
        .port()
        .ok_or_else(|| ConfigError::InvalidAddress(value.into()))?;
    let address = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    Ok(MpdAddress::Tcp(address))
}

fn validate_log_level(value: &str) -> Result<(), ConfigError> {
    if matches!(
        value.to_ascii_lowercase().as_str(),
        "trace" | "debug" | "info" | "warn" | "error"
    ) {
        Ok(())
    } else {
        Err(ConfigError::InvalidLogLevel(value.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn empty_environment() -> HashMap<String, String> {
        HashMap::new()
    }

    #[test]
    fn missing_file_uses_zero_config_defaults() {
        let temp = TempDir::new().unwrap();
        let config = AppConfig::load_from(temp.path(), &empty_environment()).unwrap();

        assert_eq!(
            config.connection.address,
            MpdAddress::Tcp("localhost:6600".into())
        );
        assert_eq!(config.connection.password, None);
        assert_eq!(
            config.cache_dir,
            temp.path().join("Library/Caches/nowplayd")
        );
        assert_eq!(config.log_level, "info");
        assert_eq!(config.sources.mpd_address, SettingSource::Default);
    }

    #[test]
    fn config_file_overrides_defaults_and_supports_unix_socket() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join(".config/nowplayd");
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(
            config_dir.join("config.toml"),
            format!(
                "mpd_address = \"unix:///tmp/mpd.sock\"\nmpd_password = \"file-secret\"\ncache_dir = {:?}\nlog_level = \"debug\"\n",
                temp.path().join("cache")
            ),
        )
        .unwrap();

        let config = AppConfig::load_from(temp.path(), &empty_environment()).unwrap();
        assert_eq!(
            config.connection.address,
            MpdAddress::Unix(PathBuf::from("/tmp/mpd.sock"))
        );
        assert_eq!(config.connection.password.as_deref(), Some("file-secret"));
        assert_eq!(config.sources.mpd_password, SettingSource::ConfigFile);
    }

    #[test]
    fn environment_wins_over_every_file_value() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join(".config/nowplayd");
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(
            config_dir.join("config.toml"),
            "mpd_address = \"tcp://file.example:6600\"\nmpd_password = \"file-secret\"\nlog_level = \"warn\"\n",
        )
        .unwrap();
        let environment = HashMap::from([
            (ENV_MPD_ADDRESS.into(), "tcp://env.example:7700".into()),
            (ENV_MPD_PASSWORD.into(), "env-secret".into()),
            (
                ENV_CACHE_DIR.into(),
                temp.path().join("env-cache").display().to_string(),
            ),
            (ENV_LOG_LEVEL.into(), "trace".into()),
        ]);

        let config = AppConfig::load_from(temp.path(), &environment).unwrap();
        assert_eq!(
            config.connection.address,
            MpdAddress::Tcp("env.example:7700".into())
        );
        assert_eq!(config.connection.password.as_deref(), Some("env-secret"));
        assert_eq!(config.log_level, "trace");
        assert_eq!(config.sources.log_level, SettingSource::Environment);
    }

    #[test]
    fn malformed_present_file_is_a_loud_error() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join(".config/nowplayd");
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(config_dir.join("config.toml"), "mpd_address = [").unwrap();

        assert!(matches!(
            AppConfig::load_from(temp.path(), &empty_environment()),
            Err(ConfigError::Parse { .. })
        ));
    }

    #[test]
    fn debug_redacts_password_in_config_and_source_report() {
        let temp = TempDir::new().unwrap();
        let environment = HashMap::from([(ENV_MPD_PASSWORD.into(), "sentinel-secret".into())]);
        let config = AppConfig::load_from(temp.path(), &environment).unwrap();

        let config_debug = format!("{config:?}");
        let source_debug = format!("{:?}", config.sources);
        assert!(config_debug.contains("<redacted>"));
        assert!(!config_debug.contains("sentinel-secret"));
        assert!(source_debug.contains("<redacted>"));
        assert!(!source_debug.contains("sentinel-secret"));
    }

    #[test]
    fn ambiguous_or_relative_inputs_are_rejected() {
        let temp = TempDir::new().unwrap();
        for address in ["localhost:6600", "tcp://localhost", "unix://relative.sock"] {
            let environment = HashMap::from([(ENV_MPD_ADDRESS.into(), address.into())]);
            assert!(matches!(
                AppConfig::load_from(temp.path(), &environment),
                Err(ConfigError::InvalidAddress(_))
            ));
        }

        let environment = HashMap::from([(ENV_CACHE_DIR.into(), "relative/cache".into())]);
        assert!(matches!(
            AppConfig::load_from(temp.path(), &environment),
            Err(ConfigError::InvalidCacheDir(_))
        ));
    }
}
