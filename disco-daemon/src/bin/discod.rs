#![allow(clippy::uninlined_format_args)]

use clap::Parser;
use disco_daemon::config::Opt;
use disco_daemon::node::Node;
use disco_daemon::settings::Settings;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
  // Initialize tracing first, before any logging happens
  tracing_subscriber::fmt()
    .with_max_level(tracing::Level::INFO)
    .with_env_filter(tracing_subscriber::EnvFilter::from_env("DISCO_LOG"))
    .with_file(true)
    .with_line_number(true)
    .init();

  // Parse the parameters passed by arguments.
  let options = Opt::parse();

  let settings = Settings::new()?;

  let node = Node::new(options, settings).await?;

  node.run().await?;

  Ok(())
}
