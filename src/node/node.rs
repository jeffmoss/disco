use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::info;

use openraft::{Config, Raft, ServerState};
use tonic::transport::Server;

use crate::grpc::app_service::AppServiceImpl;
use crate::grpc::raft_service::RaftServiceImpl;
use crate::network::Network;
use crate::protobuf::app_service_server::AppServiceServer;
use crate::protobuf::raft_service_server::RaftServiceServer;
use crate::store::LogStore;
use crate::store::StateMachineStore;
use crate::TypeConfig;

use super::runtime;

pub type NodeId = u64;

pub struct NodeService {
  node_id: NodeId,
  addr: String,
  config: Arc<Config>,
}

impl NodeService {
  pub fn new(node_id: NodeId, addr: String, config: Config) -> NodeService {
    NodeService {
      node_id,
      addr,
      config: Arc::new(config),
    }
  }

  pub async fn start(&self) -> Result<(), Box<dyn std::error::Error>> {
    let log_store = LogStore::default();
    let state_machine_store = Arc::new(StateMachineStore::default());

    // Create the network layer that will connect and communicate the raft instances and
    // will be used in conjunction with the store created above.
    let network = Network {};

    // Create a local raft instance.
    let raft = Raft::new(
      self.node_id,
      self.config.clone(),
      network,
      log_store,
      state_machine_store.clone(),
    )
    .await
    .unwrap();

    let raft_arc = Arc::new(raft);
    self.subscribe_metrics(raft_arc.clone()).await;

    // Create the management service with raft instance
    let internal_service = RaftServiceImpl::new(raft_arc.clone());
    let api_service = AppServiceImpl::new(raft_arc.clone(), state_machine_store);

    // Start server
    let server_future = Server::builder()
      .add_service(RaftServiceServer::new(internal_service))
      .add_service(AppServiceServer::new(api_service))
      .serve(self.addr.parse()?);

    info!("Node {} starting server at {}", self.node_id, self.addr);
    server_future.await?;

    Ok(())
  }

  async fn subscribe_metrics(&self, raft: Arc<Raft<TypeConfig>>) {
    let metrics = raft.metrics();
    let node_id = self.node_id.clone();

    let fut = async move {
      loop {
        let changed = raft.wait(None).current_leader(node_id, "leader").await;

        if let Err(changed_err) = changed {
          // Shutting down.
          info!(
            "{}; when:(watching metrics); quit subscribe_metrics() loop",
            changed_err
          );
          break;
        }

        let mm = metrics.borrow().clone();

        match mm.state {
          ServerState::Leader => {
            info!("Node {} is the leader", node_id);
          }
          ServerState::Follower => {
            info!("Node {} is a follower", node_id);
          }
          ServerState::Candidate => {
            info!("Node {} is a candidate", node_id);
          }
          ServerState::Shutdown => {
            info!("Node {} is shutting down", node_id);
          }
          ServerState::Learner => {
            info!("Node {} is a learner", node_id);
          }
        }

        sleep(Duration::from_millis(5000)).await;
      }
    };

    runtime::spawn(fut);
  }
}
