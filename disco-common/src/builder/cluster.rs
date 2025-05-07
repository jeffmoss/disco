use super::{Host, KeyPair};
use crate::provider::*;
use rhai::plugin::*;
use rhai::{CustomType, TypeBuilder};
use std::path::Path;
use std::sync::{Arc, RwLock};
use tokio::{runtime::Handle, task};
use tracing::{info, warn};

#[derive(Debug)]
struct ClusterInner {
  name: String,
  key_pair: RwLock<Option<KeyPair>>,
  provider: Arc<dyn Provider>,
  hosts: Hosts,
}

#[derive(Debug)]
pub struct Hosts(RwLock<Vec<Arc<Host>>>);

impl Hosts {
  pub fn new() -> Self {
    Hosts(RwLock::new(Vec::new()))
  }

  // pub fn primary(&self) -> Option<Arc<Host>> {
  //   let guard = self.0.read().unwrap();
  //   guard.first().cloned()
  // }

  pub fn for_each<F>(&self, mut f: F)
  where
    F: FnMut(&Arc<Host>),
  {
    let guard = self.0.read().unwrap();
    for host in &*guard {
      f(host);
    }
  }

  pub fn append<F, R>(&self, f: F) -> R
  where
    F: FnOnce(&mut Vec<Arc<Host>>) -> R,
  {
    let mut guard = self.0.write().unwrap();
    f(&mut guard)
  }
}

#[derive(Debug, Clone, CustomType)]
pub struct Cluster {
  inner: Arc<ClusterInner>,
}

impl Cluster {
  pub fn new(name: String, provider: impl Provider + 'static) -> Self {
    Cluster {
      inner: Arc::new(ClusterInner {
        name,
        key_pair: RwLock::new(None),
        provider: Arc::new(provider),
        hosts: Hosts::new(),
      }),
    }
  }

  pub fn name(&self) -> &str {
    &self.inner.name
  }

  pub fn provider(&self) -> Arc<dyn Provider> {
    self.inner.provider.clone()
  }

  // Getting the key pair with a closure
  pub fn with_key_pair<F, R>(&self, f: F) -> Option<R>
  where
    F: FnOnce(&KeyPair) -> R,
  {
    let guard = self.inner.key_pair.read().unwrap();
    guard.as_ref().map(f)
  }

  // Setting the key pair with a closure
  pub fn update_key_pair<F>(&self, f: F)
  where
    F: FnOnce(Option<&KeyPair>) -> Option<KeyPair>,
  {
    let mut guard = self.inner.key_pair.write().unwrap();
    *guard = f(guard.as_ref());
  }

  pub fn each_host<F>(&self, f: F)
  where
    F: FnMut(&Arc<Host>),
  {
    self.inner.hosts.for_each(f)
  }

  pub fn append_hosts<F, R>(&self, f: F) -> R
  where
    F: FnOnce(&mut Vec<Arc<Host>>) -> R,
  {
    self.inner.hosts.append(f)
  }
}

#[export_module]
pub mod cluster_module {
  use std::{fs::File, io::Read};

  use crate::ssh::Installer;

  pub type Cluster = super::Cluster;

  // Create a new cluster instance with an AwsProvider
  pub fn aws_cluster(name: &str, region: &str) -> Dynamic {
    task::block_in_place(|| {
      let handle = Handle::current();

      match handle.block_on(AwsProvider::new(name.to_string(), region.to_string())) {
        Ok(provider) => {
          // Create a new Cluster instance with the provider
          Dynamic::from(Cluster::new(name.to_string(), provider))
        }
        Err(err) => {
          warn!(
            "Failed to create AWS provider during cluster initialization: {:?}",
            err
          );

          // Return an empty Dynamic value on error, idiomatic Rhai
          // https://rhai.rs/book/rust/dynamic-return.html
          Dynamic::from(())
        }
      }
    })
  }

  pub fn healthy(cluster: &mut Cluster) -> bool {
    // No health check for now, just continue bootstrapping
    false
  }

  /// Ensure that the key_pair exists in the AWS account by creating if it doesn't exist
  pub fn set_key_pair(cluster: &mut Cluster, private_key: &str, public_key: &str) -> Dynamic {
    // Convert the string path to PathBuf
    let private_key_path = Path::new(private_key);
    let public_key_path = Path::new(public_key);

    task::block_in_place(|| {
      let handle = Handle::current();

      // Create a result variable to store the outcome
      let mut result_dynamic = Dynamic::from(());

      // Use update_key_pair with a closure that calls import_public_key while holding the lock
      cluster.update_key_pair(|_| {
        // This closure runs while holding the write lock
        match handle.block_on(cluster.provider().import_public_key(
          cluster.name(),
          private_key_path,
          public_key_path,
        )) {
          Ok(key_pair) => {
            // Store the result for returning later
            result_dynamic = Dynamic::from(key_pair.clone());
            // Return Some(key_pair) to update the key_pair field
            Some(key_pair)
          }
          Err(err) => {
            warn!("Failed to import public key: {:?}", err);
            // Return the current value (don't change anything)
            None
          }
        }
      });

      // Return the result after the update_key_pair call completes
      result_dynamic
    })
  }

