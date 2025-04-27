use async_trait::async_trait;
use rhai::{CustomType, TypeBuilder};

#[derive(Debug, Clone, CustomType)]
pub struct KeyPair {
  pub name: String,
  pub fingerprint: String,
}

#[derive(Debug, Clone, CustomType)]
pub struct Address {
  #[rhai_type(readonly)]
  pub name: String,

  #[rhai_type(readonly)]
  pub public_ip: String,

  #[rhai_type(readonly)]
  pub fingerprint: String,
}

/// A trait for providers that can create key pairs and hosts.
#[async_trait]
pub trait Provider: Send + Sync {
  /// Imports a public key to the provider using one existing on the local filesystem.
  ///
  /// # Arguments
  ///
  /// * `key_path` - Path to an existing public key file on the local filesystem
  /// * `key_name` - Name to assign to the imported key pair
  ///
  /// # Returns
  ///
  /// A future that resolves to the fingerprint of the imported key pair.
  async fn import_public_key(
    &self,
    key_path: std::path::PathBuf,
    key_name: &String,
  ) -> Result<KeyPair, Box<dyn std::error::Error + Send + Sync>>;

  /// Creates a new IP address.
  async fn create_ip_address(
    &self,
    name: &str,
  ) -> Result<Address, Box<dyn std::error::Error + Send + Sync>>;

  /// Creates a new host.
  ///
  /// # Returns
  ///
  /// A future that resolves to the ID of the created host.
  async fn create_host(
    &self,
    image_id: String,
  ) -> Result<String, Box<dyn std::error::Error + Send + Sync>>;
}
