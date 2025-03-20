use std::future::Future;

/// A trait for providers that can create key pairs and hosts.
pub trait Provider: Send {
  /// Imports a public key to the provider using one existing on the local filesystem.
  ///
  /// # Arguments
  ///
  /// * `key_path` - Path to an existing public key file on the local filesystem
  /// * `key_name` - Name to assign to the imported key pair
  ///
  /// # Returns
  ///
  /// A new key pair of type `Self::KeyPair`.
  fn import_public_key(
    &self,
    key_path: std::path::PathBuf,
    key_name: String,
  ) -> impl Future<Output = Result<String, Box<dyn std::error::Error + Send + Sync>>>;

  /// Creates a new host.
  ///
  /// # Returns
  ///
  /// A new host of type `Self::Host`.
  fn create_host(
    &self,
    image_id: String,
  ) -> impl Future<Output = Result<String, Box<dyn std::error::Error + Send + Sync>>>;
}
