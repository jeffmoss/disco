use super::Session;
use crate::builder::{Host, KeyPair};
use anyhow::{bail, Result};
use std::{
  env,
  fmt::Display,
  fs::{self},
  path::PathBuf,
  process::Stdio,
  sync::{Arc, Mutex},
  time::{SystemTime, UNIX_EPOCH},
};
use tokio::{fs::File as TokioFile, io::BufReader, process::Command};
use tracing::info;

pub struct Installer {
  key_pair: KeyPair,
  username: String,
  remote_directory: String,
  certificate: Option<PathBuf>,
  tar_file: Mutex<Option<PathBuf>>,
}

impl Installer {
  pub fn new<U>(key_pair: KeyPair, username: U, certificate: Option<PathBuf>) -> Arc<Self>
  where
    U: Into<String>,
  {
    let username = username.into();
    let remote_directory = format!("/home/{}/disco", username);

    Arc::new(Self {
      key_pair,
      username,
      remote_directory,
      certificate,
      tar_file: Mutex::new(None),
    })
  }

  pub async fn install_to_host(&self, host: &Host) -> Result<()> {
    // Connect to the host
    let session = self.connect_to_host(host).await?;

    // Ensure the target directory exists
    self.ensure_remote_directory(&session).await?;

    // Stream the cached tar to remote
    self.stream_tar_to_remote(&session).await?;

    session.close().await?;

    Ok(())
  }

  async fn connect_to_host(&self, host: &Host) -> Result<Session> {
    let session = Session::connect(
      &self.key_pair.private_key,
      &self.username,
      self.certificate.as_ref(),
      (host.public_ip.as_ref(), 22),
    )
    .await
    .map_err(|e| anyhow::anyhow!("Failed to connect to host: {}", e))?;

    info!("SSH session established with host: {:?}", host);
    Ok(session)
  }

  async fn ensure_remote_directory(&self, session: &Session) -> Result<()> {
    let exit_status = session
      .run_command(format!("mkdir -p {}", self.remote_directory))
      .await?;

    if exit_status != 0 {
      bail!(
        "Failed to create target directory, exit status: {}",
        exit_status
      );
    }

    Ok(())
  }

  async fn stream_tar_to_remote(&self, session: &Session) -> Result<()> {
    // Get or create the cached tar file
    let tar_path = self.get_or_create_tar_file().await?;

    // Open the cached tar file for reading
    let tar_file = TokioFile::open(tar_path).await?;
    let reader = BufReader::with_capacity(256 * 1024, tar_file);

    // Stream to remote tar extraction command
    let exit_status = session
      .run_command_with_input(format!("tar -xzf - -C {}", self.remote_directory), reader)
      .await?;

    if exit_status != 0 {
      bail!(
        "Remote tar extraction failed with exit status: {}",
        exit_status
      );
    }

    Ok(())
  }

  // TODO: this could cache the tarball on the remote host for when scaling a cluster
  async fn get_or_create_tar_file(&self) -> Result<PathBuf> {
    // First check if we already have a path
    {
      let guard = self.tar_file.lock().unwrap();
      if let Some(path) = &*guard {
        if path.exists() {
          return Ok(path.clone());
        }
      }
    }

    // Create a new path
    let timestamp = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .unwrap_or_default()
      .as_nanos();

    let pid = std::process::id();
    let unique_name = format!("disco_{}_{}.tar.gz", timestamp, pid);
    let tar_path = env::temp_dir().join(unique_name);

    // Create the tar file
    self.create_tar_file(&tar_path).await?;

    // Store the path
    {
      let mut guard = self.tar_file.lock().unwrap();
      *guard = Some(tar_path.clone());
    }

    Ok(tar_path)
  }

  async fn create_tar_file(&self, tar_path: &PathBuf) -> Result<()> {
    // Create a command to tar the current directory
    let mut tar_cmd = Command::new("tar");
    tar_cmd
      .args(&["-chzf", tar_path.to_str().unwrap(), "."])
      .stdout(Stdio::null());

    let status = tar_cmd
      .status()
      .await
      .map_err(|e| anyhow::anyhow!("Failed to run tar command: {}", e))?;

    if !status.success() {
      bail!("Tar command failed with exit code: {:?}", status.code());
    }

    info!("Created tar archive at: {:?}", tar_path);
    Ok(())
  }

  // Clean up the temporary tar file when the installer is no longer needed
  pub fn cleanup(&self) -> Result<()> {
    let guard = self.tar_file.lock().unwrap();
    if let Some(tar_path) = &*guard {
      if tar_path.exists() {
        fs::remove_file(tar_path)
          .map_err(|e| anyhow::anyhow!("Failed to remove temporary tar file: {}", e))?;
        info!("Removed temporary tar file: {:?}", tar_path);
      }
    }
    Ok(())
  }
}

impl Drop for Installer {
  fn drop(&mut self) {
    if let Ok(guard) = self.tar_file.lock() {
      if let Some(tar_path) = &*guard {
        let _ = fs::remove_file(tar_path);
      }
    }
  }
}
