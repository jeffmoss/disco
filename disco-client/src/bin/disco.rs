use clap::{Parser, Subcommand};

use disco_client::client::RaftClient;
use disco_client::command::{Bootstrap, Command};
use disco_common::engine::*;

#[derive(Parser, Clone, Debug)]
#[clap(author, version, about, long_about = None)]
pub struct Opt {
  #[clap(subcommand)]
  pub command: SubCommand,
}

#[derive(Subcommand, Clone, Debug)]
pub enum SubCommand {
  /// Get a value by key
  Get {
    /// Network address to connect with
    #[clap(long)]
    addr: String,

    /// Key to look up
    key: String,
  },
  /// Set a value for a key
  Set {
    /// Network address to connect with
    #[clap(long)]
    addr: String,

    /// Key to set
    key: String,
    /// Value to store
    value: String,
  },
  /// Start the server
  Start {},
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
  // Initialize tracing first, before any logging happens
  tracing_subscriber::fmt()
    .with_max_level(tracing::Level::INFO)
    .with_file(true)
    .with_line_number(true)
    .init();

  let options = Opt::parse();

  let engine = Engine::new("client.rhai")?;

  match options.command {
    SubCommand::Get { addr, key } => {
      let client = RaftClient::new(addr).await?;
      let result = client.get_value(key).await?;
      println!("Value: {:?}", result);
    }
    SubCommand::Set { addr, key, value } => {
      let client = RaftClient::new(addr).await?;
      let result = client.set_value(key, value).await?;
      println!("Set result: {:?}", result);
    }
    SubCommand::Start {} => {
      let _ = Bootstrap::new(4, engine).run()?;
    }
  }

  Ok(())
}
