use std::sync::Arc;
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tracing::info;

use openraft::Config;
use tonic::transport::Server;

use crate::controller::Controller;
use crate::grpc::app_service::AppServiceImpl;
use crate::grpc::raft_service::RaftServiceImpl;
use crate::network::Network;
use crate::protobuf::app_service_server::AppServiceServer;
use crate::protobuf::raft_service_server::RaftServiceServer;
use crate::raft_types::Raft;
use crate::store::LogStore;
use crate::store::StateMachineStore;
use crate::Command;

use super::runtime;

pub type NodeId = u64;

#[derive(Clone)]
pub struct Node {
  inner: Arc<NodeInner>,
}

impl Node {
  pub async fn new(node_id: NodeId, addr: String, config: Config) -> Node {
    let config = Arc::new(config);
    let (controller_tx, controller_rx) = channel::<Command>(10);
    let log_store = LogStore::default();
    let state_machine_store = Arc::new(StateMachineStore::default());

    // Create the network layer that will connect and communicate the raft instances and
    // will be used in conjunction with the store created above.
    let network = Network {};

    // Create a local raft instance.
    let raft = Raft::new(
      node_id,
      config.clone(),
      network,
      log_store,
      state_machine_store.clone(),
    )
    .await
    .unwrap();

    Controller::new(controller_rx);

    let node_inner = NodeInner::new(
      controller_tx,
      node_id,
      addr,
      raft,
      state_machine_store,
      config,
    );

    Node {
      inner: Arc::new(node_inner),
    }
  }

  /// Return the config of this Raft node.
  pub async fn start(&self) -> Result<(), Box<dyn std::error::Error>> {
    self.inner.start().await
  }
}

pub struct NodeInner {
  controller_tx: Sender<Command>,
  node_id: NodeId,
  addr: String,
  raft: Raft,
  state_machine_store: Arc<StateMachineStore>,
  config: Arc<Config>,
}

impl NodeInner {
  pub fn new(
    controller_tx: Sender<Command>,
    node_id: NodeId,
    addr: String,
    raft: Raft,
    state_machine_store: Arc<StateMachineStore>,
    config: Arc<Config>,
  ) -> NodeInner {
    NodeInner {
      controller_tx,
      node_id,
      addr,
      raft,
      state_machine_store,
      config,
    }
  }

  pub async fn start(&self) -> Result<(), Box<dyn std::error::Error>> {
    self.monitor_leader_election(self.raft.clone()).await;

    // Create the management service with raft instance
    let internal_service = RaftServiceImpl::new(self.raft.clone());
    let api_service = AppServiceImpl::new(self.raft.clone(), self.state_machine_store.clone());

    // Start server
    let server_future = Server::builder()
      .add_service(RaftServiceServer::new(internal_service))
      .add_service(AppServiceServer::new(api_service))
      .serve(self.addr.parse()?);

    info!("Node {} starting server at {}", self.node_id, self.addr);
    server_future.await?;

    Ok(())
  }

  async fn monitor_leader_election(&self, raft: Raft) {
    let controller_tx = self.controller_tx.clone();
    // raft.metrics() includes heartbeat notifications. server_metrics() is a subset that does not.
    let mut metrics = raft.server_metrics();

    let fut = async move {
      let mut leader_id: Option<NodeId> = None;

      loop {
        let changed = metrics.changed().await;

        info!("Changed");
        if let Err(changed_err) = changed {
          // Shutting down.
          info!(
            "{}; when:(watching metrics); quit monitor_leader_election() loop",
            changed_err
          );
          break;
        }

        let mm = metrics.borrow().clone();

        if leader_id == mm.current_leader {
          continue;
        }

        leader_id = mm.current_leader;

        if leader_id == Some(mm.id) {
          controller_tx.send(Command::StartController).await.unwrap();
        } else {
          controller_tx.send(Command::StopController).await.unwrap();
        }
      }
    };

    runtime::spawn(fut);
  }
}
