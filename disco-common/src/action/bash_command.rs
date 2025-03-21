use super::actor::{Actor, ActorResponse, CommandResult, Sender};

/// Run a bash command and capture its output
///
pub struct BashCommand {
  command: String,
}

impl BashCommand {
  pub fn new(command: String) -> Box<Self> {
    Box::new(Self { command })
  }
}

impl Actor for BashCommand {
  fn process(self: Box<Self>, respond_to: Sender<ActorResponse>) {
    // Execute the command
    let output = std::process::Command::new("bash")
      .arg("-c")
      .arg(&self.command)
      .output()
      .expect("failed to execute process");

    // Create the result
    let result = CommandResult {
      stdout: String::from_utf8_lossy(&output.stdout).to_string(),
      stderr: String::from_utf8_lossy(&output.stderr).to_string(),
      status: output.status.code().unwrap_or(-1),
    };

    // Send the result
    let _ = respond_to.send(ActorResponse::CommandResult(result));
  }
}
