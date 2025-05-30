use clap::Parser;

#[derive(Parser, Clone, Debug)]
#[clap(author, version, about, long_about = None)]
pub struct Opt {
  #[clap(long, env = "DISCO_ID")]
  pub id: u64,

  #[clap(long, env = "DISCO_ADDR")]
  /// Network address to bind the server to (e.g., "127.0.0.1:50051")
  pub addr: String,

  #[clap(long, env = "DISCO_CA_CERT")]
  /// Path to the Certificate Authority certificate file
  pub ca_cert: String,

  #[clap(long, env = "DISCO_SERVER_CERT")]
  /// Path to the server certificate file
  pub server_cert: String,

  #[clap(long, env = "DISCO_SERVER_KEY")]
  /// Path to the server private key file
  pub server_key: String,

  #[clap(long, env = "DISCO_CLIENT_CERT")]
  /// Path to the client certificate file
  pub client_cert: String,

  #[clap(long, env = "DISCO_CLIENT_KEY")]
  /// Path to the client private key file
  pub client_key: String,

  #[clap(long, env = "DISCO_DATA_DIR")]
  /// Directory for storing application data
  pub data_dir: String,
}
