use anyhow::Result;
use async_trait::async_trait;

mod aws;
pub use aws::AwsProvider;

/// Represents an EC2 instance that may have incomplete information
#[derive(Debug, Clone)]
pub struct InstanceInfo {
  pub id: String,
  pub name: Option<String>,
  pub public_ip: Option<String>,
  pub state: Option<InstanceState>,
}

#[derive(Debug, Clone)]
pub enum InstanceState {
  Pending,
  Running,
  ShuttingDown,
  Terminated,
  Stopping,
  Stopped,
}

/// A trait for providers that can create key pairs and hosts.
#[async_trait]
pub trait Provider: Send + Sync + std::fmt::Debug {
  async fn new(cluster_name: String, region: String) -> Result<Self>
  where
    Self: Sized;

  /// Checks for the existence of a key pair by the cluster name.
  ///
  /// # Arguments
  /// * `name` - The name of the cluster to check for a key pair.
  ///
  /// # Returns
  ///
  /// A future that resolves to the fingerprint, which is `Some` if the key pair exists, or `None` if it does not.
  async fn get_key_pair_by_name(&self, name: &str) -> Result<Option<String>>;

  /// Imports a public key to the provider using one existing on the local filesystem.
  ///
  /// # Arguments
  ///
  /// * `key_path` - Path to an existing public key file on the local filesystem
  ///
  /// # Returns
  ///
  /// A future that resolves to the fingerprint of the imported key pair.
  async fn import_public_key(
    &self,
    name: &str,
    public_key_path: &std::path::Path,
  ) -> Result<String>;

  /// Checks for the existence of an IP address by the cluster name.
  ///
  /// # Arguments
  /// * `name` - The name of the IP address.
  ///
  /// # Returns
  ///
  /// A future that resolves to an `Option<(public_ip, allocation_id)>`, which is `Some` if the IP address exists, or `None` if it does not.
  async fn get_ip_address_by_name(&self, name: &str) -> Result<Option<(String, String)>>;

  /// Creates a new IP address, checking for its existence first.
  ///
  /// # Arguments
  /// * `name` - The name of the IP address.
  ///
  /// # Returns
  ///
  /// A future that resolves to an `Option<(public_ip, allocation_id)>` if the operation was successful.
  async fn primary_ip_address(&self, name: &str) -> Result<(String, String)>;

  /// Attaches an IP address to an instance.
  ///
  /// # Arguments
  /// * `address_id` - The ID of the IP address.
  /// * `host_id` - The ID of the instance to attach the IP address to.
  ///
  /// # Returns
  ///
  /// A future that resolves if the IP the operation was successful.
  async fn attach_ip_address_to_instance(&self, address_id: &str, host_id: &str) -> Result<()>;

  /// Checks for the existence of a host by the a tag name (ie. Name)
  ///
  /// # Arguments
  /// * `name` - The name of the instance (ie. the cluster_name for a primary node).
  ///
  /// # Returns
  ///
  /// A future that resolves to an `Option<String>`, which is `Some` if the host exists, or `None` if it does not.
  async fn get_instance_by_name(&self, name: &str) -> Result<Option<InstanceInfo>>;

  /// Waits for a host to become available with a public IP address.
  ///
  /// # Arguments
  ///
  /// * `instance_id` - The ID of the instance to wait for.
  /// * `timeout_seconds` - Maximum time to wait for the instance to become available.
  /// * `poll_interval_seconds` - Time to wait between status checks.
  ///
  /// # Returns
  ///
  /// A future that resolves to a fully initialized `Host` struct with a valid public IP address.
  /// Returns an error if the timeout is exceeded or if there's another issue retrieving the host information.
  async fn wait_for_instances(
    &self,
    instance_ids: &[String],
    timeout_seconds: u64,
    poll_interval_seconds: u64,
  ) -> Result<Vec<InstanceInfo>>;

  /// Creates a new host, checking for its existence first.
  ///
  /// # Arguments
  ///
  /// * `name` - The name to tag the instance with (ie. the cluster_name for a primary node).
  /// * `image_id` - The ID of the image to use for the host.
  /// * `instance_type` - The type of instance to create.
  ///
  /// # Returns
  ///
  /// A future that resolves to the ID of the created host.
  async fn create_instances(
    &self,
    name: &str,
    image_id: &str,
    instance_type: &str,
    key_pair: &str,
    count: i64,
  ) -> Result<Vec<InstanceInfo>>;

  async fn instance_profile(&self, role_name: &str, profile_name: &str) -> Result<()>;

  /// Creates a new storage with private access and specific role read/write permissions.
  ///
  /// # Arguments
  ///
  /// * `storage_name` - The name of the bucket to create.
  /// * `role` - The role that should have read/write access.
  ///
  /// # Returns
  ///
  /// A future that resolves to the bucket name if successful.
  async fn create_storage(&self, storage_name: &str, role: &str) -> Result<()>;

  /// Uploads a file to a storage with a given key.
  ///
  /// # Arguments
  ///
  /// * `storage_name` - The name of the storage.
  /// * `file_path` - The local file path to upload.
  /// * `key` - The key (path) for the object in the storage.
  ///
  /// # Returns
  ///
  /// A future that resolves if successful.
  async fn upload_file_to_storage(
    &self,
    storage_name: &str,
    file_path: &str,
    key: &str,
  ) -> Result<()>;

  /// Downloads a file from a storage.
  ///
  /// # Arguments
  ///
  /// * `storage_name` - The name of the storage.
  /// * `file_path` - The local file path to save the downloaded file.
  /// * `key` - The key (path) for the object in the storage.
  ///
  /// # Returns
  ///
  /// A future that resolves if successful.
  async fn download_file_from_storage(
    &self,
    storage_name: &str,
    file_path: &str,
    key: &str,
  ) -> Result<()>;
}
