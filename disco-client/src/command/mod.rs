mod bootstrap;

pub use bootstrap::*;

// A Command trait that ensures we have a run() method on each struct:
pub trait Command {
  fn run(&self) -> Result<(), String>;
}
