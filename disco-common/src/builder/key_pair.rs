use rhai::{CustomType, TypeBuilder};

#[derive(Debug, Clone, CustomType)]
pub struct KeyPair {
  #[rhai_type(readonly)]
  pub name: String,

  #[rhai_type(readonly)]
  pub private_key: std::path::PathBuf,

  #[rhai_type(readonly)]
  pub fingerprint: String,
}
