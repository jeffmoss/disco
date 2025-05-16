use disco_common::engine::*;
use disco_common::task_pool::TaskPool;
use tracing::info;

use super::Command;

pub struct Bootstrap {
  task_pool: TaskPool,
  engine: Engine,
}

impl Bootstrap {
  pub fn new(max_concurrent_tasks: usize, engine: Engine) -> Self {
    Self {
      task_pool: TaskPool::new(max_concurrent_tasks),
      engine,
    }
  }
}

impl Command for Bootstrap {
  fn run(&self) -> Result<(), String> {
    info!("Bootstrapping Disco cluster...");

    let _ = self.engine.callback("bootstrap", &[])?;

    Ok(())
  }
}
