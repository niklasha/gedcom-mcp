use serde::Deserialize;
use std::{
    fs,
    net::SocketAddr,
    path::{Path, PathBuf},
};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub bind_addr: SocketAddr,
    pub gedcom_path: PathBuf,
    pub persistence_path: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct RawConfig {
    bind_address: String,
    gedcom_path: PathBuf,
    #[serde(default)]
    persistence_path: Option<PathBuf>,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to parse config: {0}")]
    ParseToml(#[from] toml::de::Error),
    #[error("invalid bind address: {0}")]
    InvalidBindAddress(#[from] std::net::AddrParseError),
    #[error("failed to read config file: {0}")]
    Io(#[from] std::io::Error),
}

impl Config {
    pub fn from_str(contents: &str) -> Result<Self, ConfigError> {
        let raw: RawConfig = toml::from_str(contents)?;
        let bind_addr = raw.bind_address.parse()?;

        Ok(Self {
            bind_addr,
            gedcom_path: raw.gedcom_path,
            persistence_path: raw.persistence_path,
        })
    }

    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let contents = fs::read_to_string(path)?;
        Self::from_str(&contents)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_config() {
        let config = Config::from_str(
            r#"
            bind_address = "127.0.0.1:8080"
            gedcom_path = "/data/example.ged"
            persistence_path = "/data/state.json"
            "#,
        )
        .expect("config should parse");

        assert_eq!(
            config,
            Config {
                bind_addr: "127.0.0.1:8080".parse().unwrap(),
                gedcom_path: PathBuf::from("/data/example.ged"),
                persistence_path: Some(PathBuf::from("/data/state.json")),
            }
        );
    }

    #[test]
    fn rejects_invalid_bind_address() {
        let err = Config::from_str(
            r#"
            bind_address = "not an address"
            gedcom_path = "/data/example.ged"
            persistence_path = "/tmp/out.json"
            "#,
        )
        .expect_err("config should fail");

        assert!(matches!(err, ConfigError::InvalidBindAddress(_)));
    }

    #[test]
    fn rejects_missing_required_fields() {
        let err = Config::from_str(
            r#"
            bind_address = "127.0.0.1:8080"
            "#,
        )
        .expect_err("config should fail");

        assert!(matches!(err, ConfigError::ParseToml(_)));
    }
}
