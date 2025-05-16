use anyhow::{anyhow, Context, Result};
use base64ct::{Base64, Encoding};
use russh::keys::{HashAlg, PublicKey};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct KeyPair {
  pub name: String,

  pub private_key: std::path::PathBuf,

  pub fingerprint: String,
}

impl KeyPair {
  /// Computes the SHA-256 fingerprint for a public key in AWS format (Base64 encoded)
  pub async fn compute_fingerprint(public_key_path: &Path) -> Result<String> {
    // Read the public key file
    let public_key_data = tokio::fs::read_to_string(public_key_path)
      .await
      .with_context(|| format!("Failed to read public key file at {:?}", public_key_path))?;

    // Parse the public key using russh
    let public_key = PublicKey::from_openssh(&public_key_data)
      .map_err(|e| anyhow!("Failed to parse public key: {}", e))?;

    // Get the fingerprint bytes
    let fingerprint_data = public_key.fingerprint(HashAlg::default());
    let fingerprint_bytes = fingerprint_data.as_bytes();

    // Encode to Base64 with padding
    let fingerprint = Base64::encode_string(fingerprint_bytes);

    Ok(fingerprint)
  }

  /// Compares this key pair's fingerprint with the fingerprint of a local public key
  /// Using &str is more flexible than &String as it can accept both String and &str
  pub async fn fingerprint_matches_local_public_key(
    fingerprint: &str,
    public_key_path: &Path,
  ) -> Result<bool> {
    let local_fingerprint = Self::compute_fingerprint(public_key_path).await?;
    Ok(fingerprint == local_fingerprint)
  }
}
