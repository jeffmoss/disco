pub use boa_engine::JsValue;
use boa_engine::{
  Context, JsArgs, JsError, JsNativeError, JsResult, JsString, Module, NativeFunction, Source,
  builtins::promise::PromiseState, context::ContextBuilder, property::Attribute,
};
use boa_runtime::Console;
use std::{
  cell::RefCell, future::Future, mem, path::Path, rc::Rc, thread::JoinHandle, time::Duration,
};
use tokio::{
  io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader},
  runtime::Builder,
  sync::{
    mpsc::{self, Sender},
    oneshot,
  },
  time::{self, Instant},
};
use tracing::{info, warn};

mod queue;
use queue::Queue;

mod api;

use crate::{builder::Cluster, provider::AwsProvider};

// Example async function. Note that the returned future must be 'static.
fn delay(
  _this: &JsValue,
  args: &[JsValue],
  context: &RefCell<&mut Context>,
) -> impl Future<Output = JsResult<JsValue>> {
  async move {
    let millis = args.get_or_undefined(0).to_u32(&mut context.borrow_mut())?;

    println!("(delay) Sleep for {millis} milliseconds ...");

    let now = Instant::now();
    time::sleep(Duration::from_millis(u64::from(millis))).await;
    let elapsed = now.elapsed().as_secs_f64();

    Ok(elapsed.into())
  }
}

// Ask the user for input.
// TODO: move this to a utility module.
fn ask(
  _this: &JsValue,
  args: &[JsValue],
  context: &RefCell<&mut Context>,
) -> impl Future<Output = JsResult<JsValue>> {
  async move {
    let prompt = args
      .get_or_undefined(0)
      .to_string(&mut context.borrow_mut())?;

    let mut stdout = io::stdout();
    stdout
      .write_all(format!("{} (yes/no): ", prompt.to_std_string_lossy()).as_bytes())
      .await
      .unwrap();
    stdout.flush().await.unwrap();

    let stdin = io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut input = String::new();
    reader.read_line(&mut input).await.unwrap();

    Ok(matches!(input.trim().to_lowercase().as_str(), "yes").into())
  }
}

#[derive(Debug)]
pub enum EngineError {
  SendCallback(mpsc::error::SendError<Command>),
  ReceiveCallback(oneshot::error::RecvError),
  Script(String),
  NoModuleLoaded,
}

impl std::fmt::Display for EngineError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      EngineError::SendCallback(e) => write!(f, "Send error: {}", e),
      EngineError::ReceiveCallback(e) => write!(f, "Receive error: {}", e),
      EngineError::Script(e) => write!(f, "Script error: {}", e),
      EngineError::NoModuleLoaded => write!(f, "No module has been loaded"),
    }
  }
}

impl From<std::io::Error> for EngineError {
  fn from(err: std::io::Error) -> Self {
    EngineError::Script(err.to_string())
  }
}

impl From<JsError> for EngineError {
  fn from(err: JsError) -> Self {
    EngineError::Script(err.to_string())
  }
}

impl From<String> for EngineError {
  fn from(err: String) -> Self {
    EngineError::Script(err.to_string())
  }
}

impl std::error::Error for EngineError {}

pub enum Command {
  Process(String, Vec<JsValue>, oneshot::Sender<JsValue>),
  LoadModule(String, oneshot::Sender<Result<(), String>>),
  Terminate,
}

pub struct Engine {
  thread_handle: JoinHandle<()>,
  command_tx: Sender<Command>,
}

