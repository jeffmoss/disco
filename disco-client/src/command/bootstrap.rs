use super::Command;
use async_trait::async_trait;
use disco_common::engine::*;
use tracing::info;

pub struct Bootstrap {
  engine: Engine,
}

impl Bootstrap {
  pub fn new(engine: Engine) -> Self {
    Self { engine }
  }
}

#[async_trait]
impl Command for Bootstrap {
  async fn run(&self) -> Result<(), EngineError> {
    info!("Bootstrapping Disco cluster...");

    let cluster = self.engine.init().await?;

    info!("Cluster initialized: {:?}", cluster);

    let _ = self.engine.callback("bootstrap", &[cluster]).await?;

    Ok(())
  }
}
