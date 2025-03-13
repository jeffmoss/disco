use std::time::Duration;

use crate::protobuf::app_service_client::AppServiceClient;
use crate::protobuf::{GetRequest, SetRequest};
use tonic::{transport::Channel, Request, Status};

pub struct RaftClient {
  channel: Channel,
}

impl RaftClient {
  pub async fn new(addr: String) -> Result<Self, Box<dyn std::error::Error>> {
    let channel = Channel::from_shared(addr.clone())?
      .timeout(Duration::from_secs(5))
      .connect()
      .await?;

    Ok(Self { channel })
  }

  pub async fn get_value(&self, key: String) -> Result<Option<String>, Status> {
    // Create a client using the channel
    let mut client = AppServiceClient::new(self.channel.clone());

    // Create the GetRequest message
    let request = Request::new(GetRequest { key });

    // Make the RPC call
    let response = client.get(request).await?;
    let result = response.into_inner();

    // Return the response inner data
    Ok(result.value)
  }

  pub async fn set_value(
    &self,
    key: String,
    value: String,
  ) -> Result<Option<String>, tonic::Status> {
    // Create a client using the channel
    let mut client = AppServiceClient::new(self.channel.clone());

    // Create the SetRequest message
    let request = Request::new(SetRequest { key, value });

    // Make the RPC call
    let response = client.set(request).await?;
    let result = response.into_inner();

    // Return the response inner data (success flag)
    Ok(result.value)
  }
}
