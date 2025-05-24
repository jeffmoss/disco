use disco_common::engine::*;
use std::sync::Arc;
use tracing::info;

use openraft::{Config, ServerState, metrics::RaftServerMetrics};
use tokio::sync::{Mutex, watch::Receiver};
use tonic::transport::Server;

use crate::TypeConfig;
use crate::controller::Controller;
use crate::grpc::app_service::AppServiceImpl;
use crate::grpc::raft_service::RaftServiceImpl;
use crate::network::Network;
use crate::protobuf;
use crate::raft_types::Raft;
use crate::settings::Settings;
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

  // cluster-wide settings that never change
  settings: Settings,

  // each node runs a disco Engine for scripted customizations
  engine: Engine,

  // controller is started and stopped based on raft leader status
  controller: Arc<Mutex<Option<Controller>>>,
}

impl Node {
  pub async fn new(node_id: NodeId, addr: String, settings: Settings) -> Node {
    let log_store = LogStore::default();
    let state_machine_store = Arc::new(StateMachineStore::default());

    // Create the network layer
    let network = Network {};

    let config: Config = Config {
      cluster_name: settings.cluster_name.clone(),
      election_timeout_min: settings.election_timeout_min,
      election_timeout_max: settings.election_timeout_max,
      heartbeat_interval: settings.heartbeat_interval,
      install_snapshot_timeout: settings.install_snapshot_timeout,
      ..Default::default()
    }
    .validate()
    .unwrap(); // Handle the Result by unwrapping or use proper error handling

    // Create a local raft instance
    let raft = Raft::new(
      node_id,
      Arc::new(config),
      network,
      log_store,
      state_machine_store.clone(),
    )
    .await
    .unwrap();

    let engine = Engine::new("cluster.js").unwrap();

    let _cluster = engine.callback("init", &[]).await.unwrap();

    let node_inner = NodeInner {
      node_id,
      addr,
      raft,
      state_machine_store,
      settings,
      engine,
      controller: Arc::new(Mutex::new(None)),
    };

    Node {
      inner: Arc::new(node_inner),
    }
  }

  pub async fn run(self) -> Result<(), Box<dyn std::error::Error>> {
    let inner_arc = self.inner.clone();

    // Spawn the leader election monitor
    let metrics = inner_arc.raft.server_metrics();
    let controller = inner_arc.controller.clone();

    runtime::spawn(Self::monitor_leader_election(
      metrics, controller, inner_arc,
    ));

    // Now we can directly use the inner fields without any locking
    info!(
      "Node {} starting server at {}",
      self.inner.node_id, self.inner.addr
    );

    // Create the services
    let internal_service = RaftServiceImpl::new(self.inner.raft.clone());
    let api_service = AppServiceImpl::new(
      self.inner.raft.clone(),
      self.inner.state_machine_store.clone(),
    );

    // Start and await the server
    Server::builder()
      .add_service(protobuf::raft_service_server::RaftServiceServer::new(
        internal_service,
      ))
      .add_service(protobuf::app_service_server::AppServiceServer::new(
        api_service,
      ))
      .serve(self.inner.addr.parse()?)
      .await?;

    Ok(())
  }

  async fn monitor_leader_election(
    mut metrics: Receiver<RaftServerMetrics<TypeConfig>>,
    controller: Arc<Mutex<Option<Controller>>>,
    node_inner: Arc<NodeInner>,
  ) {
    info!("Monitoring leader election");

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
          NodeInner::start_controller(&controller).await;

          node_inner
            .engine
            .callback("leader", &[mm.id.into()])
            .await
            .unwrap();

          // Note: No engine interaction here, as we don't have access to it
          // Any engine interaction would need to happen elsewhere
        }
        Some(ServerState::Follower) => {
          info!("Node {} is a follower", mm.id);
        }
        _ => {
          // info!("Node {} is a something", mm.id);
        }
      }
    }
  }
}

impl NodeInner {
  pub async fn start_controller(controller: &Arc<Mutex<Option<Controller>>>) {
    let mut controller_guard = controller.lock().await;
    if controller_guard.is_none() {
      *controller_guard = Some(Controller::new());
      info!("Started controller");
    }
  }

  pub async fn stop_controller(controller: &Arc<Mutex<Option<Controller>>>) {
    let mut controller_guard = controller.lock().await;
    if let Some(_controller_ref) = controller_guard.take() {
      drop(controller_guard); // Release the lock before the potentially long-running stop

      // if let Err(e) = controller_ref.stop().await {
      //   info!("Failed to stop controller: {:?}", e);
      // }
    }
  }
}
