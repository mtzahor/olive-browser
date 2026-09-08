//! Small document-only browser globals. Storage belongs to one document realm,
//! is memory-only, and cannot read browser profiles or another page's data.
use super::{accessor, host, string_arg};
use boa_engine::{
    Context, JsData, JsNativeError, JsResult, JsString, JsValue, NativeFunction, js_string,
    object::{ObjectInitializer, builtins::JsArray},
    property::Attribute,
};
use boa_gc::{Finalize, Trace};

const MAX_STORAGE_BYTES: usize = 64 * 1024;
const MAX_STORAGE_KEYS: usize = 128;

#[derive(Default)]
pub(super) struct Storage {
    entries: Vec<(String, String)>,
    bytes: usize,
}

#[derive(Debug, Trace, Finalize, JsData)]
struct StorageHandle {
    index: usize,
}

pub(super) fn install(context: &mut Context) -> JsResult<()> {
    for (index, name) in ["localStorage", "sessionStorage"].into_iter().enumerate() {
        let mut builder = ObjectInitializer::with_native_data(StorageHandle { index }, context);
        builder
            .function(
                NativeFunction::from_fn_ptr(get_item),
                js_string!("getItem"),
                1,
            )
            .function(
                NativeFunction::from_fn_ptr(set_item),
                js_string!("setItem"),
                2,
            )
            .function(
                NativeFunction::from_fn_ptr(remove_item),
                js_string!("removeItem"),
                1,
            )
            .function(NativeFunction::from_fn_ptr(clear), js_string!("clear"), 0)
            .function(NativeFunction::from_fn_ptr(key), js_string!("key"), 1);
        accessor(&mut builder, "length", length, None);
        let storage = builder.build();
        context.register_global_property(JsString::from(name), storage, Attribute::READONLY)?;
    }
    let languages = JsArray::from_iter([JsValue::from(js_string!("en-US"))], context);
    let navigator = ObjectInitializer::new(context)
        .property(
            js_string!("userAgent"),
            JsString::from(concat!("OliveBrowser/", env!("CARGO_PKG_VERSION"))),
            Attribute::READONLY,
        )
        .property(
            js_string!("appName"),
            js_string!("Netscape"),
            Attribute::READONLY,
        )
        .property(
            js_string!("appVersion"),
            JsString::from(env!("CARGO_PKG_VERSION")),
            Attribute::READONLY,
        )
        .property(
            js_string!("platform"),
            js_string!("Olive"),
            Attribute::READONLY,
        )
        .property(
            js_string!("language"),
            js_string!("en-US"),
            Attribute::READONLY,
        )
        .property(js_string!("languages"), languages, Attribute::READONLY)
        .property(js_string!("cookieEnabled"), false, Attribute::READONLY)
        .property(js_string!("maxTouchPoints"), 0, Attribute::READONLY)
        .build();
    context.register_global_property(js_string!("navigator"), navigator, Attribute::READONLY)
}

fn index(this: &JsValue) -> JsResult<usize> {
    this.as_object()
        .and_then(|o| o.downcast_ref::<StorageHandle>().map(|h| h.index))
        .ok_or_else(|| {
            JsNativeError::typ()
                .with_message("Expected an Olive Storage object")
                .into()
        })
}

fn get_item(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = index(this)?;
    let key = string_arg(args, 0, context)?;
    let mut host = host(context).borrow_mut();
    host.spend(MAX_STORAGE_KEYS, 0, 0)?;
    Ok(host.storage[index]
        .entries
        .iter()
        .find(|(k, _)| k == &key)
        .map_or_else(JsValue::null, |(_, value)| {
            JsString::from(value.as_str()).into()
        }))
}

fn set_item(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = index(this)?;
    let key = string_arg(args, 0, context)?;
    let value = string_arg(args, 1, context)?;
    let mut host = host(context).borrow_mut();
    host.spend(MAX_STORAGE_KEYS, 0, 0)?;
    let storage = &host.storage[index];
    let existing = storage.entries.iter().position(|(k, _)| k == &key);
    let previous = existing.map_or(0, |i| {
        storage.entries[i].0.len() + storage.entries[i].1.len()
    });
    let size = storage.bytes - previous + key.len() + value.len();
    if size > MAX_STORAGE_BYTES || (existing.is_none() && storage.entries.len() == MAX_STORAGE_KEYS)
    {
        return Err(JsNativeError::range()
            .with_message("Olive storage quota exceeded (64 KiB / 128 keys per store)")
            .into());
    }
    host.spend(0, key.len() + value.len(), 0)?;
    let storage = &mut host.storage[index];
    storage.bytes = size;
    if let Some(i) = existing {
        storage.entries[i] = (key, value);
    } else {
        storage.entries.push((key, value));
    }
    Ok(JsValue::undefined())
}

fn remove_item(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = index(this)?;
    let key = string_arg(args, 0, context)?;
    let mut host = host(context).borrow_mut();
    host.spend(MAX_STORAGE_KEYS, 0, 0)?;
    let storage = &mut host.storage[index];
    if let Some(i) = storage.entries.iter().position(|(k, _)| k == &key) {
        let (key, value) = storage.entries.remove(i);
        storage.bytes -= key.len() + value.len();
    }
    Ok(JsValue::undefined())
}

fn clear(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = index(this)?;
    let mut host = host(context).borrow_mut();
    host.spend(MAX_STORAGE_KEYS, 0, 0)?;
    host.storage[index] = Storage::default();
    Ok(JsValue::undefined())
}

fn length(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = index(this)?;
    let mut host = host(context).borrow_mut();
    host.spend(1, 0, 0)?;
    Ok(host.storage[index].entries.len().into())
}

fn key(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = index(this)?;
    let value = args.first().cloned().unwrap_or_default();
    if value.is_object() || value.is_symbol() || value.is_bigint() {
        return Err(JsNativeError::typ()
            .with_message("Storage key index must be a primitive")
            .into());
    }
    let key = value.to_u32(context)? as usize;
    let mut host = host(context).borrow_mut();
    host.spend(1, 0, 0)?;
    Ok(host.storage[index]
        .entries
        .get(key)
        .map_or_else(JsValue::null, |(key, _)| {
            JsString::from(key.as_str()).into()
        }))
}
