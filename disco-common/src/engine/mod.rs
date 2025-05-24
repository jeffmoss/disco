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
}

impl std::fmt::Display for EngineError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      EngineError::SendCallback(e) => write!(f, "Send error: {}", e),
      EngineError::ReceiveCallback(e) => write!(f, "Receive error: {}", e),
      EngineError::Script(e) => write!(f, "Script error: {}", e),
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
  Terminate,
}

pub struct Engine {
  thread_handle: JoinHandle<()>,
  command_tx: Sender<Command>,
}

impl Engine {
  pub fn new(filename: &str) -> Result<Self, EngineError> {
    let (command_tx, mut command_rx) = mpsc::channel::<Command>(10);

    // Load the script file
    let (_script_path, script_contents) = Self::load_script(filename)?;

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

      // Parse and load the module
      let source = Source::from_bytes(&script_contents);
      let module = Module::parse(source, None, context).expect("Could not parse script module");
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
        let promise_result = module.load_link_evaluate(context);

        context
          .run_jobs_async()
          .await
          .expect("Import module job failed");

        match promise_result.state() {
          PromiseState::Fulfilled(_value) => {
            // info!(
            //   "Import: Promise fulfilled with value: {}",
            //   value.display()
            // );
          }
          PromiseState::Rejected(reason) => {
            info!("Import: Promise rejected with reason: {}", reason.display());
          }
          PromiseState::Pending => {
            info!("Import: Promise is still pending");
          }
        }

        let namespace = module.namespace(context);

        // Can also pass a `Some(realm)` if you need to execute the module in another realm.
        while let Some(command) = command_rx.recv().await {
          match command {
            Command::Process(data, input, response_tx) => {
              info!("Processing command: {:?}", data);

              let func = namespace
                .get(JsString::from(data), context)
                .expect("Could not get command function")
                .as_callable()
                .cloned()
                .ok_or_else(|| {
                  JsNativeError::typ().with_message("command export wasn't a function")
                })
                .expect("Could not convert to callable");

              let result = func
                .call(&JsValue::undefined(), &input, context)
                .and_then(|result| {
                  info!("Pending promise: {:?}", result);
                  Ok(result)
                })
                .expect("Could not call command function");

              let prom = result.as_promise().expect("Error with JsPromise");

              let command_future = prom.into_js_future(context);

              local_runtime.spawn_local(async move {
                let result = command_future.await;
                info!("command_future done awaiting, sending response...");
                match result {
                  Ok(value) => {
                    response_tx.send(value).expect("Failed to send response");
                  }
                  Err(err) => info!("Promise rejected with: {}", err),
                }
              });

              let unsafe_context: &'static mut Context = unsafe {
                // This extends the lifetime to 'static, but it's a lie.
                // context_ref could be freed once the command_rx loop exits.
                //
                // In order to make this "safe" we need to ensure that the
                // task spawned here complete before the command loop is terminated
                // (as shown) or cancel the tasks upon termination.
                mem::transmute::<&mut Context, &'static mut Context>(context)
              };

              let _job_handle = local_runtime.spawn_local(async move {
                unsafe_context
                  .run_jobs_async()
                  .await
                  .expect("Failed to run jobs");
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
    self.command_tx.send(Command::Terminate).await.unwrap();
    self.thread_handle.join().unwrap();
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
