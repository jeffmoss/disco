use rhai::plugin::*;
use rhai::{CustomType, TypeBuilder};

#[derive(Debug, Clone, CustomType)]
pub struct Host {
  #[rhai_type(readonly)]
  pub name: String,

  #[rhai_type(readonly)]
  pub id: String,

  #[rhai_type(readonly)]
  pub public_ip: String,
}

#[export_module]
pub mod host_module {
  use tracing::warn;

  pub type Host = super::Host;

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
