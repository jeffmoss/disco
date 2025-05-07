use rhai::plugin::*;
use std::io::{self, Write};

#[export_module]
pub mod utils_module {
  pub fn ask(prompt: String) -> bool {
    let mut input = String::new();
    print!("{} (y/n): ", prompt);
    io::stdout().flush().unwrap();

    io::stdin().read_line(&mut input).unwrap();
    matches!(input.trim().to_lowercase().as_str(), "y")
  }
}
