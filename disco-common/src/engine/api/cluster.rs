use std::cell::RefCell;

use boa_engine::{
  Context, JsNativeError, JsObject, JsResult, JsString, JsValue, NativeFunction,
  class::{Class, ClassBuilder},
  property::Attribute,
};
use boa_interop::{IntoJsFunctionCopied, JsClass};
use tracing::info;

use crate::{builder::Cluster, provider::AwsProvider};

fn healthy(
  _this: &JsValue,
  args: &[JsValue],
  _context: &RefCell<&mut Context>,
) -> impl Future<Output = JsResult<JsValue>> {
  async move {
    info!("Cluster::healthy called with args: {:?}", args);

    Ok(JsValue::from(false))
  }
}

fn set_key_pair(
  this: &JsValue,
  args: &[JsValue],
  context: &RefCell<&mut Context>,
) -> impl Future<Output = JsResult<JsValue>> {
  async move {
    let object = args
      .first()
      .ok_or_else(|| JsNativeError::typ().with_message("Missing argument"))?
      .as_object()
      .ok_or_else(|| JsNativeError::typ().with_message("Argument is not an object"))?;

    let private_key = object
      .get(JsString::from("private"), &mut context.borrow_mut())?
      .as_string()
      .ok_or_else(|| JsNativeError::typ().with_message("Argument `private` is not a string"))?
      .to_std_string_lossy();

    let public_key = object
      .get(JsString::from("public"), &mut context.borrow_mut())?
      .as_string()
      .ok_or_else(|| JsNativeError::typ().with_message("Argument `public` is not a string"))?
      .to_std_string_lossy();

    let cluster = this
      .as_object()
      .unwrap()
      .downcast_ref::<Cluster>()
      .unwrap()
      .clone();

    cluster
      .set_key_pair(&private_key, &public_key)
      .await
      .map_err(|e| JsNativeError::typ().with_message(e.to_string()))?;

    Ok(JsValue::undefined())
  }
}

fn start_instance(
  this: &JsValue,
  args: &[JsValue],
  context: &RefCell<&mut Context>,
) -> impl Future<Output = JsResult<JsValue>> {
  async move {
    let object = args
      .first()
      .ok_or_else(|| JsNativeError::typ().with_message("Missing argument"))?
      .as_object()
      .ok_or_else(|| JsNativeError::typ().with_message("Argument is not an object"))?;

    let image = object
      .get(JsString::from("image"), &mut context.borrow_mut())?
      .as_string()
      .ok_or_else(|| JsNativeError::typ().with_message("Argument `image` is not a string"))?
      .to_std_string_lossy();

    let instance_type = object
      .get(JsString::from("instance_type"), &mut context.borrow_mut())?
      .as_string()
      .ok_or_else(|| JsNativeError::typ().with_message("Argument `instance_type` is not a string"))?
      .to_std_string_lossy();

    let cluster = this
      .as_object()
      .ok_or_else(|| JsNativeError::typ().with_message("`this` is not an object"))?
      .downcast_ref::<Cluster>()
      .ok_or_else(|| JsNativeError::typ().with_message("`this` is not a Cluster"))?
      .clone();

    cluster
      .start_instance(&image, &instance_type)
      .await
      .map_err(|e| JsNativeError::typ().with_message(e.to_string()))?;

    Ok(JsValue::undefined())
  }
}

fn attach_ip(
  this: &JsValue,
  _args: &[JsValue],
  _context: &RefCell<&mut Context>,
) -> impl Future<Output = JsResult<JsValue>> {
  async move {
    let cluster = this
      .as_object()
      .ok_or_else(|| JsNativeError::typ().with_message("`this` is not an object"))?
      .downcast_ref::<Cluster>()
      .ok_or_else(|| JsNativeError::typ().with_message("`this` is not a Cluster"))?
      .clone();

    cluster
      .attach_ip()
      .await
      .map_err(|e| JsNativeError::typ().with_message(e.to_string()))?;

    Ok(JsValue::undefined())
  }
}

fn ssh_install(
  this: &JsValue,
  _args: &[JsValue],
  _context: &RefCell<&mut Context>,
) -> impl Future<Output = JsResult<JsValue>> {
  async move {
    let cluster = this
      .as_object()
      .ok_or_else(|| JsNativeError::typ().with_message("`this` is not an object"))?
      .downcast_ref::<Cluster>()
      .ok_or_else(|| JsNativeError::typ().with_message("`this` is not a Cluster"))?
      .clone();

    cluster
      .ssh_install()
      .await
      .map_err(|e| JsNativeError::typ().with_message(e.to_string()))?;

    Ok(JsValue::undefined())
  }
}

impl Class for Cluster {
  const NAME: &'static str = "Cluster";
  const LENGTH: usize = 0;

  #[allow(clippy::items_after_statements)]
  fn init(class: &mut ClassBuilder<'_>) -> JsResult<()> {
    #[allow(dead_code)]
    let function_get = IntoJsFunctionCopied::into_js_function_copied(
      |this: JsClass<Cluster>| -> JsString { this.borrow().name().into() },
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
      JsString::from("healthy"),
      0,
      NativeFunction::from_async_fn(healthy),
    );

    class.method(
      JsString::from("set_key_pair"),
      2,
      NativeFunction::from_async_fn(set_key_pair),
    );

    class.method(
      JsString::from("start_instance"),
      2,
      NativeFunction::from_async_fn(start_instance),
    );

    class.method(
      JsString::from("attach_ip"),
      0,
      NativeFunction::from_async_fn(attach_ip),
    );

    class.method(
      JsString::from("ssh_install"),
      0,
      NativeFunction::from_async_fn(ssh_install),
    );

    Ok(())
  }
  #[allow(unused_variables)]
  fn data_constructor(
    new_target: &JsValue,
    args: &[JsValue],
    context: &mut Context,
  ) -> JsResult<Cluster> {
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

    let provider_value = object.get(JsString::from("provider"), context)?;
    let provider_object = provider_value
      .as_object()
      .ok_or_else(|| JsNativeError::typ().with_message("Argument `provider` is not an object"))?;

    let cluster = Cluster::new(
      name,
      provider_object
        .downcast_ref::<AwsProvider>()
        .unwrap()
        .clone(),
    );

    Ok(cluster)
  }

  fn object_constructor(
    _instance: &JsObject,
    _args: &[JsValue],
    _context: &mut Context,
  ) -> JsResult<()> {
    Ok(())
  }
}
