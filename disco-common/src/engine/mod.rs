#[cfg(feature = "rhai-engine")]
mod rhai_impl;
#[cfg(feature = "rhai-engine")]
pub use rhai_impl::Engine;

#[cfg(feature = "boa-engine")]
mod boa_impl;
#[cfg(feature = "boa-engine")]
pub use boa_impl::Engine;

// Make sure features are mutually exclusive
#[cfg(all(feature = "rhai-engine", feature = "boa-engine"))]
compile_error!("Features 'rhai-engine' and 'boa-engine' are mutually exclusive");

// Ensure at least one engine is selected
#[cfg(not(any(feature = "rhai-engine", feature = "boa-engine")))]
compile_error!("Either 'rhai-engine' or 'boa-engine' feature must be enabled");

pub trait EngineInterface<V> {
  /// Create a new script engine instance from a script file
  fn new(filename: &str) -> Result<Self, String>
  where
    Self: Sized;

  /// Call a function in the script with the given arguments
  fn callback(&self, name: &str, args: &[V]) -> Result<V, String>;
}
