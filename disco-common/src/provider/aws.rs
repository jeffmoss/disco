use crate::builder::{Host, KeyPair};
use crate::provider::{Address, Provider};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use aws_config::{self, BehaviorVersion};
use aws_sdk_ec2::operation::describe_instances::DescribeInstancesOutput;
use aws_sdk_ec2::types::{
  DomainType, Filter, InstanceStateName, IpPermission, IpRange, ResourceType, Tag, TagSpecification,
};
use aws_sdk_ec2::{config::Region, primitives::Blob, types::InstanceType, Client};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct AwsProvider {
  pub cluster_name: String,
  pub client: Client,
}

/// Represents an EC2 instance that may have incomplete information
#[derive(Debug, Clone)]
pub struct InstanceInfo {
  pub id: String,
  pub name: Option<String>,
  pub public_ip: Option<String>,
  pub state: Option<InstanceStateName>,
}

impl TryInto<Host> for InstanceInfo {
  type Error = anyhow::Error;

  fn try_into(self) -> Result<Host, Self::Error> {
    // Check for required name
    let name = match self.name {
      Some(name) => name,
      None => bail!("Instance '{}' does not have a name tag", self.id),
    };

    // Check for required public IP
    let public_ip = match self.public_ip {
      Some(ip) => ip,
      None => bail!(
        "Instance '{}' ({}) does not have a public IP address",
        name,
        self.id
      ),
    };

    Ok(Host {
      name,
      id: self.id,
      public_ip,
    })
  }
}

impl AwsProvider {
  fn instances_from_response(&self, resp: &DescribeInstancesOutput) -> Result<Vec<InstanceInfo>> {
    let mut instances = Vec::new();

    // Process all reservations
    for reservation in resp.reservations() {
      // Process all instances in this reservation
      for instance in reservation.instances() {
        // Get the instance ID, bail if not available
        let id = match instance.instance_id() {
          Some(id) => id.to_string(),
          None => bail!("Encountered an instance without an ID in AWS response"),
        };

        // Get name from tags (now optional)
        let name = instance
          .tags()
          .iter()
          .find(|tag| tag.key() == Some("Name"))
          .and_then(|tag| tag.value().map(|v| v.to_string()));

        // Get the instance state if available
        let state = instance.state().and_then(|s| s.name().cloned());

        // Get the public IP as an Option
        let public_ip = instance.public_ip_address().map(|ip| ip.to_string());

        // Create and add the InstanceInfo to our collection
        instances.push(InstanceInfo {
          name,
          id,
          public_ip,
          state,
        });
      }
    }

    Ok(instances)
  }

  /// Look for the named security group, create it if it doesn't exist, allowing traffic on port 22
  async fn security_group(&self, name: &str) -> Result<String> {
    // First, try to find existing security group by name
    let resp = self
      .client
      .describe_security_groups()
      .filters(Filter::builder().name("group-name").values(name).build())
      .send()
      .await
      .with_context(|| format!("Failed to query AWS for security group '{}'", name))?;

    // If security group exists, return its ID
    if let Some(group) = resp.security_groups().first() {
      return Ok(
        group
          .group_id()
          .ok_or_else(|| anyhow::anyhow!("Security group exists but has no ID"))?
          .to_string(),
      );
    }

    // Security group not found, create a new one
    let vpc_id = self
      .get_default_vpc_id()
      .await
      .context("Failed to get default VPC ID when creating security group")?;

    // Create security group
    let create_resp = self
      .client
      .create_security_group()
      .group_name(name)
      .description(format!("Security group for SSH access to {}", name))
      .vpc_id(vpc_id)
      .tag_specifications(
        TagSpecification::builder()
          .resource_type(ResourceType::SecurityGroup)
          .tags(Tag::builder().key("Name").value(name).build())
          .build(),
      )
      .send()
      .await
      .with_context(|| format!("Failed to create security group '{}'", name))?;

    let group_id = create_resp
      .group_id()
      .ok_or_else(|| anyhow::anyhow!("No group ID returned after creating security group"))?
      .to_string();

    // Add inbound rule for SSH (port 22)
    self
      .client
      .authorize_security_group_ingress()
      .group_id(&group_id)
      .ip_permissions(
        IpPermission::builder()
          .ip_protocol("tcp")
          .from_port(22)
          .to_port(22)
          .ip_ranges(
            IpRange::builder()
              .cidr_ip("0.0.0.0/0")
              .description("Allow SSH access from anywhere")
              .build(),
          )
          .build(),
      )
      .send()
      .await
      .with_context(|| format!("Failed to add SSH rule to security group '{}'", name))?;

    Ok(group_id)
  }