  /// Ensure that the key_pair exists in the AWS account by creating if it doesn't exist
  #[rhai_fn()]
  pub fn primary_ip(cluster: &mut Cluster) -> Dynamic {
    task::block_in_place(|| {
      let handle = Handle::current();

      // Block on the async function using the existing runtime
      match handle.block_on(cluster.provider().primary_ip_address(cluster.name())) {
        Ok(address) => Dynamic::from(address),
        Err(err) => {
          warn!("Failed to acquire IP address: {:?}", err);

          // Return an empty Dynamic value on error, idiomatic Rhai
          // https://rhai.rs/book/rust/dynamic-return.html
          Dynamic::from(())
        }
      }
    })
  }

  pub fn start_instance(cluster: &mut Cluster, image: &str, instance_type: &str) -> Dynamic {
    task::block_in_place(|| {
      let handle = Handle::current();
      let provider = cluster.provider();
      let cluster_name = cluster.name();

      // Wrap most of the function body in the append_hosts closure
      let result = cluster.append_hosts(|hosts| {
        // First check if we already have a host with matching name in our collection
        for host in hosts.iter() {
          if host.name == *cluster_name {
            info!("Found existing host in memory: {:?}", host);
            // We need to clone the host to return it outside the Arc
            return Ok((**host).clone());
          }
        }

        // If not found locally, check if a host with this cluster name exists remotely
        if let Ok(Some(existing_host)) = handle.block_on(provider.get_host_by_name(cluster_name)) {
          // Add the existing host to our collection
          hosts.push(Arc::new(existing_host.clone()));

          info!("Found existing host from provider: {:?}", existing_host);
          return Ok(existing_host);
        }

        // No existing host found, try to create one using the key pair
        cluster
          .with_key_pair(|key_pair| {
            // Create exactly one host and get the first one from the returned vector
            let new_hosts = handle
              .block_on(provider.create_hosts(cluster_name, image, instance_type, key_pair, 1))
              .map_err(|err| format!("Could not create hosts: {}", err))?;

            // Get the first host from the returned vector
            let new_host = new_hosts
              .into_iter()
              .next()
              .ok_or_else(|| String::from("No host was created"))?;

            // Add the new host to our collection
            hosts.push(Arc::new(new_host.clone()));

            Ok(new_host)
          })
          .unwrap_or(Err(String::from(
            "Cannot start instance without a key pair",
          )))
      });

      // Handle the result
      match result {
        Ok(host) => {
          // Successfully got a host
          info!("Successfully started instance: {:?}", host);
          Dynamic::from(host)
        }
        Err(err_msg) => {
          // There was an error
          warn!("Failed to start instance: {}", err_msg);
          Dynamic::from(())
        }
      }
    })
  }

  /// Attach an elastic IP address to a host
  pub fn attach_ip(cluster: &mut Cluster, host: Host, address: Address) -> bool {
    // Check if the host already has the specified IP address
    if host.public_ip == address.public_ip {
      return true;
    }

    task::block_in_place(|| {
      let handle = Handle::current();

      match handle.block_on(
        cluster
          .provider()
          .attach_ip_address_to_host(&address, &host.clone()),
      ) {
        Ok(_) => true,
        Err(err) => {
          warn!("Failed to attach IP address to host: {:?}", err);
          false
        }
      }
    })
  }

  pub fn ssh_install(cluster: &mut Cluster) -> bool {
    let mut success = true;

    cluster.with_key_pair(|key_pair| {
      // Create an Arc-wrapped installer so it can be shared across tasks
      let installer = Installer::new(key_pair.clone(), "ubuntu", None);

      // Execute all installations in parallel
      task::block_in_place(|| {
        let handle = Handle::current();

        success = handle.block_on(async {
          // Create a JoinSet to collect and manage all the tasks
          let mut set = tokio::task::JoinSet::new();

          // Spawn tasks for each host
          cluster.each_host(|host| {
            let host_clone = host.clone();
            // Clone the Arc, not the installer itself
            let installer_clone = installer.clone();

            // Spawn the task into the JoinSet
            set.spawn(async move {
              match installer_clone.install_to_host(&host_clone).await {
                Ok(_) => {
                  info!("SSH installation successful for host: {:?}", host_clone);
                  true
                }
                Err(err) => {
                  warn!(
                    "SSH installation failed for host: {:?}, error: {:?}",
                    host_clone, err
                  );
                  false
                }
              }
            });
          });

          // Use JoinSet to await all tasks and collect results
          let mut all_success = true;
          while let Some(result) = set.join_next().await {
            match result {
              Ok(success) => {
                if !success {
                  all_success = false;
                }
              }
              Err(err) => {
                warn!("Task panicked: {:?}", err);
                all_success = false;
              }
            }
          }

          all_success
        });
      });
    });

    success
  }

  /// Scale the cluster to a specific number of nodes
  pub fn scale(cluster: &mut Cluster, node_count: i64) -> Dynamic {
    task::block_in_place(|| {
      let handle = Handle::current();

      // We should make an API call to the daemon in order to scale the cluster

      Dynamic::from(())
    })
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  // #[test]
  // fn test_new_cluster() {
  //   let cluster = Cluster::new("test".to_string(), );
  //   assert_eq!(cluster.name(), "test");
  // }

  // #[test]
  // fn test_thread_safety() {
  //   let cluster = Cluster::new("shared".to_string());
  //   let cluster_clone = cluster.clone();

  //   let handle = std::thread::spawn(move || {
  //     cluster_clone.set_region("us-west".to_string());
  //   });
  //   let empty_fn = FnPtr::new("empty_fn").unwrap();
  //   cluster.set_bootstrap(empty_fn);
  //   handle.join().unwrap();

  //   assert_eq!(cluster.get_region(), "us-west");
  // }
}
