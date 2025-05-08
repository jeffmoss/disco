use super::client::Client;
use anyhow::{bail, Result};
use russh::{client, keys::*, ChannelMsg, Disconnect, Preferred};
use std::{borrow::Cow, path::Path, sync::Arc, time::Duration};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::ToSocketAddrs;

// Define a helper enum for stdin sources
pub enum StdinSource<'a> {
  None,
  Reader(Box<dyn AsyncRead + Unpin + 'a>),
}

impl<'a> StdinSource<'a> {
  pub fn from_reader<R>(reader: R) -> Self
  where
    R: AsyncRead + Unpin + 'a,
  {
    Self::Reader(Box::new(reader))
  }
}

pub struct Session {
  session: client::Handle<Client>,
}

impl Session {
  pub async fn connect<P: AsRef<Path>, A: ToSocketAddrs>(
    key_path: P,
    user: impl Into<String>,
    openssh_cert_path: Option<P>,
    addrs: A,
  ) -> Result<Self> {
    let key_pair = load_secret_key(key_path, None)?;

    // load ssh certificate
    let mut openssh_cert = None;
    if openssh_cert_path.is_some() {
      openssh_cert = Some(load_openssh_certificate(openssh_cert_path.unwrap())?);
    }

    let config = client::Config {
      inactivity_timeout: Some(Duration::from_secs(600)),
      preferred: Preferred {
        kex: Cow::Owned(vec![
          russh::kex::CURVE25519_PRE_RFC_8731,
          russh::kex::EXTENSION_SUPPORT_AS_CLIENT,
        ]),
        ..Default::default()
      },
      ..<_>::default()
    };

    let config = Arc::new(config);
    let sh = Client {};

    let mut session = client::connect(config, addrs, sh).await?;
    // use publickey authentication, with or without certificate
    if openssh_cert.is_none() {
      let auth_res = session
        .authenticate_publickey(
          user,
          PrivateKeyWithHashAlg::new(
            Arc::new(key_pair),
            session.best_supported_rsa_hash().await?.flatten(),
          ),
        )
        .await?;

      if !auth_res.success() {
        anyhow::bail!("Authentication (with publickey) failed");
      }
    } else {
      let auth_res = session
        .authenticate_openssh_cert(user, Arc::new(key_pair), openssh_cert.unwrap())
        .await?;

      if !auth_res.success() {
        anyhow::bail!("Authentication (with publickey+cert) failed");
      }
    }

    Ok(Self { session })
  }

  // Method for running commands without input
  pub async fn run_command<S>(&self, command: S) -> Result<u32>
  where
    S: Into<Vec<u8>>,
  {
    let mut channel = self.session.channel_open_session().await?;
    channel.exec(true, command).await?;

    // Process the channel events
    self.process_channel_events(&mut channel).await
  }

  // Method specifically for running commands with input
  pub async fn run_command_with_input<S, R>(&self, command: S, input: R) -> Result<u32>
  where
    S: Into<Vec<u8>>,
    R: AsyncRead + Unpin,
  {
    let mut channel = self.session.channel_open_session().await?;
    channel.exec(true, command).await?;

    // Send input data directly to the channel
    channel.data(input).await?;

    // Signal EOF after input is fully sent
    channel.eof().await?;

    // Process the channel events
    self.process_channel_events(&mut channel).await
  }

  // Private helper method to process channel events and get the exit code
  async fn process_channel_events(
    &self,
    channel: &mut russh::Channel<russh::client::Msg>,
  ) -> Result<u32> {
    let mut code = None;
    let mut stdout = tokio::io::stdout();
    let mut stderr = tokio::io::stderr();

    // Wait for channel events
    loop {
      let Some(msg) = channel.wait().await else {
        break;
      };

      match msg {
        // Write data to the terminal (stdout)
        ChannelMsg::Data { ref data } => {
          stdout.write_all(data).await?;
          stdout.flush().await?;
        }
        // Write extended data to stderr
        ChannelMsg::ExtendedData { ref data, ext } => {
          // ext == 1 is stderr in the SSH protocol
          if ext == 1 {
            stderr.write_all(data).await?;
            stderr.flush().await?;
          }
        }
        // The command has returned an exit code
        ChannelMsg::ExitStatus { exit_status } => {
          code = Some(exit_status);
          // cannot leave the loop immediately in the first method,
          // but we can in the second method. For consistency, we'll
          // continue in both cases until there are no more messages.
        }
        _ => {}
      }
    }

    match code {
      Some(exit_code) => Ok(exit_code),
      None => bail!("Program did not exit cleanly"),
    }
  }

  pub async fn close(&self) -> Result<()> {
    self
      .session
      .disconnect(Disconnect::ByApplication, "", "English")
      .await?;
    Ok(())
  }
}
