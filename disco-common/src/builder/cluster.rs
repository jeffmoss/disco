use super::{Host, KeyPair};
use crate::provider::*;

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

#[derive(Debug, Clone)]
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
  pub fn update_key_pair<F, E>(&self, f: F) -> Result<(), E>
  where
    F: FnOnce(Option<&KeyPair>) -> Result<Option<KeyPair>, E>,
  {
    let mut guard = self.inner.key_pair.write().unwrap();
    match f(guard.as_ref()) {
      Ok(new_key_pair) => {
        *guard = new_key_pair;
        Ok(())
      }
      Err(e) => Err(e),
    }
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
