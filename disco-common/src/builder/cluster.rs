use crate::engine;
use crate::provider::{AwsProvider, Provider};
use rhai::plugin::*;
use rhai::{CustomType, FnPtr, TypeBuilder};
use std::sync::{Arc, RwLock};
use tracing::{info, warn};

// Inner data structure
struct ClusterInner {
  name: String,
  provider: Box<dyn Provider>,
}

// Public wrapper that hides Arc/RwLock implementation
#[derive(Clone, CustomType)]
pub struct Cluster {
  inner: Arc<RwLock<ClusterInner>>,
}

impl Cluster {
  pub fn new(name: String, provider: impl Provider + 'static) -> Self {
    Cluster {
      inner: Arc::new(RwLock::new(ClusterInner {
        name,
        provider: Box::new(provider),
      })),
    }
  }

  pub fn get_name(&self) -> String {
    let guard = self.inner.read().unwrap();
    guard.name.clone()
  }

  pub fn set_name<S: Into<String>>(&self, name: S) {
    let mut guard = self.inner.write().unwrap();
    guard.name = name.into();
  }

  // Get the cluster name
  pub fn name(&self) -> String {
    let guard = self.inner.read().unwrap();
    guard.name.clone()
  }

  // Set the bootstrap function
  pub fn set_bootstrap(&self, func: FnPtr) {
    let mut guard = self.inner.write().unwrap();
    guard.bootstrap = Some(func);
  }

  // Set the region
  pub fn set_region(&self, region: String) -> Self {
    let mut guard = self.inner.write().unwrap();
    guard.region = Some(region);
    self.clone()
  }

  // Get the region
  pub fn get_region(&self) -> String {
    let guard = self.inner.read().unwrap();
    guard
      .region
      .clone()
      .unwrap_or_else(|| "not set".to_string())
  }

  pub fn run_bootstrap(&self, engine: &engine::Engine) {
    let guard = self.inner.read().unwrap();
    if let Some(bootstrap) = &guard.bootstrap {
      // Call the bootstrap function
      let result = if bootstrap.is_curried() {
        bootstrap.call::<()>(&engine.rhai_engine, &engine.ast, (self.clone(),))
      } else {
        engine.rhai_engine.call_fn::<()>(
          &mut *engine.scope.lock().unwrap(),
          &engine.ast,
          bootstrap.fn_name(),
          (self.clone(),),
        )
      };

      if let Err(e) = result {
        warn!("Failed to execute bootstrap function: {:?}", e);
      }
    } else {
      warn!("No bootstrap function set");
    }
  }
}

#[export_module]
pub mod cluster_module {
  pub type Cluster = super::Cluster;

  #[rhai_fn()]
  pub fn aws_cluster(name: String, region: String) -> Cluster {
    let provider = AwsProvider::new(name, region);

    Cluster::new(name, provider)
  }

  #[rhai_fn(get = "name", pure)]
  pub fn get_name(cluster: &mut Cluster) -> String {
    cluster.get_name()
  }

  #[rhai_fn(set = "name")]
  pub fn set_name(cluster: &mut Cluster, name: String) {
    cluster.set_name(name);
  }

  #[rhai_fn(set = "region")]
  pub fn set_region(cluster: &mut Cluster, region: String) {
    cluster.set_region(region);
  }

  #[rhai_fn(get = "region", pure)]
  pub fn get_region(cluster: &mut Cluster) -> String {
    cluster.get_region()
  }

  #[rhai_fn(set = "bootstrap")]
  pub fn set_bootstrap(cluster: &mut Cluster, func: FnPtr) {
    cluster.set_bootstrap(func);
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_new_cluster() {
    let cluster = Cluster::new("test".to_string());
    assert_eq!(cluster.name(), "test");
  }

  #[test]
  fn test_thread_safety() {
    let cluster = Cluster::new("shared".to_string());
    let cluster_clone = cluster.clone();

    let handle = std::thread::spawn(move || {
      cluster_clone.set_region("us-west".to_string());
    });
    let empty_fn = FnPtr::new("empty_fn").unwrap();
    cluster.set_bootstrap(empty_fn);
    handle.join().unwrap();

    assert_eq!(cluster.get_region(), "us-west");
  }
}
