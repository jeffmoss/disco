use std::cell::RefCell;

use boa_engine::{
  Context, JsNativeError, JsObject, JsResult, JsString, JsValue, NativeFunction,
  class::{Class, ClassBuilder},
  property::Attribute,
  value::TryFromJs,
};
use boa_interop::{IntoJsFunctionCopied, JsClass};

use crate::provider::{AwsProvider, Provider};

#[derive(TryFromJs)]
struct Parameters {
  name: String,
  region: String,
}

fn storage(
  _this: &JsValue,
  args: &[JsValue],
  _context: &RefCell<&mut Context>,
) -> impl Future<Output = JsResult<JsValue>> {
  async move { Ok(JsValue::from(false)) }
}

fn init(
  _this: &JsValue,
  args: &[JsValue],
  context: &RefCell<&mut Context>,
) -> impl Future<Output = JsResult<JsValue>> {
  async move {
    if let Some(arg) = args.first() {
      let native_args = Parameters::try_from_js(arg, &mut context.borrow_mut())?;

      // We check if the type of `args[0]` is `Person`
      let provider = AwsProvider::new(native_args.name, native_args.region)
        .await
        .expect("Failed to create AwsProvider");

      return Ok(
        AwsProvider::from_data(provider, &mut context.borrow_mut())
          .unwrap()
          .into(),
      );
    }
    // If `this` was not an object or the type of `this` was not a native object `Person`,
    // we throw a `TypeError`.
    Err(
      JsNativeError::typ()
        .with_message("AwsProvider.init() failed. Did you pass the correct parameters?")
        .into(),
    )
  }
}

impl Class for AwsProvider {
  const NAME: &'static str = "AwsProvider";
  const LENGTH: usize = 0;

  #[allow(clippy::items_after_statements)]
  fn init(class: &mut ClassBuilder<'_>) -> JsResult<()> {
    #[allow(dead_code)]
    let function_get = IntoJsFunctionCopied::into_js_function_copied(
      |this: JsClass<AwsProvider>| -> JsString { this.borrow().cluster_name.clone().into() },
      class.context(),
    )
    .to_js_function(class.context().realm());

    class.accessor(
      JsString::from("name"),
      Some(function_get),
      None,
      Attribute::CONFIGURABLE | Attribute::NON_ENUMERABLE,
    );

    class.method(
      JsString::from("storage"),
      1,
      NativeFunction::from_async_fn(storage),
    );

    class.static_method(
      JsString::from("init"),
      1,
      NativeFunction::from_async_fn(init),
    );

    Ok(())
  }

  #[allow(unused_variables)]
  fn data_constructor(
    new_target: &JsValue,
    args: &[JsValue],
    context: &mut Context,
  ) -> JsResult<Self> {
    let rest = args;
    return Err(
      JsNativeError::typ()
        .with_message("AwsProvider cannot be constructed directly. Use AwsProvider.init()")
        .into(),
    );
  }
  fn object_constructor(
    _instance: &JsObject,
    _args: &[JsValue],
    _context: &mut Context,
  ) -> JsResult<()> {
    Ok(())
  }
}
