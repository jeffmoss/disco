use disco_common::engine::*;
use std::sync::Arc;
use tokio::fs;
use tokio::try_join;
use tracing::info;

use openraft::{Config, ServerState, metrics::RaftServerMetrics};
use tokio::sync::{Mutex, watch::Receiver};
use tonic::transport::{Certificate, Identity, Server, ServerTlsConfig};

use crate::TypeConfig;
use crate::config::Opt;
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

struct NodeInner {
  // Store the entire config
  config: Opt,

  // Keep the fields you were using directly
  raft: Raft,
  state_machine_store: Arc<StateMachineStore>,

  // cluster-wide settings that never change
  #[allow(dead_code)]
  settings: Settings,

  // each node runs a disco Engine for scripted customizations
  engine: Engine,

  // controller is started and stopped based on raft leader status
  controller: Arc<Mutex<Option<Controller>>>,

  // TLS certificates
  server_cert: Vec<u8>,
  server_key: Vec<u8>,
  ca_cert: Vec<u8>,
  client_cert: Vec<u8>,
  client_key: Vec<u8>,
}

impl Node {
  const START_FILE: &str = "cluster.js";

  pub async fn new(config: Opt, settings: Settings) -> Result<Node, Box<dyn std::error::Error>> {
    // Load all TLS certificates in parallel at startup
    let (server_cert, server_key, ca_cert, client_cert, client_key) = try_join!(
      fs::read(&config.server_cert),
      fs::read(&config.server_key),
      fs::read(&config.ca_cert),
      fs::read(&config.client_cert),
      fs::read(&config.client_key)
    )?;

    let log_store = LogStore::default();
    let state_machine_store = Arc::new(StateMachineStore::default());

    // Create the network layer with client certificates
    let network = Network::new(&ca_cert, &client_cert, &client_key)?;

    let raft_config: Config = Config {
      cluster_name: settings.cluster_name.clone(),
      election_timeout_min: settings.election_timeout_min,
      election_timeout_max: settings.election_timeout_max,
      heartbeat_interval: settings.heartbeat_interval,
      install_snapshot_timeout: settings.install_snapshot_timeout,
      ..Default::default()
    }
    .validate()?; // Proper error handling instead of unwrap

    // Create a local raft instance
    let raft = Raft::new(
      config.id.clone(),
      Arc::new(raft_config),
      network,
      log_store,
      state_machine_store.clone(),
    )
    .await?; // Proper error handling

    let engine = Engine::new(Some(Self::START_FILE))?;

    let _cluster = engine.callback("init", &[]).await?;

    let node_inner = NodeInner {
      config,
      raft,
      state_machine_store,
      settings,
      engine,
      controller: Arc::new(Mutex::new(None)),

      // Store the loaded certificates
      server_cert,
      server_key,
      ca_cert,
      client_cert,
      client_key,
    };

    Ok(Node {
      inner: Arc::new(node_inner),
    })
  }

  pub async fn run(&self) -> Result<(), Box<dyn std::error::Error>> {
    // Spawn the leader election monitor - just clone what you need
    runtime::spawn(Self::monitor_leader_election(
      self.inner.raft.server_metrics(),
      self.inner.controller.clone(),
      self.inner.clone(),
    ));

    info!(
      "Node {} starting server at {}",
      self.inner.config.id, self.inner.config.addr
    );

    rustls::crypto::aws_lc_rs::default_provider()
      .install_default()
      .expect("Failed to install crypto provider");

    let server_identity = Identity::from_pem(&self.inner.server_cert, &self.inner.server_key);
    let ca_certificate = Certificate::from_pem(&self.inner.ca_cert);

    // Configure TLS
    let tls_config = ServerTlsConfig::new()
      .identity(server_identity)
      .client_ca_root(ca_certificate);

    // Create the services
    let internal_service = RaftServiceImpl::new(self.inner.raft.clone());
    let api_service = AppServiceImpl::new(
      self.inner.raft.clone(),
      self.inner.state_machine_store.clone(),
    );

    // Start and await the server with TLS
    Server::builder()
      .tls_config(tls_config)?
      .add_service(protobuf::raft_service_server::RaftServiceServer::new(
        internal_service,
      ))
      .add_service(protobuf::app_service_server::AppServiceServer::new(
        api_service,
      ))
      .serve(self.inner.config.addr.parse()?)
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
