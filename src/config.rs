use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    #[serde(default = "default_listen_addr")]
    pub listen_addr: SocketAddr,
    #[serde(default)]
    pub seed_peers: Vec<SocketAddr>,
    #[serde(default)]
    pub keypair_path: Option<PathBuf>,
    #[serde(default = "default_data_dir")]
    pub data_dir: PathBuf,
    #[serde(default = "default_rpc_port")]
    pub rpc_port: u16,
    #[serde(default = "default_block_time_ms")]
    pub block_time_ms: u64,
    #[serde(default = "default_coupling_k")]
    pub coupling_k: f64,
    #[serde(default = "default_chiral_eta")]
    pub chiral_eta: f64,
}

fn default_listen_addr() -> SocketAddr {
    "0.0.0.0:9000".parse().unwrap()
}

fn default_data_dir() -> PathBuf {
    PathBuf::from("data")
}

fn default_rpc_port() -> u16 {
    9090
}

fn default_block_time_ms() -> u64 {
    5000
}

fn default_coupling_k() -> f64 {
    0.5
}

fn default_chiral_eta() -> f64 {
    0.01
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen_addr: default_listen_addr(),
            seed_peers: Vec::new(),
            keypair_path: None,
            data_dir: default_data_dir(),
            rpc_port: default_rpc_port(),
            block_time_ms: default_block_time_ms(),
            coupling_k: default_coupling_k(),
            chiral_eta: default_chiral_eta(),
        }
    }
}

impl Config {
    /// Validate configuration values.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.block_time_ms == 0 {
            return Err(ConfigError::Validation("block_time_ms must be > 0".into()));
        }
        if self.coupling_k < 0.0 || !self.coupling_k.is_finite() {
            return Err(ConfigError::Validation("coupling_k must be non-negative and finite".into()));
        }
        if self.chiral_eta < 0.0 || !self.chiral_eta.is_finite() {
            return Err(ConfigError::Validation("chiral_eta must be non-negative and finite".into()));
        }
        Ok(())
    }

    /// Load config from a TOML file, falling back to defaults for missing fields.
    pub fn load(path: &std::path::Path) -> Result<Self, ConfigError> {
        let contents = std::fs::read_to_string(path)
            .map_err(|e| ConfigError::Io(e))?;
        let config: Config = toml::from_str(&contents)
            .map_err(|e| ConfigError::Parse(e))?;
        config.validate()?;
        Ok(config)
    }

    /// Load from file if it exists, otherwise return defaults.
    pub fn load_or_default(path: Option<&std::path::Path>) -> Result<Self, ConfigError> {
        match path {
            Some(p) if p.exists() => Self::load(p),
            _ => Ok(Self::default()),
        }
    }

    /// Serialize to TOML string.
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(self)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("TOML parse error: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("Validation error: {0}")]
    Validation(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_defaults() {
        let cfg = Config::default();
        assert_eq!(cfg.listen_addr, "0.0.0.0:9000".parse().unwrap());
        assert!(cfg.seed_peers.is_empty());
        assert_eq!(cfg.keypair_path, None);
        assert_eq!(cfg.data_dir, PathBuf::from("data"));
        assert_eq!(cfg.rpc_port, 9090);
        assert_eq!(cfg.block_time_ms, 5000);
        assert_eq!(cfg.coupling_k, 0.5);
        assert_eq!(cfg.chiral_eta, 0.01);
    }

    #[test]
    fn test_toml_roundtrip() {
        let cfg = Config {
            listen_addr: "127.0.0.1:8000".parse().unwrap(),
            seed_peers: vec!["10.0.0.1:9000".parse().unwrap(), "10.0.0.2:9000".parse().unwrap()],
            keypair_path: Some(PathBuf::from("/keys/node.key")),
            data_dir: PathBuf::from("/var/oxscada"),
            rpc_port: 7070,
            block_time_ms: 3000,
            coupling_k: 1.5,
            chiral_eta: 0.05,
        };
        let toml_str = cfg.to_toml().unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();
        assert_eq!(cfg, parsed);
    }

    #[test]
    fn test_partial_toml_uses_defaults() {
        let toml_str = r#"
            rpc_port = 1234
            coupling_k = 2.0
        "#;
        let cfg: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.rpc_port, 1234);
        assert_eq!(cfg.coupling_k, 2.0);
        // rest should be defaults
        assert_eq!(cfg.listen_addr, "0.0.0.0:9000".parse().unwrap());
        assert_eq!(cfg.block_time_ms, 5000);
        assert_eq!(cfg.chiral_eta, 0.01);
    }

    #[test]
    fn test_empty_toml_gives_defaults() {
        let cfg: Config = toml::from_str("").unwrap();
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn test_load_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let cfg = Config {
            block_time_ms: 1000,
            ..Default::default()
        };
        std::fs::write(&path, cfg.to_toml().unwrap()).unwrap();
        let loaded = Config::load(&path).unwrap();
        assert_eq!(loaded.block_time_ms, 1000);
    }

    #[test]
    fn test_load_or_default_missing() {
        let cfg = Config::load_or_default(Some(std::path::Path::new("/nonexistent/config.toml"))).unwrap();
        assert_eq!(cfg, Config::default());
    }
}
