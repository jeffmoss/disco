use crate::provider::{InstanceInfo, InstanceState, Provider};
use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use aws_config::{self, BehaviorVersion};
use aws_sdk_ec2::operation::describe_instances::DescribeInstancesOutput;
use aws_sdk_ec2::types::{
  DomainType, Filter, IamInstanceProfileSpecification, InstanceStateName, InstanceType,
  IpPermission, IpRange, ResourceType, Tag as EC2Tag, TagSpecification as EC2TagSpecification,
  UserIdGroupPair,
};
use aws_sdk_ec2::{Client as EC2Client, config::Region, primitives::Blob};
use aws_sdk_iam::Client as IAMClient;
use aws_sdk_iam::types::Tag as IAMTag;
use boa_engine::JsData;
use boa_gc::{Finalize, Trace};
use core::panic;
use std::path::Path;
use tokio::runtime::Handle;
use tokio::task;

impl From<InstanceStateName> for InstanceState {
  fn from(state: InstanceStateName) -> Self {
    match state {
      InstanceStateName::Pending => InstanceState::Pending,
      InstanceStateName::Running => InstanceState::Running,
      InstanceStateName::ShuttingDown => InstanceState::ShuttingDown,
      InstanceStateName::Terminated => InstanceState::Terminated,
      InstanceStateName::Stopping => InstanceState::Stopping,
      InstanceStateName::Stopped => InstanceState::Stopped,
      _ => panic!("Unknown instance state: {:?}", state),
    }
  }
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "js", derive(Trace, Finalize, JsData))]
pub struct AwsProvider {
  pub cluster_name: String,

  #[unsafe_ignore_trace]
  pub ec2_client: EC2Client,

  #[unsafe_ignore_trace]
  pub iam_client: IAMClient,
}

