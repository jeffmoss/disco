use crate::provider::{InstanceInfo, InstanceState, Provider};
use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use aws_config;
use aws_sdk_ec2::types::{
  DomainType, Filter, IamInstanceProfileSpecification, InstanceStateName, InstanceType,
  IpPermission, IpRange, ResourceType, UserIdGroupPair,
};
use aws_sdk_iam;
use aws_sdk_s3;
use boa_engine::JsData;
use boa_gc::{Finalize, Trace};
use core::panic;
use serde_json::json;
use std::path::Path;

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
  pub ec2_client: aws_sdk_ec2::Client,

  #[unsafe_ignore_trace]
  pub iam_client: aws_sdk_iam::Client,

  #[unsafe_ignore_trace]
  pub s3_client: aws_sdk_s3::Client,
}

impl AwsProvider {
  fn instances_from_response(
    &self,
    resp: &aws_sdk_ec2::operation::describe_instances::DescribeInstancesOutput,
  ) -> Result<Vec<InstanceInfo>> {
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
  async fn iam_role(&self, role_name: &str) -> Result<()> {
    // Check if role exists first
    let role_exists = match self.iam_client.get_role().role_name(role_name).send().await {
      Ok(_) => true,
      Err(aws_sdk_iam::error::SdkError::ServiceError(service_error)) => {
        if matches!(
          service_error.err(),
          aws_sdk_iam::operation::get_role::GetRoleError::NoSuchEntityException(_)
        ) {
          false
        } else {
          return Err(service_error.into_err().into());
        }
      }
      Err(e) => return Err(e.into()),
    };

    let assume_role_policy = json!({
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
    });

    // Create the IAM role only if it doesn't exist
    if !role_exists {
      self
        .iam_client
        .create_role()
        .role_name(role_name)
        .tags(
          aws_sdk_iam::types::Tag::builder()
            .key("Name")
            .value(role_name)
            .build()?,
        )
        .assume_role_policy_document(assume_role_policy.to_string())
        .max_session_duration(3600) // 1 hour
        .description(format!("Role for {} EC2 instances", role_name))
        .send()
        .await
        .with_context(|| format!("Failed to create role '{}'", role_name))?;
    }

    // Instance policy document for the role
    let policy_document = json!({
       "Version": "2012-10-17",
       "Statement": [
           {
               "Effect": "Allow",
               "Action": [
                   "ssm:GetParameter",
                   "ssm:GetParameters",
                   "ec2:DescribeInstances",
                   "route53:ChangeResourceRecordSets"
               ],
               "Resource": "*"
           },
           {
               "Effect": "Allow",
               "Action": [
                   "ec2:RunInstances",
                   "ec2:DescribeImages",
                   "ec2:DescribeInstanceTypes",
                   "ec2:DescribeKeyPairs",
                   "ec2:DescribeSecurityGroups",
                   "ec2:DescribeSubnets",
                   "ec2:DescribeVpcs",
                   "ec2:CreateTags",
                   "ec2:TerminateInstances",
                   "ec2:StopInstances",
                   "ec2:AssociateAddress",
                   "ec2:DisassociateAddress",
                   "ec2:DescribeAddresses"
               ],
               "Resource": "*"
           },
           {
               "Effect": "Allow",
               "Action": "iam:PassRole",
               "Resource": format!("arn:aws:iam::*:role/{}", role_name)
           }
       ]
    });

    let policy_name = format!("{}-policy", role_name);

    self
      .iam_client
      .put_role_policy()
      .role_name(role_name)
      .policy_name(&policy_name)
      .policy_document(policy_document.to_string())
      .send()
      .await
      .with_context(|| format!("Failed to attach policy for role '{}'", role_name))?;

    Ok(())
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
        aws_sdk_ec2::types::TagSpecification::builder()
          .resource_type(ResourceType::SecurityGroup)
          .tags(
            aws_sdk_ec2::types::Tag::builder()
              .key("Name")
              .value(name)
              .build(),
          )
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
    let shared_config = aws_config::defaults(aws_config::BehaviorVersion::v2025_01_17())
      .region(aws_sdk_ec2::config::Region::new(region))
      .load()
      .await;

    let ec2_client = aws_sdk_ec2::Client::new(&shared_config);
    let iam_client = aws_sdk_iam::Client::new(&shared_config);
    let s3_client = aws_sdk_s3::Client::new(&shared_config);

    Ok(AwsProvider {
      cluster_name,
      ec2_client,
      iam_client,
      s3_client,
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
      .public_key_material(aws_sdk_ec2::primitives::Blob::new(public_key))
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
        aws_sdk_ec2::types::TagSpecification::builder()
          .resource_type(ResourceType::ElasticIp)
          .tags(
            aws_sdk_ec2::types::Tag::builder()
              .key("Name")
              .value(name)
              .build(),
          )
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

  /// Create an IAM instance profile with role and policies
  async fn instance_profile(&self, role_name: &str, profile_name: &str) -> Result<()> {
    // Return early if instance profile already exists
    let (profile_exists, has_role) = match self
      .iam_client
      .get_instance_profile()
      .instance_profile_name(profile_name)
      .send()
      .await
    {
      Ok(response) => {
        let profile = response
          .instance_profile()
          .ok_or_else(|| anyhow::anyhow!("Instance profile exists but has no data"))?;

        // Check if the profile already has the role with the same name
        let has_role = profile
          .roles()
          .iter()
          .any(|role| role.role_name() == role_name);

        (true, has_role)
      }
      Err(aws_sdk_iam::error::SdkError::ServiceError(service_error)) => {
        if matches!(
          service_error.err(),
          aws_sdk_iam::operation::get_instance_profile::GetInstanceProfileError::NoSuchEntityException(_)
        ) {
          (false, false)
        } else {
          return Err(service_error.into_err().into())
        }
      }
      Err(e) => return Err(e.into())
    };

    if !profile_exists {
      // Create the instance profile
      self
        .iam_client
        .create_instance_profile()
        .instance_profile_name(profile_name)
        .send()
        .await?;
    }

    if !has_role {
      self.iam_role(role_name).await?;

      // Add the role to the instance profile
      self
        .iam_client
        .add_role_to_instance_profile()
        .instance_profile_name(profile_name)
        .role_name(role_name)
        .send()
        .await?;
    }

    Ok(())
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

    self.instance_profile(name, name).await?;

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
          .name(name)
          .build(),
      )
      // Add a Name tag to identify these instances
      .tag_specifications(
        aws_sdk_ec2::types::TagSpecification::builder()
          .resource_type(ResourceType::Instance)
          .tags(
            aws_sdk_ec2::types::Tag::builder()
              .key("Name")
              .value(name)
              .build(),
          )
          .build(),
      )
      .send()
      .await
      .with_context(|| {
        format!(
          "Failed to create {} EC2 instances with image '{}' and type '{}'",
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

  async fn create_storage(&self, bucket_name: &str, iam_role_arn: &str) -> Result<()> {
    // Create the bucket
    self
      .s3_client
      .create_bucket()
      .bucket(bucket_name)
      .send()
      .await
      .with_context(|| format!("Failed to create '{}' S3 bucket", bucket_name))?;

    // Block all public access
    self
      .s3_client
      .put_public_access_block()
      .bucket(bucket_name)
      .public_access_block_configuration(
        aws_sdk_s3::types::PublicAccessBlockConfiguration::builder()
          .block_public_acls(true)
          .ignore_public_acls(true)
          .block_public_policy(true)
          .restrict_public_buckets(true)
          .build(),
      )
      .send()
      .await
      .with_context(|| {
        format!(
          "Failed to set public access block for '{}' bucket",
          bucket_name
        )
      })?;

    // Set bucket policy to only allow account owner and specific IAM role
    let bucket_policy = format!(
      r#"{{
  "Version": "2012-10-17",
  "Statement": [
    {{
      "Sid": "DenyAllExceptOwnerAndRole",
      "Effect": "Deny",
      "Principal": "*",
      "Action": "s3:*",
      "Resource": [
        "arn:aws:s3:::{bucket_name}",
        "arn:aws:s3:::{bucket_name}/*"
      ],
      "Condition": {{
        "StringNotEquals": {{
          "aws:PrincipalArn": [
            "{iam_role_arn}"
          ]
        }},
        "Bool": {{
          "aws:PrincipalIsAWSService": "false"
        }}
      }}
    }},
    {{
      "Sid": "DenyListBucket",
      "Effect": "Deny",
      "Principal": "*",
      "Action": "s3:ListBucket",
      "Resource": "arn:aws:s3:::{bucket_name}",
      "Condition": {{
        "StringNotEquals": {{
          "aws:PrincipalArn": [
            "{iam_role_arn}"
          ]
        }}
      }}
    }}
  ]
}}"#,
      bucket_name = bucket_name,
      iam_role_arn = iam_role_arn
    );

    self
      .s3_client
      .put_bucket_policy()
      .bucket(bucket_name)
      .policy(bucket_policy)
      .send()
      .await
      .with_context(|| format!("Failed to set bucket policy for '{}'", bucket_name))?;

    Ok(())
  }

  async fn upload_file_to_storage(
    &self,
    storage_name: &str,
    file_path: &str,
    key: &str,
  ) -> Result<()> {
    let file_path = Path::new(file_path);
    let body = aws_sdk_s3::primitives::ByteStream::from_path(file_path)
      .await
      .with_context(|| format!("Failed to read file at {}", file_path.display()))?;

    self
      .s3_client
      .put_object()
      .bucket(storage_name)
      .key(key)
      .body(body)
      .send()
      .await
      .with_context(|| {
        format!(
          "Failed to upload file '{}' to storage '{}'",
          file_path.display(),
          storage_name
        )
      })?;

    Ok(())
  }

  async fn download_file_from_storage(
    &self,
    bucket_name: &str,
    file_path: &str,
    key: &str,
  ) -> Result<()> {
    use std::path::Path;
    use tokio::fs::File;
    use tokio::io::AsyncWriteExt;

    let response = self
      .s3_client
      .get_object()
      .bucket(bucket_name)
      .key(key)
      .send()
      .await
      .with_context(|| {
        format!(
          "Failed to get object '{}' from bucket '{}'",
          key, bucket_name
        )
      })?;

    let local_path = Path::new(file_path);

    let mut file = File::create(local_path)
      .await
      .with_context(|| format!("Failed to create local file at {}", local_path.display()))?;

    let mut body = response.body;
    while let Some(chunk) = body
      .try_next()
      .await
      .with_context(|| format!("Failed to read body from S3 response for key '{}'", key))?
    {
      file.write_all(&chunk).await.with_context(|| {
        format!(
          "Failed to write chunk to local file at {}",
          local_path.display()
        )
      })?;
    }

    file
      .flush()
      .await
      .with_context(|| format!("Failed to flush file at {}", local_path.display()))?;

    Ok(())
  }
}
