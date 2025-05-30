use tokio::sync::oneshot;

pub use oneshot::Sender;

/// The actors can be implemented as various types that perform unique tasks, but they
/// all must conform to a definitive set of responses.

/// Define a unified Response enum that can handle all possible response types
#[derive(Debug)]
pub enum ActorResponse {
  Empty,
  Boolean(bool),
  CommandResult(CommandResult),
  // Probably not a good idea to use this...
  Custom(Box<dyn std::any::Any + Send>), // Fallback for custom types
}

/// Command result structure
/// stderr and stdout could become very large. Consider defining a custom BashCommand that captures output differently.
#[derive(Debug)]
pub struct CommandResult {
  pub stdout: String,
  pub stderr: String,
  pub status: i32,
}

/// Base trait for all actor types
pub trait Actor: Send + 'static {
  fn process(self: Box<Self>, respond_to: oneshot::Sender<ActorResponse>);
}
