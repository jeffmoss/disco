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
      inactivity_timeout: Some(Duration::from_secs(5)),
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
  pub async fn run_command(&self, command: &str) -> Result<u32> {
    let mut channel = self.session.channel_open_session().await?;
    channel.exec(true, command).await?;

    let mut code = None;
    let mut stdout = tokio::io::stdout();

    loop {
      // There's an event available on the session channel
      let Some(msg) = channel.wait().await else {
        break;
      };

      match msg {
        // Write data to the terminal
        ChannelMsg::Data { ref data } => {
          stdout.write_all(data).await?;
          stdout.flush().await?;
        }
        // The command has returned an exit code
        ChannelMsg::ExitStatus { exit_status } => {
          code = Some(exit_status);
          // cannot leave the loop immediately, there might still be more data to receive
        }
        _ => {}
      }
    }

    match code {
      Some(exit_code) => Ok(exit_code),
      None => bail!("Program did not exit cleanly"),
    }
  }

  // Method specifically for running commands with input
  pub async fn run_command_with_input<R>(&self, command: &str, mut input: R) -> Result<u32>
  where
    R: AsyncReadExt + Unpin,
  {
    let mut channel = self.session.channel_open_session().await?;
    channel.exec(true, command).await?;

    let mut code = None;
    let mut stdout = tokio::io::stdout();
    let mut stderr = tokio::io::stderr();

    let mut stdin_closed = false;

    // Buffer for reading from the stdin source
    let mut buf = vec![0; 8192]; // 8KB buffer for performance

    loop {
      tokio::select! {
          // Read from input and send to the remote process
          r = input.read(&mut buf), if !stdin_closed => {
              match r {
                  Ok(0) => {
                      // End of input stream, send EOF
                      stdin_closed = true;
                      channel.eof().await?;
                  },
                  Ok(n) => {
                      // Send data to the remote process
                      channel.data(&buf[..n]).await?;
                  },
                  Err(e) => return Err(e.into()),
              }
          },

          // There's an event available on the session channel
          Some(msg) = channel.wait() => {
              match msg {
                  // Write data to the terminal
                  ChannelMsg::Data { ref data } => {
                      stdout.write_all(data).await?;
                      stdout.flush().await?;
                  },
                                  // Write extended data to stderr
                ChannelMsg::ExtendedData { ref data, ext } => {
                  // ext == 1 is stderr in the SSH protocol
                  if ext == 1 {
                      stderr.write_all(data).await?;
                      stderr.flush().await?;
                  }
              },
                  // The command has returned an exit code
                  ChannelMsg::ExitStatus { exit_status } => {
                      code = Some(exit_status);
                      if !stdin_closed {
                          channel.eof().await?;
                          stdin_closed = true;
                      }
                  },
                  _ => {}
              }

              // If we have an exit code and stdin is closed, we can exit
              if code.is_some() && stdin_closed {
                  break;
              }
          },

          // No more events and stdin is still open
          else => {
              if !stdin_closed {
                  channel.eof().await?;
                  stdin_closed = true;
              } else {
                  break;
              }
          }
      }
    }

    match code {
      Some(exit_code) => Ok(exit_code),
      None => bail!("Program did not exit cleanly"),
    }
  }

  async fn close(&mut self) -> Result<()> {
    self
      .session
      .disconnect(Disconnect::ByApplication, "", "English")
      .await?;
    Ok(())
  }
}
