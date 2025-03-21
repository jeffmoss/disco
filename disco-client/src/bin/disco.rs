use clap::{Parser, Subcommand};

use disco_client::RaftClient;

#[derive(Parser, Clone, Debug)]
#[clap(author, version, about, long_about = None)]
pub struct Opt {
  #[clap(long)]
  // Network address to connect with
  pub addr: String,

  #[clap(subcommand)]
  pub command: Command,
}

#[derive(Subcommand, Clone, Debug)]
pub enum Command {
  /// Get a value by key
  Get {
    /// Key to look up
    key: String,
  },
  /// Set a value for a key
  Set {
    /// Key to set
    key: String,
    /// Value to store
    value: String,
  },
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

  let client = RaftClient::new(options.addr).await?;

  match options.command {
    Command::Get { key } => {
      let result = client.get_value(key).await?;
      println!("Value: {:?}", result);
    }
    Command::Set { key, value } => {
      let result = client.set_value(key, value).await?;
      println!("Set result: {:?}", result);
    }
  }

  Ok(())
}
