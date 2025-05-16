use crate::builder;
use rhai::plugin::*;

#[export_module]
pub mod cluster_module {
  use std::path::Path;
  use std::sync::Arc;

  use crate::builder::IPAddress;
  use crate::provider::*;
  use crate::ssh::Installer;
  use tokio::{runtime::Handle, task};
  use tracing::info;
  use tracing::warn;

  pub type Cluster = builder::Cluster;

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
  #[rhai_fn(return_raw)]
  pub fn set_key_pair(
    cluster: &mut Cluster,
    private_key: &str,
    public_key: &str,
  ) -> Result<(), Box<EvalAltResult>> {
    // Convert the string path to PathBuf
    let private_key_path = Path::new(private_key);
    let public_key_path = Path::new(public_key);

    task::block_in_place(|| {
      let handle = Handle::current();

      // Use the improved update_key_pair that handles Results
      cluster.update_key_pair(|_current_key_pair| {
        let provider = cluster.provider();
        let name = cluster.name();

        // Check if key pair exists
        let existing_fingerprint = handle
          .block_on(provider.get_key_pair_by_name(name))
          .map_err(|e| format!("Failed to check for existing key pair: {}", e))?;

        // If we have an existing fingerprint, check if it matches
        if let Some(fingerprint) = existing_fingerprint {
          let fingerprints_match = handle
            .block_on(builder::KeyPair::fingerprint_matches_local_public_key(
              &fingerprint,
              public_key_path,
            ))
            .map_err(|e| format!("Failed to compare fingerprints: {}", e))?;

          if !fingerprints_match {
            return Err("Key pair exists in AWS but has a different fingerprint".into());
          }

          // Fingerprints match, return the existing key pair
          Ok(Some(builder::KeyPair {
            name: name.to_string(),
            private_key: private_key_path.to_path_buf(),
            fingerprint,
          }))
        } else {
          // No existing key pair, import a new one
          let key_pair = handle
            .block_on(provider.import_public_key(name, private_key_path, public_key_path))
            .map_err(|e| format!("Failed to import public key: {}", e))?;

          Ok(Some(key_pair))
        }
      })
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
  pub fn attach_ip(cluster: &mut Cluster, host: builder::Host, address: IPAddress) -> bool {
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
            info!("Spawning SSH installation task for host: {:?}", &host_clone);

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

            info!("Spawned SSH installation task for host");
          });

          // Use JoinSet to await all tasks and collect results
          let mut all_success = true;
          while let Some(result) = set.join_next().await {
            info!("Awaiting a thread to finish");
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
            info!("Thread finished");
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

#[export_module]
pub mod host_module {
  use tracing::warn;

  pub type Host = builder::Host;

  /// Install the daemon via SSH
  pub fn ssh_install(host: &mut Host) -> bool {
    // Placeholder for SSH implementation
    warn!(
      "SSH install not yet implemented - would connect to {}",
      host.public_ip
    );
    true
  }
}

#[export_module]
pub mod utils_module {
  use std::io::{self, Write};

  pub fn ask(prompt: String) -> bool {
    let mut input = String::new();
    print!("{} (y/n): ", prompt);
    io::stdout().flush().unwrap();

    io::stdin().read_line(&mut input).unwrap();
    matches!(input.trim().to_lowercase().as_str(), "y")
  }
}
