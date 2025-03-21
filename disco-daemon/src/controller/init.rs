use std::path::{Path, PathBuf};

use disco_common::builder::cluster_module;

use rhai;
use rhai::{exported_module, EvalAltResult, Position};
use tracing::{info, warn};

pub struct Startup {
  script_path: PathBuf,
  engine: rhai::Engine,
}

impl Startup {
  pub fn new<S: Into<String>>(filename: S) -> Result<Self, Box<dyn std::error::Error>> {
    let engine = Self::configure_engine();

    // Load the script file
    let (script_path, script_contents) = Self::load_script(&filename.into())?;

    let expanded_filename = script_path.to_string_lossy();

    // Run the loaded script
    if let Err(err) = engine
      .compile(script_contents.clone())
      .map_err(|err| err.into())
      .and_then(|mut ast| {
        ast.set_source(expanded_filename.to_string());
        engine.run_ast(&ast)
      })
    {
      warn!("{:=<1$}", "", expanded_filename.len());
      warn!("{expanded_filename}");
      warn!("{:=<1$}", "", expanded_filename.len());
      eprintln!();

      Self::print_script_error(&script_contents, *err);
    }

    Ok(Self {
      script_path,
      engine,
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

  fn configure_engine() -> rhai::Engine {
    let mut engine = rhai::Engine::new();
    let module = exported_module!(cluster_module);
    // Register custom functions
    engine.register_global_module(module.into());

    // You can add more configuration here as needed

    engine
  }
}
