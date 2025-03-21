use rhai::plugin::*;
use rhai::{CustomType, TypeBuilder};
use tracing::info;

#[derive(Clone, CustomType, Debug)]
pub struct Cluster {
  name: String,

  region: Option<String>,
}

impl Cluster {
  pub fn new(name: String) -> Self {
    info!("Created cluster: {name}");

    Cluster { name, region: None }
  }
}

#[export_module]
pub mod cluster_module {
  pub type Cluster = super::Cluster;

  #[rhai_fn()]
  pub fn aws_cluster(name: String) -> Cluster {
    Cluster::new(name)
  }

  #[rhai_fn(name = "region")]
  pub fn set_region(cluster: &mut Cluster, region: String) -> Cluster {
    cluster.region = Some(region);
    cluster.clone()
  }

  #[rhai_fn(get = "region", pure)]
  pub fn get_region(cluster: &mut Cluster) -> String {
    cluster
      .region
      .clone()
      .unwrap_or_else(|| "not set".to_string())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_new_cluster() {
    let cluster = Cluster::new("test".to_string());
    assert_eq!(cluster.name, "test");
  }
}
