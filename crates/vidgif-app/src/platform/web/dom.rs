use js_sys::{Function, Promise};
use std::cell::RefCell;
use std::future::Future;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::EventTarget;

type Callback = Closure<dyn FnMut()>;

pub fn document() -> web_sys::Document {
    web_sys::window()
        .and_then(|w| w.document())
        .expect("running in a browser window")
}

pub fn js_error(e: JsValue) -> String {
    e.as_string()
        .or_else(|| {
            js_sys::Reflect::get(&e, &"message".into())
                .ok()
                .and_then(|m| m.as_string())
        })
        .unwrap_or_else(|| format!("{e:?}"))
}

/// Starts listening right away (so no event can be missed between this call and the first
/// `.await`) and resolves with the name of the first of `names` that fires on `target`, or
/// `"timeout"` after `timeout_ms`.
pub fn first_event(
    target: &EventTarget,
    names: &[&str],
    timeout_ms: i32,
) -> impl Future<Output = String> {
    let window = web_sys::window().expect("window");
    let listeners: Rc<RefCell<Vec<(String, Callback)>>> = Rc::default();
    let timeout: Rc<RefCell<Option<Callback>>> = Rc::default();
    let mut timer = 0;

    let promise = Promise::new(&mut |resolve: Function, _reject: Function| {
        for name in names {
            let (resolve, fired) = (resolve.clone(), name.to_string());
            let callback = Callback::new(move || {
                let _ = resolve.call1(&JsValue::NULL, &JsValue::from_str(&fired));
            });
            target
                .add_event_listener_with_callback(name, callback.as_ref().unchecked_ref())
                .expect("add_event_listener");
            listeners.borrow_mut().push((name.to_string(), callback));
        }
        let resolve = resolve.clone();
        let callback = Callback::new(move || {
            let _ = resolve.call1(&JsValue::NULL, &JsValue::from_str("timeout"));
        });
        timer = window
            .set_timeout_with_callback_and_timeout_and_arguments_0(
                callback.as_ref().unchecked_ref(),
                timeout_ms,
            )
            .expect("set_timeout");
        *timeout.borrow_mut() = Some(callback);
    });

    let target = target.clone();
    async move {
        let fired = JsFuture::from(promise).await;
        window.clear_timeout_with_handle(timer);
        for (name, callback) in listeners.borrow().iter() {
            let _ =
                target.remove_event_listener_with_callback(name, callback.as_ref().unchecked_ref());
        }
        timeout.borrow_mut().take();
        fired.ok().and_then(|v| v.as_string()).unwrap_or_default()
    }
}
