use std::cell::RefCell;

use boa_engine::{
  Context, JsNativeError, JsObject, JsResult, JsString, JsValue, NativeFunction,
  class::{Class, ClassBuilder},
  property::Attribute,
};
use boa_interop::{IntoJsFunctionCopied, JsClass};

use crate::{builder::Storage, provider::AwsProvider};

fn ensure(
  this: &JsValue,
  args: &[JsValue],
  _context: &RefCell<&mut Context>,
) -> impl Future<Output = JsResult<JsValue>> {
  async move {
    let storage = this
      .as_object()
      .unwrap()
      .downcast_ref::<Storage>()
      .unwrap()
      .clone();

    storage
      .ensure()
      .await
      .map_err(|e| JsNativeError::typ().with_message(e.to_string()))?;

    Ok(JsValue::from(true))
  }
}

impl Class for Storage {
  const NAME: &'static str = "Storage";
  const LENGTH: usize = 0;

  #[allow(clippy::items_after_statements)]
  fn init(class: &mut ClassBuilder<'_>) -> JsResult<()> {
    #[allow(dead_code)]
    let function_get = IntoJsFunctionCopied::into_js_function_copied(
      |this: JsClass<Storage>| -> JsString { this.borrow().name().into() },
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
      JsString::from("ensure"),
      0,
      NativeFunction::from_async_fn(ensure),
    );

    Ok(())
  }
  #[allow(unused_variables)]
  fn data_constructor(
    new_target: &JsValue,
    args: &[JsValue],
    context: &mut Context,
  ) -> JsResult<Storage> {
    let object = args
      .first()
      .ok_or_else(|| JsNativeError::typ().with_message("Missing argument"))?
      .as_object()
      .ok_or_else(|| JsNativeError::typ().with_message("Argument is not an object"))?;

    let name = object
      .get(JsString::from("name"), context)?
      .as_string()
      .ok_or_else(|| JsNativeError::typ().with_message("Argument `name` is not a string"))?
      .to_std_string_lossy();

    let role = object
      .get(JsString::from("role"), context)?
      .as_string()
      .ok_or_else(|| JsNativeError::typ().with_message("Argument `role` is not a string"))?
      .to_std_string_lossy();

    let provider_value = object.get(JsString::from("provider"), context)?;
    let provider_object = provider_value
      .as_object()
      .ok_or_else(|| JsNativeError::typ().with_message("Argument `provider` is not an object"))?;

    let storage = Storage::new(
      name,
      role,
      provider_object
        .downcast_ref::<AwsProvider>()
        .unwrap()
        .clone(),
    );

    Ok(storage)
  }

  fn object_constructor(
    _instance: &JsObject,
    _args: &[JsValue],
    _context: &mut Context,
  ) -> JsResult<()> {
    Ok(())
  }
}
