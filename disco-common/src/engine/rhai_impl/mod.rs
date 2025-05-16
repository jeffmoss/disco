use std::path::Path;
use std::sync::Mutex;

mod plugin;
use plugin::{cluster_module, host_module, utils_module};

use rhai::{self, Dynamic, FuncArgs, Scope};
use rhai::{exported_module, EvalAltResult, Position};
use tracing::{info, warn};

use rhai::packages::Package;
#[cfg(feature = "fs-access")]
use rhai_fs::FilesystemPackage;

use super::EngineInterface;

pub struct Engine {
  script_path: String,
  script_contents: String,
  engine: rhai::Engine,
  scope: Mutex<rhai::Scope<'static>>,
  ast: rhai::AST,
}

impl EngineInterface<Dynamic> for Engine {
  fn new(filename: &str) -> Result<Self, String> {
    let engine = Self::configure_engine();

    // Load the script file
    let (script_path, script_contents) = Self::load_script(filename)?;

    // Run the loaded script with improved error handling using map_err
    let mut ast = engine.compile(script_contents.clone()).map_err(|err| {
      Self::print_script_error(&script_path, &script_contents, err.clone().into());
      format!("Failed to compile script: {err}")
    })?;

    // Set the source on the AST
    ast.set_source(script_path.clone());

    let scope = Scope::new();

    Ok(Self {
      script_path,
      script_contents,
      engine,
      scope: Mutex::new(scope),
      ast,
    })
  }

  fn callback(&self, name: &str, args: &[Dynamic]) -> Result<Dynamic, String> {
    let mut scope = self.scope.lock().unwrap();

    info!("About to run {}()", name);
    self
      .engine
      .call_fn::<Dynamic>(&mut scope, &self.ast, name, args.to_vec())
      .map_err(|err| {
        Self::print_script_error(&self.script_path, &self.script_contents, *err);
        format!("Could not run the {} callback", name)
      })
  }
}

impl Engine {
  fn configure_engine() -> rhai::Engine {
    let mut engine = rhai::Engine::new();

    // Exposes functions like `aws_cluster` to the scripts
    let utils_module = exported_module!(utils_module);
    let cluster_module = exported_module!(cluster_module);
    let host_module = exported_module!(host_module);

    // Register custom functions
    engine.register_global_module(cluster_module.into());
    engine.register_global_module(host_module.into());
    engine.register_global_module(utils_module.into());

    #[cfg(feature = "fs-access")]
    {
      let fs_pkg = FilesystemPackage::new();
      fs_pkg.register_into_engine(&mut engine);
    }

    engine
  }

  // Load the startup script from a file
  fn load_script(filename: &str) -> Result<(String, String), String> {
    // Canonicalize the path
    let canonical_path = Path::new(filename)
      .canonicalize()
      .map_err(|err| format!("Error in script file path: {filename}\n{err}"))?;

    let current_dir = std::env::current_dir()
      .map_err(|err| format!("Error getting current directory: {err}"))?
      .canonicalize()
      .map_err(|err| format!("Error canonicalizing current directory: {err}"))?;

    // Create a relative path if possible
    let canonical_path = canonical_path
      .strip_prefix(current_dir)
      .map_err(|err| format!("Error stripping script file path: {filename}\n{err}"))?;

    // Read file contents
    let contents = std::fs::read_to_string(&canonical_path).map_err(|err| {
      format!(
        "Error reading script file: {}\n{}",
        canonical_path.display(),
        err
      )
    })?;

    Ok((canonical_path.to_string_lossy().into(), contents))
  }

  fn print_script_error(script_path: &str, input: &str, mut err: EvalAltResult) {
    // Print the script path
    warn!("{:=<1$}", "", script_path.len());
    warn!("{script_path}");
    warn!("{:=<1$}", "", script_path.len());
    eprintln!();

    fn eprint_line(lines: &[&str], pos: Position, err_msg: &str) {
      let line = pos.line().unwrap();
      let line_no = format!("{line}: ");

      eprintln!("{line_no}{}", lines[line - 1]);

      for (i, err_line) in err_msg.to_string().lines().enumerate() {
        // Display position marker
        println!(
          "{0:>1$}{err_line}",
          if i > 0 { "| " } else { "^ " },
          line_no.len() + pos.position().unwrap() + 1,
        );
      }
      eprintln!();
    }

    // Do not use `line` because it "eats" the last empty line if the script ends with a newline.
    let lines: Vec<_> = input.split('\n').collect();

    // Print error
    let pos = err.take_position();

    if pos.is_none() {
      // No position
      eprintln!("{err}");
    } else {
      // Specific position
      eprint_line(&lines, pos, &err.to_string())
    }
  }
}
