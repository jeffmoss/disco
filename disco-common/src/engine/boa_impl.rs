use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use boa_engine::{
  builtins::promise::PromiseState, js_str, module::SimpleModuleLoader, Context, JsError,
  JsNativeError, JsString, JsValue, Module, Source,
};
use tracing::{info, warn};

use super::EngineInterface;

pub struct Engine {
  script_path: String,
  script_contents: String,
  context: Mutex<Context>,
  module: Module,
}

impl EngineInterface<JsValue> for Engine {
  fn new(filename: &str) -> Result<Self, String> {
    // Create the module loader (similar to your example)
    let module_dir = Path::new(filename)
      .parent()
      .ok_or_else(|| format!("Could not determine parent directory of {}", filename))?
      .to_path_buf();

    // Create the context with the module loader
    let mut context = Context::builder()
      .build()
      .map_err(|err| format!("Failed to create JavaScript context: {}", err))?;

    // Register custom bindings (similar to how you register modules in Rhai)
    Self::register_bindings(&mut context)?;

    // Load the script file
    let (script_path, script_contents) = Self::load_script(filename)?;

    // Parse and load the module
    let source = Source::from_bytes(&script_contents);
    let module = Module::parse(source, None, &mut context).map_err(|err| {
      Self::print_script_error(&script_path, &script_contents, err);
      format!("Failed to parse script module: {}", filename)
    })?;

    // Link and evaluate the module
    let promise_result = module.load_link_evaluate(&mut context);
    context.run_jobs();

    // Check if the module loaded successfully
    match promise_result.state() {
      PromiseState::Pending => {
        return Err("Module didn't execute completely".into());
      }
      PromiseState::Rejected(err) => {
        let js_error = JsError::from_opaque(err.clone());
        return Err(format!("Module evaluation failed: {}", js_error));
      }
      PromiseState::Fulfilled(_) => {
        // Module executed successfully
      }
    };

    Ok(Self {
      script_path,
      script_contents,
      context: Mutex::new(context),
      module,
    })
  }

  fn callback(&self, name: &str, args: &[JsValue]) -> Result<JsValue, String> {
    let mut context = self.context.lock().unwrap();

    info!("About to run {}()", name);

    // Get the module namespace and the exported function
    let namespace = self.module.namespace(&mut context);

    let function = namespace
      .get(JsString::from(name), &mut context)
      .map_err(|err| format!("Error accessing function '{}': {}", name, err))?
      .as_callable()
      .cloned()
      .ok_or_else(|| format!("'{}' is not a callable function", name))?;

    // Call the function with the provided arguments
    function
      .call(
        &JsValue::undefined(), // this value
        args,                  // function arguments
        &mut context,
      )
      .map_err(|err| {
        Self::print_script_error(&self.script_path, &self.script_contents, err);
        format!("Could not run the {} callback", name)
      })
  }
}

impl Engine {
  // Register custom bindings with the JavaScript context
  fn register_bindings(context: &mut Context) -> Result<(), String> {
    // Similar to how you register modules in Rhai
    // For example, to register a Game class like in your example:
    #[cfg(feature = "game")]
    {
      use crate::game::Game;
      context
        .register_global_class::<Game>()
        .map_err(|err| format!("Failed to register Game class: {}", err))?;
    }

    // Register other bindings for your builder modules
    // This would depend on how your builder module defines JavaScript bindings
    // You might need to create JavaScript-compatible wrappers for your Rust functions

    Ok(())
  }

  // Load the script file
  fn load_script(filename: &str) -> Result<(String, String), String> {
    // Canonicalize the path
    let canonical_path = Path::new(filename)
      .canonicalize()
      .map_err(|err| format!("Error in script file path: {}\n{}", filename, err))?;

    let current_dir = std::env::current_dir()
      .map_err(|err| format!("Error getting current directory: {}", err))?
      .canonicalize()
      .map_err(|err| format!("Error canonicalizing current directory: {}", err))?;

    // Create a relative path if possible
    let canonical_path = match canonical_path.strip_prefix(&current_dir) {
      Ok(path) => path.to_path_buf(),
      Err(_) => canonical_path,
    };

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

  // Print script errors
  fn print_script_error(script_path: &str, script_contents: &str, err: impl std::fmt::Display) {
    warn!("{:=<1$}", "", script_path.len());
    warn!("{}", script_path);
    warn!("{:=<1$}", "", script_path.len());

    // Boa doesn't have the same position info as Rhai, so we just print the error
    eprintln!("\nError: {}\n", err);

    // If we had line/column info, we could highlight the error location like in Rhai
    // For now, we just print the script contents for reference
    eprintln!("Script contents:");
    for (i, line) in script_contents.lines().enumerate() {
      eprintln!("{}: {}", i + 1, line);
    }
  }
}
