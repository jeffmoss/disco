use disco_common::action::{Actor, BashCommand};
use disco_common::engine::*;
use disco_common::task_pool::TaskPool;

pub struct Controller {
  task_pool: TaskPool,
}

impl Controller {
  pub fn new(max_concurrent_tasks: usize) -> Controller {
    Controller {
      task_pool: TaskPool::new(max_concurrent_tasks),
    }
  }

  pub async fn stop(self) -> Result<(), tokio::task::JoinError> {
    self.task_pool.stop().await
  }

  pub async fn send_actor(
    &self,
    actor: Box<dyn Actor>,
  ) -> Result<(), tokio::sync::mpsc::error::SendError<Box<dyn Actor>>> {
    self.task_pool.send_actor(actor).await
  }

  pub async fn run_command(
    &self,
    command: String,
  ) -> Result<(), tokio::sync::mpsc::error::SendError<Box<dyn Actor>>> {
    self.send_actor(BashCommand::new(command)).await
  }
}
