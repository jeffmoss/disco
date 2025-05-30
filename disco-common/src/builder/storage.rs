use crate::provider::Provider;
use anyhow::{Context, Result};
use boa_engine::JsData;
use boa_gc::{Finalize, Trace};
use std::sync::Arc;

#[derive(Debug)]
struct StorageInner {
  name: String,
  role: String,
  provider: Arc<dyn Provider>,
}

#[derive(Clone, Debug)]
#[cfg_attr(feature = "js", derive(Trace, Finalize, JsData))]
pub struct Storage {
  #[unsafe_ignore_trace]
  inner: Arc<StorageInner>,
}

impl Storage {
  pub fn new(name: String, role: String, provider: impl Provider + 'static) -> Self {
    Self {
      inner: Arc::new(StorageInner {
        name,
        role,
        provider: Arc::new(provider),
      }),
    }
  }

  pub fn name(&self) -> &str {
    &self.inner.name
  }

  pub async fn ensure(&self) -> Result<()> {
    // Ensure the storage bucket exists
    self
      .inner
      .provider
      .create_storage(&self.inner.name, &self.inner.role)
      .await
      .with_context(|| {
        format!(
          "Failed to create storage with name '{}' and role '{}'",
          &self.inner.name, &self.inner.role
        )
      })?;

    Ok(())
  }

  pub async fn upload(&self, file: &str, key: &str) -> Result<()> {
    self
      .inner
      .provider
      .upload_file_to_storage(&self.inner.name, file, key)
      .await
      .with_context(|| {
        format!(
          "Failed to upload file '{}' to storage with key '{}'",
          file, key
        )
      })?;

    Ok(())
  }
}
