use disco_common::engine::*;
use std::sync::Arc;
use tracing::info;

use openraft::{metrics::RaftServerMetrics, Config, ServerState};
use tokio::sync::{watch::Receiver, Mutex};
use tonic::transport::Server;

use crate::controller::Controller;
use crate::grpc::app_service::AppServiceImpl;
use crate::grpc::raft_service::RaftServiceImpl;
use crate::network::Network;
use crate::protobuf;
use crate::raft_types::Raft;
use crate::settings::Settings;
use crate::store::LogStore;
use crate::store::StateMachineStore;
use crate::TypeConfig;

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

    let engine = Engine::new("node.rhai").unwrap();

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
    let max_concurrent_tasks = inner_arc.settings.external_commands_max;

    runtime::spawn(Self::monitor_leader_election(
      metrics,
      controller,
      max_concurrent_tasks,
    ));

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
      .add_service(protobuf::raft_service_server::RaftServiceServer::new(
        internal_service,
      ))
      .add_service(protobuf::app_service_server::AppServiceServer::new(
        api_service,
      ))
      .serve(inner_arc.addr.parse()?)
      .await?;

    Ok(())
  }

  async fn monitor_leader_election(
    mut metrics: Receiver<RaftServerMetrics<TypeConfig>>,
    controller: Arc<Mutex<Option<Controller>>>,
    max_concurrent_tasks: usize,
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
          NodeInner::start_controller(&controller, max_concurrent_tasks).await;

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
  pub async fn start_controller(
    controller: &Arc<Mutex<Option<Controller>>>,
    max_concurrent_tasks: usize,
  ) {
    let mut controller_guard = controller.lock().await;
    if controller_guard.is_none() {
      *controller_guard = Some(Controller::new(max_concurrent_tasks));
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