impl Engine {
  pub fn new(filename: Option<&str>) -> Result<Self, EngineError> {
    let (command_tx, mut command_rx) = mpsc::channel::<Command>(10);

    // Optionally load the script file if provided
    let initial_script = if let Some(filename) = filename {
      Some(Self::load_script(filename)?.1)
    } else {
      None
    };

    let thread_handle = std::thread::spawn(move || {
      // Create a second runtime in this separate OS thread
      let local_runtime = Builder::new_current_thread()
        .enable_time()
        .enable_io()
        .build_local(&mut Default::default())
        .unwrap();

      let queue = Queue::new();

      let context = &mut ContextBuilder::new()
        .job_executor(Rc::new(queue))
        .build()
        .unwrap();

      // Set up the context with globals and console
      let console = Console::init(context);
      context
        .register_global_property(Console::NAME, console, Attribute::all())
        .expect("the console builtin shouldn't exist");

      context.register_global_class::<AwsProvider>().unwrap();
      context.register_global_class::<Cluster>().unwrap();

      // Bind the delay async function to the ECMAScript function "delay". (testing purposes)
      context
        .register_global_builtin_callable(
          JsString::from("delay"),
          1,
          NativeFunction::from_async_fn(delay),
        )
        .expect("the delay function shouldn't exist");

      context
        .register_global_builtin_callable(
          JsString::from("ask"),
          1,
          NativeFunction::from_async_fn(ask),
        )
        .expect("the ask function shouldn't exist");

      local_runtime.block_on(async {
        let mut current_module: Option<Module> = None;

        // Load initial module if provided
        if let Some(script_contents) = initial_script {
          match Self::load_module_from_contents(&script_contents, context).await {
            Ok(module) => {
              current_module = Some(module);
            }
            Err(e) => {
              warn!("Failed to load initial module: {}", e);
            }
          }
        }

        // Can also pass a `Some(realm)` if you need to execute the module in another realm.
        while let Some(command) = command_rx.recv().await {
          match command {
            Command::LoadModule(script_contents, response_tx) => {
              info!("Loading new module");

              match Self::load_module_from_contents(&script_contents, context).await {
                Ok(module) => {
                  current_module = Some(module);
                  let _ = response_tx.send(Ok(()));
                }
                Err(e) => {
                  let _ = response_tx.send(Err(e));
                }
              }
            }
            Command::Process(data, input, response_tx) => {
              info!("Processing command: {:?}", data);

              let module = match &current_module {
                Some(module) => module,
                None => {
                  let _ = response_tx.send(JsValue::undefined());
                  continue;
                }
              };

              let namespace = module.namespace(context);

              let func = match namespace.get(JsString::from(data.clone()), context) {
                Ok(value) => match value.as_callable().cloned() {
                  Some(func) => func,
                  None => {
                    warn!("Command '{}' is not a callable function", &data);
                    let _ = response_tx.send(JsValue::undefined());
                    continue;
                  }
                },
                Err(e) => {
                  warn!("Could not get command function '{}': {}", &data, e);
                  let _ = response_tx.send(JsValue::undefined());
                  continue;
                }
              };

              let result = match func.call(&JsValue::undefined(), &input, context) {
                Ok(result) => {
                  info!("Pending promise: {:?}", result);
                  result
                }
                Err(e) => {
                  warn!("Could not call command function: {}", e);
                  let _ = response_tx.send(JsValue::undefined());
                  continue;
                }
              };

              let prom = match result.as_promise() {
                Some(prom) => prom,
                None => {
                  // Not a promise, send the result directly
                  let _ = response_tx.send(result);
                  continue;
                }
              };

              let command_future = prom.into_js_future(context);

              local_runtime.spawn_local(async move {
                let result = command_future.await;
                info!("command_future done awaiting, sending response...");
                match result {
                  Ok(value) => {
                    let _ = response_tx.send(value);
                  }
                  Err(err) => {
                    info!("Promise rejected with: {}", err);
                    let _ = response_tx.send(JsValue::undefined());
                  }
                }
              });

              let unsafe_context: &'static mut Context = unsafe {
                // This extends the lifetime to 'static, but it's a lie.
                // context_ref could be freed once the command_rx loop exits.
                //
                // In order to make this "safe" we need to ensure that the
                // task spawned here completes before the command loop is terminated
                // or cancel the tasks upon termination. In our case the tasks are
                // automatically canceled when the runtime is dropped.
                //
                // It is essential that mutable RefCell<&Context> borrows are not
                // held across await points in native async code that runs within
                // the spawned tasks.
                mem::transmute::<&mut Context, &'static mut Context>(context)
              };

              let _job_handle = local_runtime.spawn_local(async move {
                let _ = unsafe_context.run_jobs_async().await;
              });
            }
            Command::Terminate => {
              break;
            }
          }
        }
      });
    });

    Ok(Engine {
      thread_handle,
      command_tx,
    })
  }

  pub async fn load_module(&self, script_contents: &str) -> Result<(), EngineError> {
    let (response_tx, response_rx) = oneshot::channel();

    self
      .command_tx
      .send(Command::LoadModule(
        script_contents.to_string(),
        response_tx,
      ))
      .await
      .map_err(EngineError::SendCallback)?;

    response_rx
      .await
      .map_err(EngineError::ReceiveCallback)?
      .map_err(EngineError::Script)
  }

  pub async fn load_module_from_file(&self, filename: &str) -> Result<(), EngineError> {
    let (_, script_contents) = Self::load_script(filename)?;
    self.load_module(&script_contents).await
  }

  pub async fn callback(&self, data: &str, input: &[JsValue]) -> Result<JsValue, EngineError> {
    let (response_tx, response_rx) = oneshot::channel();

    self
      .command_tx
      .send(Command::Process(data.into(), input.to_vec(), response_tx))
      .await
      .map_err(EngineError::SendCallback)?;

    response_rx.await.map_err(EngineError::ReceiveCallback)
  }

  pub async fn init(&self) -> Result<JsValue, EngineError> {
    // Call the init function in the script
    let cluster = self.callback("init", &[]).await?;

    Ok(cluster)
  }

  pub async fn terminate(self) {
    let _ = self.command_tx.send(Command::Terminate).await;
    let _ = self.thread_handle.join();
  }

  // Helper function to load and parse a module from script contents
  async fn load_module_from_contents(
    script_contents: &str,
    context: &mut Context,
  ) -> Result<Module, String> {
    // Parse and load the module
    let source = Source::from_bytes(script_contents);
    let module = Module::parse(source, None, context)
      .map_err(|e| format!("Could not parse script module: {}", e))?;

    let promise_result = module.load_link_evaluate(context);

    context
      .run_jobs_async()
      .await
      .map_err(|e| format!("Import module job failed: {}", e))?;

    match promise_result.state() {
      PromiseState::Fulfilled(_value) => {
        info!("Module loaded successfully");
      }
      PromiseState::Rejected(reason) => {
        let error_msg = format!("Module loading rejected: {}", reason.display());
        warn!("{}", error_msg);
        return Err(error_msg);
      }
      PromiseState::Pending => {
        let error_msg = "Module loading is still pending".to_string();
        warn!("{}", error_msg);
        return Err(error_msg);
      }
    }

    Ok(module)
  }

  // Load the script file
  fn load_script(filename: &str) -> Result<(String, String), EngineError> {
    // Canonicalize the path
    let canonical_path = Path::new(filename).canonicalize()?;

    let current_dir = std::env::current_dir()?.canonicalize()?;

    // Create a relative path if possible
    let canonical_path = match canonical_path.strip_prefix(&current_dir) {
      Ok(path) => path.to_path_buf(),
      Err(_) => canonical_path,
    };

    // Read file contents
    let contents = std::fs::read_to_string(&canonical_path)?;

    Ok((canonical_path.to_string_lossy().into(), contents))
  }

  // Print script errors (not the best)
  fn _print_script_error(script_path: &str, script_contents: &str, err: impl std::fmt::Display) {
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
