mod start;

pub use start::Start;

// A Command trait that ensures we have a run() method on each struct:
pub trait Command {
  fn run(&self);
}
