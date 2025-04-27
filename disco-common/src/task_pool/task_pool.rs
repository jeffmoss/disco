use std::sync::Arc;
use tokio::sync::oneshot;
use tokio::sync::{
  mpsc::{channel, Receiver, Sender},
  OwnedSemaphorePermit, Semaphore,
};
use tokio::task::JoinHandle;
use tracing::info;

use crate::action::{Actor, ActorResponse};

pub struct TaskPool {
  sender: Sender<Box<dyn Actor>>,
  task_handle: JoinHandle<()>,
}

impl TaskPool {
  pub fn new(max_concurrent_tasks: usize) -> TaskPool {
    let (sender, receiver) = channel::<Box<dyn Actor>>(100);
    let semaphore = Arc::new(Semaphore::new(max_concurrent_tasks));

    let task_handle = {
      let semaphore = semaphore.clone();
      tokio::spawn(process_receiver(receiver, semaphore))
    };

    TaskPool {
      sender,
      task_handle,
    }
  }

  pub async fn stop(self) -> Result<(), tokio::task::JoinError> {
    drop(self.sender);
    self.task_handle.await
  }

  pub async fn send_actor(
    &self,
    actor: Box<dyn Actor>,
  ) -> Result<(), tokio::sync::mpsc::error::SendError<Box<dyn Actor>>> {
    self.sender.send(actor).await
  }
}

async fn process_receiver(mut receiver: Receiver<Box<dyn Actor>>, semaphore: Arc<Semaphore>) {
  while let Some(actor) = receiver.recv().await {
    let permit = semaphore.clone().acquire_owned().await.unwrap();
    tokio::spawn(process_actor(actor, permit));
  }
}

// Standalone function to run an actor
pub async fn run_actor(actor: Box<dyn Actor>) -> Result<ActorResponse, oneshot::error::RecvError> {
  let (tx, rx) = oneshot::channel();
  actor.process(tx);
  rx.await
}

pub async fn process_actor(actor: Box<dyn Actor>, _permit: OwnedSemaphorePermit) {
  if let Ok(result) = run_actor(actor).await {
    match &result {
      ActorResponse::CommandResult(cmd) => {
        info!(
          "Command executed with status: {}, stdout: {}, stderr: {}",
          cmd.status, cmd.stdout, cmd.stderr
        );
      }
      ActorResponse::Boolean(val) => {
        info!("Boolean result: {}", val);
      }
      _ => (),
    }
  }
}
