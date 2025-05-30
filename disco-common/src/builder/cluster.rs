use super::{Host, KeyPair};
use crate::builder::IPAddress;
use crate::provider::*;
use crate::ssh::Installer;

use boa_engine::JsData;
use boa_gc::{Finalize, Trace};
use std::path::Path;
use std::sync::{Arc, RwLock};
use tokio::task;
use tracing::{info, warn};

#[derive(Debug)]
struct ClusterInner {
  name: String,
  key_pair: RwLock<Option<KeyPair>>,
  provider: Arc<dyn Provider>,
  hosts: RwLock<Vec<Arc<Host>>>,
}

#[derive(Clone, Debug)]
#[cfg_attr(feature = "js", derive(Trace, Finalize, JsData))]
pub struct Cluster {
  #[unsafe_ignore_trace]
  inner: Arc<ClusterInner>,
}

impl Cluster {
  pub fn new(name: String, provider: impl Provider + 'static) -> Self {
    Self {
      inner: Arc::new(ClusterInner {
        name,
        key_pair: RwLock::new(None),
        provider: Arc::new(provider),
        hosts: RwLock::new(Vec::new()),
      }),
    }
  }

  pub fn name(&self) -> &str {
    &self.inner.name
  }

  pub fn provider(&self) -> Arc<dyn Provider> {
    self.inner.provider.clone()
  }

  pub fn key_pair(&self) -> std::sync::RwLockReadGuard<Option<KeyPair>> {
    self.inner.key_pair.read().unwrap()
  }

  pub fn key_pair_mut(&self) -> std::sync::RwLockWriteGuard<Option<KeyPair>> {
    self.inner.key_pair.write().unwrap()
  }

  pub fn hosts(&self) -> std::sync::RwLockReadGuard<Vec<Arc<Host>>> {
    self.inner.hosts.read().unwrap()
  }

  pub fn hosts_mut(&self) -> std::sync::RwLockWriteGuard<Vec<Arc<Host>>> {
    self.inner.hosts.write().unwrap()
  }

  pub async fn set_key_pair(
    &self,
    private_key: &str,
    public_key: &str,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let private_key_path = Path::new(private_key);
    let public_key_path = Path::new(public_key);

    // Lock the key pair
    let mut key_pair = self.key_pair_mut();

    let provider = self.provider();

    // Check if key pair exists
    let existing_fingerprint = provider.get_key_pair_by_name(self.name()).await?;

    // If we have an existing fingerprint, check if it matches
    if let Some(fingerprint) = existing_fingerprint {
      let fingerprints_match =
        KeyPair::fingerprint_matches_local_public_key(&fingerprint, public_key_path).await?;

      if !fingerprints_match {
        return Err("Key pair exists in AWS but has a different fingerprint".into());
      }

      // Fingerprints match, set the existing key pair
      *key_pair = Some(KeyPair {
        name: self.name().to_string(),
        private_key: private_key_path.to_path_buf(),
        fingerprint,
      });
    } else {
      // No existing key pair, import a new one
      let fingerprint = provider
        .import_public_key(self.name(), public_key_path)
        .await?;

      *key_pair = Some(KeyPair {
        name: self.name().to_string(),
        private_key: private_key_path.to_path_buf(),
        fingerprint,
      });
    }

    Ok(())
  }

  pub async fn start_instance(
    &self,
    image: &str,
    instance_type: &str,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let provider = self.provider();
    let cluster_name = self.name();

    let mut hosts = self.hosts_mut();

    // First check if we already have a host with matching name in our collection
    for host in hosts.iter() {
      if host.name == cluster_name {
        info!("Found existing host in memory: {:?}", host);
        // We need to clone the host to return it outside the Arc
        return Ok(());
      }
    }

    // If not found locally, check if a host with this cluster name exists remotely
    if let Some(existing_instance) = provider.get_instance_by_name(cluster_name).await? {
      // Add the existing host to our collection
      info!(
        "Found existing instance from provider: {:?}",
        existing_instance
      );
      hosts.push(Arc::new(Host::try_from(existing_instance)?));
      return Ok(());
    }

    // No existing host found, try to create one using the key pair
    let key_pair = self
      .key_pair()
      .as_ref()
      .ok_or_else(|| format!("Key pair is not set on cluster: {}", cluster_name))?
      .clone();

    // Create exactly one host and get the first one from the returned vector
    let new_hosts = provider
      .create_instances(cluster_name, image, instance_type, &key_pair.name, 1)
      .await?
      .into_iter()
      .map(Host::try_from)
      .collect::<Result<Vec<Host>, String>>()?;

    // Get the first host from the returned vector
    let new_host = new_hosts
      .into_iter()
      .next()
      .ok_or_else(|| String::from("No host was created"))?;

    // Add the new host to our collection
    hosts.push(Arc::new(new_host));

    Ok(())
  }

  pub async fn primary_ip(&self) -> Result<IPAddress, Box<dyn std::error::Error>> {
    let provider = self.provider();
    let cluster_name = self.name();

    // Check if we already have an IP address for this cluster
    if let Ok(Some((public_ip, id))) = provider.get_ip_address_by_name(cluster_name).await {
      info!("Found existing IP address: {}", public_ip);
      return Ok(IPAddress {
        name: cluster_name.into(),
        public_ip,
        id,
      });
    }

    // No existing IP address found, create a new one
    let (public_ip, id) = provider.primary_ip_address(cluster_name).await?;
    info!("Created new IP address: {:?}", public_ip);

    Ok(IPAddress {
      name: cluster_name.into(),
      public_ip,
      id,
    })
  }

  pub async fn attach_ip(&self) -> Result<(), Box<dyn std::error::Error>> {
    let provider = self.provider();

    let primary_ip = self.primary_ip().await?;
    let hosts = self.hosts();

    // First check if we already have a host with matching name in our collection
    let host = hosts
      .iter()
      .next()
      .ok_or_else(|| String::from("No host was available, create one first"))?;

    if host.public_ip == primary_ip.public_ip {
      info!("IP address is already attached to host: {}", host.name);
      return Ok(());
    }

    provider
      .attach_ip_address_to_instance(&primary_ip.id, &host.id)
      .await?;

    Ok(())
  }

  pub async fn ssh_install(&self) -> Result<(), Box<dyn std::error::Error>> {
    let key_pair = self
      .key_pair()
      .as_ref()
      .ok_or_else(|| format!("Key pair is not set on cluster: {}", self.name()))?
      .clone();

    // Create an Arc-wrapped installer so it can be shared across tasks
    let installer = Installer::new(key_pair.clone(), "ubuntu", None);

    // Execute all installations in parallel
    let hosts = self.hosts();

    // Create a JoinSet to collect and manage all the tasks
    let mut set = task::JoinSet::new();

    for host in hosts.iter() {
      let host_ref = host.clone();
      let installer_ref = installer.clone();

      set.spawn_local(async move {
        match installer_ref.install_to_host(&host_ref).await {
          Ok(_) => {
            info!("SSH installation successful for host: {:?}", host_ref);
            true
          }
          Err(err) => {
            warn!(
              "SSH installation failed for host: {:?}, error: {:?}",
              host_ref, err
            );
            false
          }
        }
      });
    }

    while let Some(result) = set.join_next().await {
      match result {
        Ok(true) => info!("Installation completed successfully"),
        Ok(false) => warn!("Installation encountered an error"),
        Err(err) => warn!("Installation task failed: {:?}", err),
      }
    }

    Ok(())
  }
}
