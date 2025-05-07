use disco_common::action::{Actor, BashCommand};
use disco_common::engine::Engine;
use disco_common::task_pool::TaskPool;
use tracing::info;

use super::Command;

pub struct Start {
  task_pool: TaskPool,
  engine: Engine,
}

impl Start {
  pub fn new<S: Into<String>>(max_concurrent_tasks: usize, filename: S) -> Self {
    Self {
      task_pool: TaskPool::new(max_concurrent_tasks),
      engine: Engine::new(filename).unwrap(),
    }
  }
}

impl Command for Start {
  fn run(&self) {
    info!("Bootstrapping Disco cluster...");
    self.engine.start();
  }
}