  // Helper method to get the default VPC ID
  async fn get_default_vpc_id(&self) -> Result<String> {
    let resp = self
      .client
      .describe_vpcs()
      .filters(Filter::builder().name("isDefault").values("true").build())
      .send()
      .await
      .context("Failed to query for default VPC")?;

    let vpc_id = resp
      .vpcs()
      .first()
      .and_then(|vpc| vpc.vpc_id())
      .ok_or_else(|| anyhow::anyhow!("No default VPC found"))?;

    Ok(vpc_id.to_string())
  }
}

#[async_trait]
impl Provider for AwsProvider {
  async fn new(cluster_name: String, region: String) -> Result<Self> {
    let shared_config = aws_config::defaults(BehaviorVersion::v2025_01_17())
      .region(Region::new(region))
      .load()
      .await;

    let client = Client::new(&shared_config);

    Ok(AwsProvider {
      cluster_name,
      client,
    })
  }

  async fn get_ip_address_by_name(&self, name: &str) -> Result<Option<Address>> {
    // Get the list of Elastic IP addresses
    let resp = self
      .client
      .describe_addresses()
      .filters(Filter::builder().name("tag:Name").values(name).build())
      .send()
      .await
      .with_context(|| {
        format!(
          "Failed to query AWS for IP addresses with name tag '{}'",
          name
        )
      })?;

    // Check if addresses are empty and return None, otherwise return the public IP
    match resp.addresses().get(0) {
      Some(address) => {
        let public_ip = match address.public_ip() {
          Some(ip) => ip.to_string(),
          None => bail!(
            "No public IP returned from AWS on tag lookup for '{}'",
            name
          ),
        };

        let allocation_id = match address.allocation_id() {
          Some(id) => id.to_string(),
          None => bail!(
            "No allocation ID returned from AWS on tag lookup for '{}'",
            name
          ),
        };

        return Ok(Some(Address {
          name: name.to_string(),
          public_ip,
          id: allocation_id,
        }));
      }
      None => return Ok(None),
    };
  }

  async fn attach_ip_address_to_host(&self, address: &Address, host: &Host) -> Result<()> {
    // Associate the Elastic IP with the instance
    self
      .client
      .associate_address()
      .instance_id(&host.id)
      .allocation_id(&address.id)
      .send()
      .await
      .with_context(|| {
        format!(
          "Failed to associate IP address {:?} with instance {:?}",
          address, host
        )
      })?;

    Ok(())
  }

  async fn get_key_pair_by_name(&self, name: &str) -> Result<Option<String>> {
    // Get the list of key pairs with the Name tag
    let resp = self
      .client
      .describe_key_pairs()
      .key_names(name)
      .send()
      .await
      .with_context(|| format!("Failed to retrieve key pair '{}' from AWS", name))?;

    // Check if key pairs are empty and return None, otherwise return the key pair
    match resp.key_pairs().get(0) {
      Some(key_pair) => {
        let fingerprint = match key_pair.key_fingerprint() {
          Some(fp) => fp.to_string(),
          None => bail!("No key pair fingerprint returned from AWS for '{}'", name),
        };

        Ok(Some(fingerprint))
      }
      None => Ok(None),
    }
  }

