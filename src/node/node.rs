use std::sync::Arc;
use tracing::info;

use openraft::Config;
use openraft::ServerState;
use tokio::sync::Mutex;
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

use super::runtime;

pub type NodeId = u64;

pub struct Node {
  inner: Arc<NodeInner>, // Removed RwLock
}

pub struct NodeInner {
  node_id: NodeId,
  addr: String,
  raft: Raft,
  state_machine_store: Arc<StateMachineStore>,
  config: Arc<Config>,

  // controller is started and stopped based on raft leader status
  controller: Arc<Mutex<Option<Controller>>>,
}

impl Node {
  pub async fn new(node_id: NodeId, addr: String, config: Config) -> Node {
    let config = Arc::new(config);
    let log_store = LogStore::default();
    let state_machine_store = Arc::new(StateMachineStore::default());

    // Create the network layer
    let network = Network {};

    // Create a local raft instance
    let raft = Raft::new(
      node_id,
      config.clone(),
      network,
      log_store,
      state_machine_store.clone(),
    )
    .await
    .unwrap();

    let node_inner = NodeInner {
      node_id,
      addr,
      raft,
      state_machine_store,
      config,
      controller: Arc::new(Mutex::new(None)),
    };

    Node {
      inner: Arc::new(node_inner),
    }
  }

  pub async fn run(self) -> Result<(), Box<dyn std::error::Error>> {
    let inner_arc = self.inner.clone();

    // Spawn the leader election monitor
    runtime::spawn(Self::monitor_leader_election(inner_arc.clone()));

    // Now we can directly use the inner fields without any locking
    info!(
      "Node {} starting server at {}",
      inner_arc.node_id, inner_arc.addr
    );

    // Create the services
    let internal_service = RaftServiceImpl::new(inner_arc.raft.clone());
    let api_service = AppServiceImpl::new(
      inner_arc.raft.clone(),
      inner_arc.state_machine_store.clone(),
    );

    // Start and await the server
    Server::builder()
      .add_service(RaftServiceServer::new(internal_service))
      .add_service(AppServiceServer::new(api_service))
      .serve(inner_arc.addr.parse()?)
      .await?;

    Ok(())
  }

  async fn monitor_leader_election(inner_arc: Arc<NodeInner>) {
    info!("Monitoring leader election");

    // Get metrics directly
    let mut metrics = inner_arc.raft.server_metrics();

    let mut current_state: Option<ServerState> = None;

    loop {
      if let Err(err) = metrics.changed().await {
        info!(
          "{}; when:(watching metrics); quit monitor_leader_election() loop",
          err
        );
        break;
      }

      let mm = metrics.borrow().clone();

      // Only act if state has changed
      if current_state == Some(mm.state) {
        continue;
      }

      current_state = Some(mm.state);

      match current_state {
        Some(ServerState::Leader) => {
          info!("Node {} is the leader", mm.id);

          // Only lock the controller when we need to modify it
          NodeInner::start_controller(&inner_arc.controller).await;
        }
        Some(ServerState::Follower) => {
          info!("Node {} is a follower", mm.id);
        }
        _ => {
          info!("Node {} is a candidate", mm.id);
        }
      }
    }
  }
}

impl NodeInner {
  pub async fn start_controller(controller: &Arc<Mutex<Option<Controller>>>) {
    let mut controller_guard = controller.lock().await;
    if controller_guard.is_none() {
      *controller_guard = Some(Controller::new(10));
      info!("Started controller");

      // We need to drop the guard to avoid deadlock when running the command
      let controller_ref = controller_guard.as_ref().unwrap();
      for i in 1..=20 {
        if let Err(e) = controller_ref
          .run_command(format!("sleep 5; echo 'Hello world {}'", i))
          .await
        {
          info!("Failed to run command {}: {:?}", i, e);
        }
      }
    }
  }

  pub async fn stop_controller(controller: &Arc<Mutex<Option<Controller>>>) {
    let mut controller_guard = controller.lock().await;
    if let Some(controller_ref) = controller_guard.take() {
      drop(controller_guard); // Release the lock before the potentially long-running stop

      if let Err(e) = controller_ref.stop().await {
        info!("Failed to stop controller: {:?}", e);
      }
    }
  }
}
