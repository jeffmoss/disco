use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use openraft::AnyError;
use openraft::RaftNetworkFactory;
use openraft::error::NetworkError;
use openraft::error::Unreachable;
use openraft::network::RPCOption;
use openraft::network::v2::RaftNetworkV2;
use tonic::codegen::tokio_stream::wrappers::ReceiverStream;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint, Identity};

use crate::NodeId;
use crate::TypeConfig;
use crate::protobuf;
use crate::raft_types::*;

/// Network implementation for gRPC-based Raft communication.
/// Provides the networking layer for Raft nodes to communicate with each other.
pub struct Network {
  // TLS configuration
  tls_config: ClientTlsConfig,
}

impl Network {
  pub fn new(
    ca_cert: &[u8],
    client_cert: &[u8],
    client_key: &[u8],
  ) -> Result<Self, Box<dyn std::error::Error>> {
    // Load certificates
    let ca = Certificate::from_pem(ca_cert);
    let identity = Identity::from_pem(client_cert, client_key);

    // Configure mTLS
    let tls_config = ClientTlsConfig::new()
      .ca_certificate(ca)
      .identity(identity)
      .domain_name("localhost"); // Adjust to match your server certificate

    Ok(Network { tls_config })
  }
}

/// Implementation of the RaftNetworkFactory trait for creating new network connections.
/// This factory creates gRPC client connections to other Raft nodes.
impl RaftNetworkFactory<TypeConfig> for Network {
  type Network = NetworkConnection;

  #[tracing::instrument(level = "debug", skip_all)]
  async fn new_client(&mut self, _: NodeId, node: &Node) -> Self::Network {
    let server_addr = &node.rpc_addr;

    // Build the endpoint step by step
    let endpoint_result = Endpoint::from_shared(format!("https://{}", server_addr))
      .and_then(|ep| ep.tls_config(self.tls_config.clone()));

    let channel = match endpoint_result {
      Ok(endpoint) => {
        match endpoint
          .tcp_keepalive(Some(std::time::Duration::from_secs(30)))
          .http2_keep_alive_interval(std::time::Duration::from_secs(30))
          .keep_alive_timeout(std::time::Duration::from_secs(5))
          .connect_timeout(std::time::Duration::from_secs(10))
          .connect()
          .await
        {
          Ok(channel) => Some(channel),
          Err(e) => {
            tracing::error!("Failed to connect to {}: {}", server_addr, e);
            None
          }
        }
      }
      Err(e) => {
        tracing::error!("Failed to configure TLS for {}: {}", server_addr, e);
        None
      }
    };

    NetworkConnection::new(channel)
  }
}

/// Represents an active network connection to a remote Raft node.
/// Handles serialization and deserialization of Raft messages over gRPC.
pub struct NetworkConnection {
  // Pre-established channel, or None if connection failed
  channel: Option<Channel>,
  // Cached client created from the channel
  client: Option<protobuf::raft_service_client::RaftServiceClient<Channel>>,
}

impl NetworkConnection {
  /// Creates a new NetworkConnection with a pre-established channel
  pub fn new(channel: Option<Channel>) -> Self {
    NetworkConnection {
      channel,
      client: None,
    }
  }

  /// Get or create the gRPC client from the established channel
  fn get_client(
    &mut self,
  ) -> Result<&mut protobuf::raft_service_client::RaftServiceClient<Channel>, RPCError> {
    // If we don't have a channel, connection failed during creation
    let channel = self.channel.as_ref().ok_or_else(|| {
      RPCError::Unreachable(Unreachable::new(&std::io::Error::new(
        std::io::ErrorKind::NotConnected,
        "No connection available",
      )))
    })?;

    // Create client if we don't have one yet
    if self.client.is_none() {
      self.client = Some(protobuf::raft_service_client::RaftServiceClient::new(
        channel.clone(),
      ));
    }

    Ok(self.client.as_mut().unwrap())
  }
}

/// Implementation of RaftNetwork trait for handling Raft protocol communications.
#[allow(clippy::blocks_in_conditions)]
impl RaftNetworkV2<TypeConfig> for NetworkConnection {
  async fn append_entries(
    &mut self,
    req: AppendEntriesRequest,
    _option: RPCOption,
  ) -> Result<AppendEntriesResponse, RPCError> {
    let client = self.get_client()?;

    let response = client
      .append_entries(protobuf::AppendEntriesRequest::from(req))
      .await
      .map_err(|e| RPCError::Network(NetworkError::new(&e)))?;

    Ok(AppendEntriesResponse::from(response.into_inner()))
  }

  async fn full_snapshot(
    &mut self,
    vote: Vote,
    snapshot: Snapshot,
    _cancel: impl std::future::Future<Output = openraft::error::ReplicationClosed>
    + openraft::OptionalSend
    + 'static,
    _option: RPCOption,
  ) -> Result<SnapshotResponse, crate::raft_types::StreamingError> {
    let client = self.get_client().map_err(|e| match e {
      RPCError::Unreachable(u) => StreamingError::from(u),
      RPCError::Network(n) => StreamingError::from(n),
      _ => StreamingError::from(NetworkError::new(&AnyError::error("Connection error"))),
    })?;

    let (tx, rx) = tokio::sync::mpsc::channel(1024);
    let strm = ReceiverStream::new(rx);

    // Start the RPC call but don't await it yet
    let response_future = client.snapshot(strm);

    // 1. Send meta chunk
    let meta = &snapshot.meta;
    let request = protobuf::SnapshotRequest {
      payload: Some(protobuf::snapshot_request::Payload::Meta(
        protobuf::SnapshotRequestMeta {
          vote: Some(vote),
          last_log_id: meta.last_log_id.map(|log_id| log_id.into()),
          last_membership_log_id: meta.last_membership.log_id().map(|log_id| log_id.into()),
          last_membership: Some(meta.last_membership.membership().clone().into()),
          snapshot_id: meta.snapshot_id.to_string(),
        },
      )),
    };

    tx.send(request).await.map_err(|e| NetworkError::new(&e))?;

    // 2. Send data chunks
    let chunk_size = 1024 * 1024;
    for chunk in snapshot.snapshot.chunks(chunk_size) {
      let request = protobuf::SnapshotRequest {
        payload: Some(protobuf::snapshot_request::Payload::Chunk(chunk.to_vec())),
      };
      tx.send(request).await.map_err(|e| NetworkError::new(&e))?;
    }

    // 3. Close the stream by dropping the sender
    drop(tx);

    // 4. Now await the response
    let response = response_future.await.map_err(|e| NetworkError::new(&e))?;

    let message = response.into_inner();

    Ok(SnapshotResponse {
      vote: message.vote.ok_or_else(|| {
        NetworkError::new(&AnyError::error("Missing `vote` in snapshot response"))
      })?,
    })
  }

  async fn vote(&mut self, req: VoteRequest, _option: RPCOption) -> Result<VoteResponse, RPCError> {
    let client = self.get_client()?;

    // Convert the openraft VoteRequest to protobuf VoteRequest
    let proto_vote_req: protobuf::VoteRequest = req.into();

    // Create a tonic Request with the protobuf VoteRequest
    let request = tonic::Request::new(proto_vote_req);

    // Send the vote request
    let response = client
      .vote(request)
      .await
      .map_err(|e| RPCError::Network(NetworkError::new(&e)))?;

    // Convert the response back to openraft VoteResponse
    let proto_vote_resp: protobuf::VoteResponse = response.into_inner();
    Ok(proto_vote_resp.into())
  }
}
