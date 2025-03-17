use crate::Command;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

pub struct Controller {
  inner: Arc<Mutex<ControllerInner>>,
  handle: tokio::task::JoinHandle<()>,
}

/// The controller manages the entire cluster and starts and stops based on the node election status.
/// The raft state manager is used to track the state for any necessary changes.
impl Controller {
  pub fn new(mut rx_api: tokio::sync::mpsc::Receiver<Command>) -> Controller {
    // Create a new controller to move into the task
    let inner = Arc::new(Mutex::new(ControllerInner::new()));
    let inner_clone = inner.clone();

    let handle = tokio::spawn(async move {
      while let Some(command) = rx_api.recv().await {
        match command {
          Command::StartController => {
            info!("Starting controller");
            let mut ctrl = inner_clone.lock().await;
            ctrl.start_controller().await;
          }
          Command::StopController => {
            info!("Stopping controller");
            let mut ctrl = inner_clone.lock().await;
            ctrl.stop_controller().await;
          }
        }
      }
    });

    Controller { inner, handle }
  }
}

// Inner controller implementation with instance methods
struct ControllerInner {
  running: bool,
  // Other state
}

impl ControllerInner {
  fn new() -> Self {
    ControllerInner {
      running: false,
      // Initialize other state
    }
  }

  async fn start_controller(&mut self) {
    self.running = true;
    // Do actual controller start logic
  }

  async fn stop_controller(&mut self) {
    self.running = false;
    // Do actual controller stop logic
  }
}
