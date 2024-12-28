use std::sync::Arc;
use tracing::info;

use openraft::Config;
use tonic::transport::Server;

use crate::network::Network;
use crate::store::new_storage;
use crate::grpc::api_service::ApiServiceImpl;
use crate::grpc::internal_service::InternalServiceImpl;
use crate::grpc::management_service::ManagementServiceImpl;
use crate::protobuf::api_service_server::ApiServiceServer;
use crate::protobuf::internal_service_server::InternalServiceServer;
use crate::protobuf::management_service_server::ManagementServiceServer;

pub type NodeId = u64;

pub struct NodeService {
    node_id: NodeId,
    addr: String,
    config: Arc<Config>,
}

impl NodeService {
  pub fn new(
    node_id: NodeId,
    addr: String,
    config: Config,
  ) -> NodeService {
    NodeService {
      node_id,
      addr,
      config: Arc::new(config)
    }
  }

  pub async fn start(&self) -> Result<(), Box<dyn std::error::Error>> {
    let dir = format!("{}.db", self.addr);

    let (log_store, state_machine_store) = new_storage(&dir).await;
    let key_values = state_machine_store.data.kvs.clone();

    // Create the network layer that will connect and communicate the raft instances and
    // will be used in conjunction with the store created above.
    let network = Network {};

    // Create a local raft instance.
    let raft = openraft::Raft::new(
      self.node_id,
      self.config.clone(),
      network,
      log_store,
      state_machine_store
    ).await.unwrap();

    // Create the management service with raft instance
    let management_service = ManagementServiceImpl::new(raft.clone());
    let internal_service = InternalServiceImpl::new(raft.clone());
    let api_service = ApiServiceImpl::new(raft, key_values);

    // Start server
    let server_future = Server::builder()
        .add_service(ManagementServiceServer::new(management_service))
        .add_service(InternalServiceServer::new(internal_service))
        .add_service(ApiServiceServer::new(api_service))
        .serve(self.addr.parse()?);

    info!("Node {} starting server at {}", self.node_id, self.addr);
    server_future.await?;

    Ok(())
  }
}
