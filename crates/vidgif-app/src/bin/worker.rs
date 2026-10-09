//! GIF encoder that runs inside a Web Worker, so the UI thread never blocks on quantizing.
//! See `platform/web/encoder_client.rs` for the message protocol.

#[cfg(target_arch = "wasm32")]
fn main() {
    use js_sys::{Object, Reflect, Uint8Array};
    use std::cell::RefCell;
    use vidgif_core::{NeuQuantEncoder, Quality, RgbaFrame};
    use wasm_bindgen::prelude::*;
    use wasm_bindgen::JsCast;
    use web_sys::{DedicatedWorkerGlobalScope, MessageEvent};

    fn get(data: &JsValue, key: &str) -> JsValue {
        Reflect::get(data, &key.into()).unwrap_or(JsValue::UNDEFINED)
    }

    fn reply(scope: &DedicatedWorkerGlobalScope, kind: &str, fields: &[(&str, JsValue)]) {
        let message = Object::new();
        let _ = Reflect::set(&message, &"kind".into(), &kind.into());
        for (key, value) in fields {
            let _ = Reflect::set(&message, &(*key).into(), value);
        }
        let _ = scope.post_message(&message);
    }

    let scope: DedicatedWorkerGlobalScope = js_sys::global().unchecked_into();
    let encoder: RefCell<Option<(NeuQuantEncoder, u32, u32)>> = RefCell::new(None);

    let reply_scope = scope.clone();
    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        let data = event.data();
        let fail = |message: String| reply(&reply_scope, "error", &[("message", message.into())]);
        match get(&data, "kind").as_string().as_deref() {
            Some("start") => {
                let number = |key| get(&data, key).as_f64().unwrap_or(0.0);
                let quality =
                    Quality::ALL[(number("quality") as usize).min(Quality::ALL.len() - 1)];
                let looping = get(&data, "looping").as_bool().unwrap_or(true);
                let (w, h) = (number("width") as u32, number("height") as u32);
                match NeuQuantEncoder::new(w, h, number("delay") as u16, quality, looping) {
                    Ok(enc) => *encoder.borrow_mut() = Some((enc, w, h)),
                    Err(e) => fail(e.to_string()),
                }
            }
            Some("frame") => {
                let mut slot = encoder.borrow_mut();
                let Some((enc, w, h)) = slot.as_mut() else {
                    return fail("frame received before start".into());
                };
                let pixels = Uint8Array::new(&get(&data, "data")).to_vec();
                if pixels.len() != (*w * *h * 4) as usize {
                    return fail("frame has the wrong size".into());
                }
                match enc.push(&RgbaFrame::new(*w, *h, pixels)) {
                    Ok(()) => {
                        let size = JsValue::from_f64(enc.size_so_far() as f64);
                        reply(&reply_scope, "ack", &[("size", size)]);
                    }
                    Err(e) => fail(e.to_string()),
                }
            }
            Some("finish") => match encoder.borrow_mut().take() {
                Some((enc, ..)) => match enc.finish() {
                    Ok(bytes) => {
                        let bytes = Uint8Array::from(bytes.as_slice()).buffer();
                        reply(&reply_scope, "done", &[("bytes", bytes.into())]);
                    }
                    Err(e) => fail(e.to_string()),
                },
                None => fail("finish received before start".into()),
            },
            _ => {}
        }
    });
    scope.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    on_message.forget();
    reply(&scope, "ready", &[]);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!("the encoder worker only exists for the wasm32 target");
}
