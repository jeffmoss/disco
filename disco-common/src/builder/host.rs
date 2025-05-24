use crate::provider::InstanceInfo;

#[derive(Debug, Clone)]
pub struct Host {
  pub name: String,

  pub id: String,

  pub public_ip: String,
}

impl TryFrom<InstanceInfo> for Host {
  type Error = String;

  fn try_from(instance: InstanceInfo) -> Result<Self, Self::Error> {
    // Check for required name
    let name = instance
      .name
      .ok_or_else(|| format!("Instance '{}' does not have a name tag", instance.id))?;

    // Check for required public IP
    let public_ip = instance.public_ip.ok_or_else(|| {
      format!(
        "Instance '{}' ({}) does not have a public IP address",
        name, instance.id
      )
    })?;

    Ok(Host {
      name,
      id: instance.id,
      public_ip,
    })
  }
}
