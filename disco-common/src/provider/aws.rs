use aws_config::{self, BehaviorVersion};
use aws_sdk_ec2::{config::Region, primitives::Blob, types::InstanceType, Client};
use std::path::PathBuf;

use crate::provider::Provider;

pub struct AwsProvider {
  pub client: Client,
}

impl AwsProvider {
  pub async fn new(region: Region) -> AwsProvider {
    let shared_config = aws_config::defaults(BehaviorVersion::v2025_01_17())
      .region(region)
      .load()
      .await;
    let client = Client::new(&shared_config);

    AwsProvider { client }
  }
}

impl Provider for AwsProvider {
  async fn import_public_key(
    &self,
    key_path: PathBuf,
    key_name: String,
  ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
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

    Ok(fingerprint)
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
