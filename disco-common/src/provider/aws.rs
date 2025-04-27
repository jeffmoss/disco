use crate::provider::{Address, KeyPair, Provider};
use async_trait::async_trait;
use aws_config::{self, BehaviorVersion};
use aws_sdk_ec2::types::{DomainType, Filter, Tag};
use aws_sdk_ec2::{config::Region, primitives::Blob, types::InstanceType, Client};
use rhai::plugin::*;
use rhai::{CustomType, Map, TypeBuilder};
use std::path::PathBuf;
use tokio::runtime::Handle;
use tokio::task;
use tracing::warn;

#[derive(Debug, Clone, CustomType)]
pub struct AwsProvider {
  pub name: String,
  pub client: Client,
}

impl AwsProvider {
  pub async fn new<S: Into<String>>(name: S, region: S) -> AwsProvider {
    let shared_config = aws_config::defaults(BehaviorVersion::v2025_01_17())
      .region(Region::new(region.into()))
      .load()
      .await;

    let client = Client::new(&shared_config);

    AwsProvider {
      name: name.into(),
      client,
    }
  }

  async fn get_ip_address_by_cluster_name(
    &self,
    name: &str,
  ) -> Result<Option<Address>, Box<dyn std::error::Error + Send + Sync>> {
    // Get the list of Elastic IP addresses
    let resp = self
      .client
      .describe_addresses()
      .filters(Filter::builder().name("tag:cluster").values(name).build())
      .send()
      .await?;

    // Check if addresses are empty and return None, otherwise return the public IP
    match resp.addresses().get(0) {
      Some(address) => {
        return Ok(Some(Address {
          name: name.to_string(),
          public_ip: address
            .public_ip()
            .ok_or("No public IP returned from AWS on tag lookup")?
            .to_string(),
          fingerprint: address
            .allocation_id()
            .ok_or("No allocation ID returned from AWS on tag lookup")?
            .to_string(),
        }))
      }
      None => return Ok(None),
    };
  }
}

#[async_trait]
impl Provider for AwsProvider {
  async fn import_public_key(
    &self,
    key_path: PathBuf,
    key_name: &String,
  ) -> Result<KeyPair, Box<dyn std::error::Error + Send + Sync>> {
    // Read the public key file
    let public_key = tokio::fs::read(&key_path).await?;

    // Import the key pair to AWS
    let resp = self
      .client
      .import_key_pair()
      .key_name(key_name.clone())
      .public_key_material(Blob::new(public_key))
      .send()
      .await?;

    let fingerprint = resp
      .key_fingerprint()
      .ok_or("No fingerprint returned from AWS")?
      .to_string();

    Ok(KeyPair {
      name: key_name.clone(),
      fingerprint,
    })
  }

  async fn create_ip_address(
    &self,
    name: &str,
  ) -> Result<Address, Box<dyn std::error::Error + Send + Sync>> {
    if let Some(address) = self.get_ip_address_by_cluster_name(name).await? {
      return Ok(address);
    }

    // Allocate a new Elastic IP address
    let resp = self
      .client
      .allocate_address()
      .domain(DomainType::Vpc)
      .send()
      .await?;

    let allocation_id = resp
      .allocation_id()
      .ok_or("No allocation ID returned from AWS after creation")?
      .to_string();

    // Tag the IP address with our cluster name
    self
      .client
      .create_tags()
      .resources(allocation_id.clone())
      .tags(Tag::builder().key("cluster").value(name).build())
      .send()
      .await?;

    return Ok(Address {
      name: name.to_string(),
      public_ip: resp
        .public_ip()
        .ok_or("No public IP returned from AWS after creation")?
        .to_string(),
      fingerprint: allocation_id,
    });
  }

  async fn create_host(
    &self,
    image_id: String,
  ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // Create a basic EC2 instance
    let resp = self
      .client
      .run_instances()
      .image_id(image_id) // Amazon Linux 2 AMI (adjust as needed)
      .instance_type(InstanceType::T4gMicro)
      .min_count(1)
      .max_count(1)
      .send()
      .await
      .expect("Failed to create EC2 instance");

    let instance_id = resp
      .instances()
      .first()
      .expect("No instance created")
      .instance_id()
      .expect("Instance has no ID")
      .to_string();

    Ok(instance_id)
  }
}

#[export_module]
pub mod aws_provider_module {
  pub type Address = super::Address;
  pub type KeyPair = super::KeyPair;

  #[rhai_fn()]
  pub fn aws_provider(name: String) -> super::AwsProvider {
    task::block_in_place(|| {
      let handle = Handle::current();

      handle.block_on(super::AwsProvider::new(name, "us-west-2".to_string()))
    })
  }

  /// Ensure that the key_pair exists in the AWS account by creating if it doesn't exist
  #[rhai_fn()]
  pub fn key_pair(provider: &mut super::AwsProvider, config: Map) -> Dynamic {
    // Convert the string path to PathBuf
    let name = config.get("name").unwrap().clone().into_string().unwrap();
    let path = PathBuf::from(
      config
        .get("public_key_path")
        .unwrap()
        .clone()
        .into_string()
        .unwrap(),
    );

    task::block_in_place(|| {
      let handle = Handle::current();

      // Block on the async function using the existing runtime
      match handle.block_on(provider.import_public_key(path, &name)) {
        Ok(key_pair) => Dynamic::from(key_pair),
        Err(err) => {
          warn!("Failed to import public key: {:?}", err);

          // Return an empty Dynamic value on error, idiomatic Rhai
          // https://rhai.rs/book/rust/dynamic-return.html
          Dynamic::from(())
        }
      }
    })
  }

  /// Ensure that the key_pair exists in the AWS account by creating if it doesn't exist
  #[rhai_fn()]
  pub fn primary_ip(provider: &mut super::AwsProvider, config: Map) -> Dynamic {
    // Convert the string path to PathBuf
    let name = config.get("name").unwrap().clone().into_string().unwrap();

    task::block_in_place(|| {
      let handle = Handle::current();

      // Block on the async function using the existing runtime
      match handle.block_on(provider.create_ip_address(&name)) {
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
}
