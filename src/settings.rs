use config::{Config, ConfigError, Environment, File};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Settings {
  pub cluster_name: String,
  pub election_timeout_min: u64,
  pub election_timeout_max: u64,
  pub heartbeat_interval: u64,
  pub install_snapshot_timeout: u64,
}

impl Settings {
  pub fn new() -> Result<Self, ConfigError> {
    let config = Config::builder()
      // Start with default values
      .set_default("cluster_name", "cluster")?
      .set_default("election_timeout_min", 150)?
      .set_default("election_timeout_max", 300)?
      .set_default("heartbeat_interval", 50)?
      .set_default("install_snapshot_timeout", 120)?
      // Load from a config file
      // Will look for config.yaml, config.json, config.toml, etc.
      .add_source(File::with_name("config").required(false))
      
      // Override with environment variables prefixed with 'APP_'
      .add_source(Environment::with_prefix("CLUSTER"))

      .build()?;

    // Deserialize the configuration into our Settings struct
    config.try_deserialize()
  }
}
