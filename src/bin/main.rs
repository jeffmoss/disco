use clap::Parser;
use openraft::Config;

use raftd::node::NodeService;
use raftd::settings::Settings;

#[derive(Parser, Clone, Debug)]
#[clap(author, version, about, long_about = None)]
pub struct Opt {
  #[clap(long)]
  pub id: u64,

  #[clap(long)]
  /// Network address to bind the server to (e.g., "127.0.0.1:50051")
  pub addr: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
  // Initialize tracing first, before any logging happens
  tracing_subscriber::fmt()
    .with_max_level(tracing::Level::INFO)
    .with_file(true)
    .with_line_number(true)
    .init();

  // Parse the parameters passed by arguments.
  let options = Opt::parse();

  let settings = Settings::new()?;

  let config: Config = Config {
    cluster_name: settings.cluster_name,
    election_timeout_min: settings.election_timeout_min,
    election_timeout_max: settings.election_timeout_max,
    heartbeat_interval: settings.heartbeat_interval,
    install_snapshot_timeout: settings.install_snapshot_timeout,
    ..Default::default()
  }
  .validate()?;

  let service = NodeService::new(options.id, options.addr, config);
  service.start().await?;

  Ok(())
}
