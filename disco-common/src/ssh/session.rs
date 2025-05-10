use super::client::Client;
use russh::keys::{load_openssh_certificate, load_secret_key, PrivateKeyWithHashAlg};
use russh::{client, ChannelMsg, Disconnect, Preferred};
use std::{borrow::Cow, path::Path, sync::Arc, time::Duration};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
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
  ) -> Result<Self, Box<dyn std::error::Error>> {
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
        return Err("Authentication (with publickey) failed".into());
      }
    } else {
      let auth_res = session
        .authenticate_openssh_cert(user, Arc::new(key_pair), openssh_cert.unwrap())
        .await?;

      if !auth_res.success() {
        return Err("Authentication (with publickey+cert) failed".into());
      }
    }

    Ok(Self { session })
  }

  // Method for running commands without input
  pub async fn run_command<S>(
    &self,
    command: S,
  ) -> Result<u32, Box<dyn std::error::Error + Send + Sync>>
  where
    S: Into<Vec<u8>>,
  {
    let channel = self.session.channel_open_session().await?;

    channel.exec(true, command).await?;

    // Get a reader for the channel
    let (mut reader, _) = channel.split();

    let mut stdout = tokio::io::stdout();
    let mut stderr = tokio::io::stderr();

    // Process the channel events
    self
      .process_channel_events(&mut reader, &mut stdout, &mut stderr)
      .await
  }

  // Method specifically for running commands with input
  pub async fn run_command_with_input<S, R>(
    &self,
    command: S,
    input: R,
  ) -> Result<u32, Box<dyn std::error::Error + Send + Sync>>
  where
    S: Into<Vec<u8>>,
    R: AsyncRead + Unpin,
  {
    let channel = self.session.channel_open_session().await?;

    channel.exec(true, command).await?;

    // Get a reader for the channel
    let (mut reader, writer) = channel.split();

    let mut stdout = tokio::io::stdout();
    let mut stderr = tokio::io::stderr();

    // Sending needs to be performed asynchronously alongside receiving channel events
    // in order to prevent the server buffer from filling up and causing a deadlock.
    let (_, status) = tokio::try_join!(
      async {
        writer.data(input).await?;

        // TODO: A progress indicator here would be nice

        writer.eof().await.map_err(|e| e.into())
      },
      self.process_channel_events(&mut reader, &mut stdout, &mut stderr)
    )?;

    Ok(status)
  }

  // New method for running commands and capturing output
  pub async fn run_command_with_output<S>(
    &self,
    command: S,
  ) -> Result<String, Box<dyn std::error::Error + Send + Sync>>
  where
    S: Into<Vec<u8>>,
  {
    let channel = self.session.channel_open_session().await?;

    channel.exec(true, command).await?;

    // Get a reader for the channel
    let (mut reader, _) = channel.split();

    // Create buffer to capture output
    let mut stdout_buffer = Vec::new();
    let mut stderr = tokio::io::stderr();

    // Process the channel events and capture output to buffers
    let mut stdout_writer = tokio::io::BufWriter::new(&mut stdout_buffer);

    let exit_code = self
      .process_channel_events(&mut reader, &mut stdout_writer, &mut stderr)
      .await?;

    // Make sure all data is flushed to the buffers
    stdout_writer.flush().await?;

    // Convert buffers to strings
    let stdout_str = String::from_utf8(stdout_buffer)?;

    // If exit code is not 0, return an error with the output
    if exit_code != 0 {
      return Err(format!("Command failed with exit code {}", exit_code).into());
    }

    Ok(stdout_str)
  }

  // Method for running commands and capturing a single line of output
  pub async fn run_command_with_output_line<S>(
    &self,
    command: S,
  ) -> Result<String, Box<dyn std::error::Error + Send + Sync>>
  where
    S: Into<Vec<u8>>,
  {
    // Run the command and get the output
    let output = self.run_command_with_output(command).await?;

    // Split by newlines to count the lines
    let lines: Vec<&str> = output.lines().collect();

    if lines.len() != 1 {
      return Err(
        format!(
          "Expected exactly one line of output, but got {} lines: {}",
          lines.len(),
          output
        )
        .into(),
      );
    }

    // Return the single line of output (without any trailing newline)
    Ok(lines[0].to_string())
  }

  // Modified helper method to process channel events and get the exit code
  async fn process_channel_events<O, E>(
    &self,
    channel: &mut russh::ChannelReadHalf,
    stdout: &mut O,
    stderr: &mut E,
  ) -> Result<u32, Box<dyn std::error::Error + Send + Sync>>
  where
    O: AsyncWrite + Unpin,
    E: AsyncWrite + Unpin,
  {
    let mut code = None;

    // Wait for channel events
    loop {
      let Some(msg) = channel.wait().await else {
        break;
      };

      match msg {
        // Write data to stdout
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
        }
        _ => {}
      }
    }

    match code {
      Some(exit_code) => Ok(exit_code),
      None => Err("Program did not exit cleanly".into()),
    }
  }

  pub async fn close(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    self
      .session
      .disconnect(Disconnect::ByApplication, "", "English")
      .await?;
    Ok(())
  }
}