  async fn import_public_key(
    &self,
    name: &str,
    private_key_path: &Path,
    public_key_path: &Path,
  ) -> Result<KeyPair> {
    if let Some(fingerprint) = self.get_key_pair_by_name(name).await? {
      // TODO: check if the fingerprint matches and update if needed
      return Ok(KeyPair {
        name: name.to_string(),
        private_key: private_key_path.into(),
        fingerprint,
      });
    }

    // Read the public key file
    let public_key = tokio::fs::read(public_key_path)
      .await
      .with_context(|| format!("Failed to read public key file at {:?}", public_key_path))?;

    // Import the key pair to AWS with tag in a single API call
    let resp = self
      .client
      .import_key_pair()
      .key_name(name)
      .public_key_material(Blob::new(public_key))
      .send()
      .await
      .with_context(|| format!("Failed to import key pair '{}' to AWS", name))?;

    let fingerprint = match resp.key_fingerprint() {
      Some(fp) => fp.to_string(),
      None => bail!(
        "No fingerprint returned from AWS when importing key pair '{}'",
        name
      ),
    };

    Ok(KeyPair {
      name: name.to_string(),
      private_key: private_key_path.into(),
      fingerprint,
    })
  }

  async fn primary_ip_address(&self, name: &str) -> Result<Address> {
    if let Some(address) = self.get_ip_address_by_name(name).await? {
      return Ok(address);
    }

    // Allocate a new Elastic IP address
    let resp = self
      .client
      .allocate_address()
      .domain(DomainType::Vpc)
      // Add the tag specification to tag the IP address during allocation
      .tag_specifications(
        TagSpecification::builder()
          .resource_type(ResourceType::ElasticIp)
          .tags(Tag::builder().key("Name").value(name).build())
          .build(),
      )
      .send()
      .await
      .with_context(|| format!("Failed to allocate new Elastic IP address for '{}'", name))?;

    let allocation_id = match resp.allocation_id() {
      Some(id) => id.to_string(),
      None => bail!(
        "No allocation ID returned from AWS after creating primary IP for '{}'",
        name
      ),
    };

    let public_ip = match resp.public_ip() {
      Some(ip) => ip.to_string(),
      None => bail!(
        "No public IP returned from AWS after creating primary IP for '{}'",
        name
      ),
    };

    Ok(Address {
      name: name.to_string(),
      public_ip,
      id: allocation_id,
    })
  }

  async fn get_host_by_name(&self, name: &str) -> Result<Option<Host>> {
    // Get the list of EC2 instances with the given name
    let resp = self
      .client
      .describe_instances()
      .filters(Filter::builder().name("tag:Name").values(name).build())
      .send()
      .await
      .with_context(|| {
        format!(
          "Failed to query AWS for EC2 instances with name tag '{}'",
          name
        )
      })?;

    // Use the helper to extract all instances
    let instances = self.instances_from_response(&resp).with_context(|| {
      format!(
        "Failed to parse instances from AWS response for name '{}'",
        name
      )
    })?;

    // Find the first non-terminated instance
    let instance = instances
      .into_iter()
      .find(|instance| instance.state != Some(InstanceStateName::Terminated));

    // Convert the found instance to a Host if it exists
    match instance {
      Some(instance) => {
        // Try to convert to Host
        Ok(Some(instance.try_into()?))
      }
      None => Ok(None),
    }
  }

