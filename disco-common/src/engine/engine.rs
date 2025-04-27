use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::builder::{cluster_module, Cluster};
use crate::provider::{aws_provider_module, Address};

use rhai::{self, Dynamic, Scope};
use rhai::{exported_module, EvalAltResult, Position};
use tracing::{info, warn};

use rhai::packages::Package;
#[cfg(feature = "fs-access")]
use rhai_fs::FilesystemPackage;

#[derive(Clone)]
pub struct Engine {
  script_path: PathBuf,
  script_contents: String,
  pub rhai_engine: Arc<rhai::Engine>,
  pub ast: rhai::AST,
  pub scope: Arc<Mutex<rhai::Scope<'static>>>,
}

impl Engine {
  pub fn new<S: Into<String>>(filename: S) -> Result<Self, Box<dyn std::error::Error>> {
    let rhai_engine = Arc::new(Self::configure_rhai_engine());

    // Load the script file
    let (script_path, script_contents) = Self::load_script(&filename.into())?;

    let expanded_filename = script_path.to_string_lossy();

    // Run the loaded script
    let ast = match rhai_engine.compile(script_contents.clone()) {
      Ok(compiled_ast) => {
        let mut compiled_ast = compiled_ast;
        compiled_ast.set_source(expanded_filename.to_string());
        compiled_ast
      }

      Err(err) => {
        warn!("{:=<1$}", "", expanded_filename.len());
        warn!("{expanded_filename}");
        warn!("{:=<1$}", "", expanded_filename.len());
        eprintln!();

        Self::print_script_error(&script_contents, err.clone().into());
        return Err(Box::new(err));
      }
    };

    let scope = Scope::new();

    Ok(Self {
      script_path,
      script_contents,
      rhai_engine: rhai_engine.into(),
      ast,
      scope: Arc::new(Mutex::new(scope)),
    })
  }

  // Load the startup script from a file
  fn load_script(filename: &str) -> Result<(PathBuf, String), Box<dyn std::error::Error>> {
    // Canonicalize the path
    let canonical_path = match Path::new(filename).canonicalize() {
      Ok(f) => match f.strip_prefix(std::env::current_dir().unwrap().canonicalize().unwrap()) {
        Ok(f) => f.into(),
        _ => f,
      },
      Err(err) => {
        info!("Error script file path: {filename}\n{err}");
        return Err(err.into());
      }
    };

    // Read file contents
    let contents = std::fs::read_to_string(&canonical_path).map_err(|err| {
      info!(
        "Error reading script file: {}\n{}",
        canonical_path.display(),
        err
      );
      Box::new(err)
    })?;

    Ok((canonical_path, contents))
  }

  fn print_script_error(input: &str, mut err: EvalAltResult) {
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

  fn configure_rhai_engine() -> rhai::Engine {
    let mut engine = rhai::Engine::new();

    // Exposes functions like `aws_cluster` to the scripts
    let cluster_module = exported_module!(cluster_module);
    let aws_module = exported_module!(aws_provider_module);

    // Register custom functions
    engine.register_global_module(cluster_module.into());
    engine.register_global_module(aws_module.into());
    engine.build_type::<Address>();

    #[cfg(feature = "fs-access")]
    {
      let fs_pkg = FilesystemPackage::new();
      fs_pkg.register_into_engine(&mut engine);
    }

    engine
  }

  pub fn main(&self) {
    let mut scope = Scope::new();

    let cluster = Cluster::new("default");

    info!("About to run main()");
    match self
      .rhai_engine
      .call_fn::<Dynamic>(&mut scope, &self.ast, "main", (cluster.clone(),))
    {
      Ok(_) => {
        // Function call succeeded
      }
      Err(err) => {
        Self::print_script_error(&self.script_contents, *err);
      }
    }

    cluster.run_bootstrap(self);
  }
}