impl Default for AwsProvider {
  fn default() -> Self {
    task::block_in_place(|| {
      let handle = Handle::current();

      handle
        .block_on(Self::new("disco".into(), "us-west-2".into()))
        .expect("Failed to create provider")
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
          state: state.map(InstanceState::from),
        });
      }
    }

    Ok(instances)
  }

  /// Look for the named IAM role, create it if it doesn't exist with EC2 permissions
  async fn iam_role(&self, name: &str) -> Result<String> {
    // First, try to find existing role by name
    match self.iam_client.get_role().role_name(name).send().await {
      Ok(resp) => {
        // Role exists, return its name
        return Ok(
          resp
            .role()
            .ok_or_else(|| anyhow::anyhow!("IAM role exists but has no data"))?
            .role_name()
            .to_string(),
        );
      }
      Err(err) => {
        // If error is not "role not found", return the error
        if !err.to_string().contains("NoSuchEntity") {
          return Err(err.into());
        }
        // Otherwise continue to create the role
      }
    }

    // Create the assume role policy document for EC2
    let assume_role_policy = r#"{
      "Version": "2012-10-17",
      "Statement": [
          {
              "Effect": "Allow",
              "Principal": {
                  "Service": "ec2.amazonaws.com"
              },
              "Action": "sts:AssumeRole"
          }
      ]
  }"#;

    // Create the IAM role
    let create_role_resp = self
      .iam_client
      .create_role()
      .role_name(name)
      .description(format!("Role for EC2 instances in {}", name))
      .assume_role_policy_document(assume_role_policy)
      .max_session_duration(3600) // 1 hour
      .tags(IAMTag::builder().key("Name").value(name).build()?)
      .send()
      .await
      .with_context(|| format!("Failed to create IAM role '{}'", name))?;

    let role = create_role_resp
      .role()
      .ok_or_else(|| anyhow::anyhow!("No role returned after creating IAM role"))?;

    // Attach the AmazonEC2FullAccess policy to the role
    self
      .iam_client
      .attach_role_policy()
      .role_name(name)
      .policy_arn("arn:aws:iam::aws:policy/AmazonEC2FullAccess") // Using AWS managed policy
      .send()
      .await
      .with_context(|| format!("Failed to attach EC2 policy to role '{}'", name))?;

    Ok(role.role_name().to_string())
  }

  /// Look for the named instance profile, create it if it doesn't exist and attach the IAM role
  async fn instance_profile(&self, name: &str) -> Result<String> {
    // Ensure the role exists before dealing with the instance profile
    let role_name = self.iam_role(name).await?;

    // Check if the instance profile already exists
    let (profile_name, needs_role_attachment) = match self
      .iam_client
      .get_instance_profile()
      .instance_profile_name(name)
      .send()
      .await
    {
      Ok(resp) => {
        // Instance profile exists
        let profile = resp
          .instance_profile()
          .ok_or_else(|| anyhow::anyhow!("Instance profile exists but has no data"))?;

        // Check if the profile already has the role with the same name
        let has_role = profile.roles().iter().any(|role| role.role_name() == name);

        // If role is not attached, we'll need to attach it
        (profile.instance_profile_name().to_string(), !has_role)
      }
      Err(err) => {
        // If error is not "profile not found", return the error
        if !err.to_string().contains("NoSuchEntity") {
          return Err(err.into());
        }

        // Create a new instance profile
        let response = self
          .iam_client
          .create_instance_profile()
          .instance_profile_name(name)
          .send()
          .await
          .with_context(|| format!("Failed to create instance profile '{}'", name))?;

        let instance_profile = response
          .instance_profile()
          .ok_or_else(|| anyhow::anyhow!("No instance profile returned after creating"))?;

        (instance_profile.instance_profile_name().to_string(), true)
      }
    };

    // Add the role to the instance profile if needed
    if needs_role_attachment {
      self
        .iam_client
        .add_role_to_instance_profile()
        .instance_profile_name(&profile_name)
        .role_name(role_name)
        .send()
        .await
        .with_context(|| format!("Failed to add role to instance profile '{}'", name))?;

      // Allow some time for the instance profile to propagate
      tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
    }

    Ok(profile_name)
  }

  /// Look for the named security group, create it if it doesn't exist, allowing traffic on port 22
  async fn security_group(&self, name: &str) -> Result<String> {
    // First, try to find existing security group by name
    let resp = self
      .ec2_client
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
      .ec2_client
      .create_security_group()
      .group_name(name)
      .description(format!("Security group for SSH access to {}", name))
      .vpc_id(vpc_id)
      .tag_specifications(
        EC2TagSpecification::builder()
          .resource_type(ResourceType::SecurityGroup)
          .tags(EC2Tag::builder().key("Name").value(name).build())
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
      .ec2_client
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

    // Add inbound rule for port 5080 from the same security group
    self
      .ec2_client
      .authorize_security_group_ingress()
      .group_id(&group_id)
      .ip_permissions(
        IpPermission::builder()
          .ip_protocol("tcp")
          .from_port(5080)
          .to_port(5080)
          .user_id_group_pairs(
            UserIdGroupPair::builder()
              .group_id(&group_id) // Reference to the same security group
              .description("Allow port 5080 access from instances in the same security group")
              .build(),
          )
          .build(),
      )
      .send()
      .await
      .with_context(|| format!("Failed to add port 5080 rule to security group '{}'", name))?;

    Ok(group_id)
  }

  // Helper method to get the default VPC ID
  async fn get_default_vpc_id(&self) -> Result<String> {
    let resp = self
      .ec2_client
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

    let ec2_client = EC2Client::new(&shared_config);
    let iam_client = IAMClient::new(&shared_config);

    Ok(AwsProvider {
      cluster_name,
      ec2_client,
      iam_client,
    })
  }

  async fn get_ip_address_by_name(&self, name: &str) -> Result<Option<(String, String)>> {
    // Get the list of Elastic IP addresses
    let resp = self
      .ec2_client
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

        return Ok(Some((public_ip, allocation_id)));
      }
      None => return Ok(None),
    };
  }

  async fn attach_ip_address_to_instance(&self, address: &str, instance_id: &str) -> Result<()> {
    // Associate the Elastic IP with the instance
    self
      .ec2_client
      .associate_address()
      .instance_id(instance_id)
      .allocation_id(address)
      .send()
      .await
      .with_context(|| {
        format!(
          "Failed to associate IP address {:?} with instance {:?}",
          address, instance_id
        )
      })?;

    Ok(())
  }

  async fn get_key_pair_by_name(&self, name: &str) -> Result<Option<String>> {
    // Get the list of key pairs with the Name tag
    let resp = self
      .ec2_client
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

  async fn import_public_key(&self, name: &str, public_key_path: &Path) -> Result<String> {
    // Read the public key file
    let public_key = tokio::fs::read(public_key_path)
      .await
      .with_context(|| format!("Failed to read public key file at {:?}", public_key_path))?;

    // Import the key pair to AWS with tag in a single API call
    let resp = self
      .ec2_client
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

    Ok(fingerprint)
  }

  async fn primary_ip_address(&self, name: &str) -> Result<(String, String)> {
    if let Some(address) = self.get_ip_address_by_name(name).await? {
      return Ok(address);
    }

    // Allocate a new Elastic IP address
    let resp = self
      .ec2_client
      .allocate_address()
      .domain(DomainType::Vpc)
      // Add the tag specification to tag the IP address during allocation
      .tag_specifications(
        EC2TagSpecification::builder()
          .resource_type(ResourceType::ElasticIp)
          .tags(EC2Tag::builder().key("Name").value(name).build())
          .build(),
      )
      .send()
      .await
      .with_context(|| format!("Failed to allocate new Elastic IP address for '{}'", name))?;

    let public_ip = match resp.public_ip() {
      Some(ip) => ip.to_string(),
      None => bail!(
        "No public IP returned from AWS after creating primary IP for '{}'",
        name
      ),
    };

    let allocation_id = match resp.allocation_id() {
      Some(id) => id.to_string(),
      None => bail!(
        "No allocation ID returned from AWS after creating primary IP for '{}'",
        name
      ),
    };

    Ok((public_ip, allocation_id))
  }

  async fn get_instance_by_name(&self, name: &str) -> Result<Option<InstanceInfo>> {
    // Get the list of EC2 instances with the given name
    let resp = self
      .ec2_client
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
    Ok(instances.into_iter().find(|instance| match instance.state {
      Some(InstanceState::Terminated) => false,
      _ => true,
    }))
  }

  async fn wait_for_instances(
    &self,
    instance_ids: &[String],
    timeout_seconds: u64,
    poll_interval_seconds: u64,
  ) -> Result<Vec<InstanceInfo>> {
    let start_time = tokio::time::Instant::now();
    let timeout = tokio::time::Duration::from_secs(timeout_seconds);
    let poll_interval = tokio::time::Duration::from_secs(poll_interval_seconds);

    // Track which instances are ready
    let mut ready_instances = Vec::with_capacity(instance_ids.len());
    let mut pending_instance_ids: Vec<String> = instance_ids.to_vec();

    loop {
      // Check if we've exceeded the timeout
      if start_time.elapsed() > timeout {
        bail!(
          "Timed out waiting for instances to become available with public IPs. Ready: {}, Pending: {}",
          ready_instances.len(),
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
        .ec2_client
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
        if matches!(instance.state, Some(InstanceState::Running)) && instance.public_ip.is_some() {
          ready_ids.push(instance.id.clone());
        } else {
          new_pending.push(instance.id.clone());
        }
      }

      // Now add any instances ready to to ready_instances
      for instance in instances {
        if ready_ids.contains(&instance.id) {
          // This will never fail since we only included IDs where public_ip.is_some()
          ready_instances.push(instance);
        }
      }

      // Update pending list
      pending_instance_ids = new_pending;
    }

    Ok(ready_instances)
  }

  // NOTE: This function signature doesn't allow more than 1 without a naming
  // convention. Name is being used to identify the primary instance here.
  async fn create_instances(
    &self,
    name: &str,
    image: &str,
    instance_type: &str,
    key_pair: &str,
    count: i64,
  ) -> Result<Vec<InstanceInfo>> {
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

    let instance_profile_name = self
      .instance_profile(name)
      .await
      .with_context(|| format!("Failed to get or create instance profile for '{}'", name))?;

    // Create EC2 instances
    let resp = self
      .ec2_client
      .run_instances()
      .image_id(image)
      .instance_type(InstanceType::from(instance_type))
      .min_count(count_i32)
      .max_count(count_i32)
      // Set the key pair for SSH access
      .key_name(key_pair)
      // Set the security group to allow SSH access
      .security_group_ids(security_group_id)
      // Set the IAM role for the instance
      .iam_instance_profile(
        IamInstanceProfileSpecification::builder()
          .name(instance_profile_name)
          .build(),
      )
      // Add a Name tag to identify these instances
      .tag_specifications(
        EC2TagSpecification::builder()
          .resource_type(ResourceType::Instance)
          .tags(EC2Tag::builder().key("Name").value(name).build())
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
        "No instances returned from AWS when creating instances for '{}'",
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
        "No valid instance IDs found in AWS response when creating instances for '{}'",
        name
      );
    }

    // Hard-coded timeout values
    // TODO: Should this stuff be configurable?
    let timeout_seconds = 300;
    let poll_interval_seconds = 5;

    // Wait for all instances to be running and have public IPs
    Ok(
      self
        .wait_for_instances(&instance_ids, timeout_seconds, poll_interval_seconds)
        .await
        .with_context(|| {
          format!(
            "Failed while waiting for instances to become available for '{}'",
            name
          )
        })?,
    )
  }
}
