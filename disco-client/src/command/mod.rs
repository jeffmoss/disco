mod bootstrap;

use async_trait::async_trait;
pub use bootstrap::*;
use disco_common::engine::EngineError;

// A Command trait that ensures we have a run() method on each struct:
#[async_trait]
pub trait Command {
  async fn run(&self) -> Result<(), EngineError>;
}