  async fn wait_for_hosts(
    &self,
    instance_ids: &[String],
    timeout_seconds: u64,
    poll_interval_seconds: u64,
  ) -> Result<Vec<Host>> {
    let start_time = tokio::time::Instant::now();
    let timeout = tokio::time::Duration::from_secs(timeout_seconds);
    let poll_interval = tokio::time::Duration::from_secs(poll_interval_seconds);

    // Track which instances are ready
    let mut ready_hosts = Vec::with_capacity(instance_ids.len());
    let mut pending_instance_ids: Vec<String> = instance_ids.to_vec();

    loop {
      // Check if we've exceeded the timeout
      if start_time.elapsed() > timeout {
        bail!(
                "Timed out waiting for instances to become available with public IPs. Ready: {}, Pending: {}",
                ready_hosts.len(),
                pending_instance_ids.len()
            );
      }

      // Wait before polling
      tokio::time::sleep(poll_interval).await;

      // No more pending instances, we're done
      if pending_instance_ids.is_empty() {
        break;
      }

      // Query the instance status for all pending instances
      let resp = self
        .client
        .describe_instances()
        .set_instance_ids(Some(pending_instance_ids.clone()))
        .send()
        .await
        .with_context(|| format!("Failed to describe instances: {:?}", pending_instance_ids))?;

      // Get all instance info objects
      let instances = self
        .instances_from_response(&resp)
        .context("Failed to parse instances from AWS response")?;

      // Process each instance - track which IDs are still pending
      let mut new_pending = Vec::new();

      // First, identify ready instances and collect their IDs
      let mut ready_ids = Vec::new();
      for instance in &instances {
        if instance.state == Some(InstanceStateName::Running) && instance.public_ip.is_some() {
          ready_ids.push(instance.id.clone());
        } else {
          new_pending.push(instance.id.clone());
        }
      }

      // Now convert ready instances to Host objects and add them to ready_hosts
      for instance in instances {
        if ready_ids.contains(&instance.id) {
          // This will never fail since we only included IDs where public_ip.is_some()
          let host: Host = instance.try_into()?;
          ready_hosts.push(host);
        }
      }

      // Update pending list
      pending_instance_ids = new_pending;
    }

    Ok(ready_hosts)
  }

  // NOTE: This function signature doesn't allow more than 1 without a naming
  // convention. Name is being used to identify the primary instance here.
  async fn create_hosts(
    &self,
    name: &str,
    image: &str,
    instance_type: &str,
    key_pair: &KeyPair,
    count: i64,
  ) -> Result<Vec<Host>> {
    // Convert i64 count to i32 for AWS SDK
    let count_i32 = match i32::try_from(count) {
      Ok(val) => val,
      Err(_) => bail!(
        "Invalid instance count: {} (must fit within i32 range)",
        count
      ),
    };

    let security_group_id = self
      .security_group(name)
      .await
      .with_context(|| format!("Failed to get or create security group for '{}'", name))?;

    // Create EC2 instances
    let resp = self
      .client
      .run_instances()
      .image_id(image)
      .instance_type(InstanceType::from(instance_type))
      .min_count(count_i32)
      .max_count(count_i32)
      // Set the key pair for SSH access
      .key_name(key_pair.name.as_str())
      // Set the security group to allow SSH access
      .security_group_ids(security_group_id)
      // Add a Name tag to identify these instances
      .tag_specifications(
        TagSpecification::builder()
          .resource_type(ResourceType::Instance)
          .tags(Tag::builder().key("Name").value(name).build())
          .build(),
      )
      .send()
      .await
      .with_context(|| {
        format!(
          "Failed to create {} EC2 instances with image {} and type {}",
          count, image, instance_type
        )
      })?;

    // Extract all instances from the response
    let instances = resp.instances();
    if instances.is_empty() {
      bail!(
        "No instances returned from AWS when creating hosts for '{}'",
        name
      );
    }

    // Collect all instance IDs
    let instance_ids: Vec<String> = instances
      .iter()
      .filter_map(|instance| instance.instance_id().map(|id| id.to_string()))
      .collect();

    if instance_ids.is_empty() {
      bail!(
        "No valid instance IDs found in AWS response when creating hosts for '{}'",
        name
      );
    }

    // Hard-coded timeout values
    // TODO: Should this stuff be configurable?
    let timeout_seconds = 300;
    let poll_interval_seconds = 5;

    // Wait for all instances to be running and have public IPs
    let hosts = self
      .wait_for_hosts(&instance_ids, timeout_seconds, poll_interval_seconds)
      .await
      .with_context(|| {
        format!(
          "Failed while waiting for hosts to become available for '{}'",
          name
        )
      })?;

    Ok(hosts)
  }
}
