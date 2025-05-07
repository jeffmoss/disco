use russh::{client, keys::*, ChannelId};
use tracing::info;

pub struct Client {}

impl client::Handler for Client {
  type Error = anyhow::Error;

  async fn check_server_key(
    &mut self,
    server_public_key: &ssh_key::PublicKey,
  ) -> Result<bool, Self::Error> {
    info!("check_server_key: {:?}", server_public_key);
    Ok(true)
  }

  async fn data(
    &mut self,
    channel: ChannelId,
    data: &[u8],
    _session: &mut client::Session,
  ) -> Result<(), Self::Error> {
    info!("data on channel {:?}: {}", channel, data.len());
    Ok(())
  }
}
