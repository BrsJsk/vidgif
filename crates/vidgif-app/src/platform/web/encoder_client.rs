//! Main-thread side of the encoder Web Worker (`src/bin/worker.rs`).
//!
//! Messages to the worker are `{kind: "start", width, height, delay, quality, looping}`,
//! `{kind: "frame", data: ArrayBuffer}` and `{kind: "finish"}`. It answers with `ready` once its
//! wasm has loaded, `ack` (plus the encoded size so far) after every frame, then `done` with the
//! GIF bytes, or `error`.

use super::dom::js_error;
use std::cell::RefCell;
use std::future::poll_fn;
use std::rc::Rc;
use std::task::{Poll, Waker};
use vidgif_core::{timing::delay_cs, ConvertSettings, Quality, RgbaFrame};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{MessageEvent, Worker};

const WORKER_URL: &str = "./worker-loader.js";

#[derive(Default)]
struct Shared {
    ready: bool,
    acked: usize,
    size: usize,
    done: Option<Vec<u8>>,
    error: Option<String>,
    waker: Option<Waker>,
}

pub struct EncoderWorker {
    worker: Worker,
    shared: Rc<RefCell<Shared>>,
    sent: usize,
    _on_message: Closure<dyn FnMut(MessageEvent)>,
}

fn get(data: &JsValue, key: &str) -> JsValue {
    js_sys::Reflect::get(data, &key.into()).unwrap_or(JsValue::UNDEFINED)
}

impl EncoderWorker {
    pub async fn spawn() -> Result<Self, String> {
        let worker = Worker::new(WORKER_URL).map_err(js_error)?;
        let shared = Rc::new(RefCell::new(Shared::default()));

        let state = shared.clone();
        let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
            let data = event.data();
            let mut s = state.borrow_mut();
            match get(&data, "kind").as_string().as_deref() {
                Some("ready") => s.ready = true,
                Some("ack") => {
                    s.acked += 1;
                    s.size = get(&data, "size").as_f64().unwrap_or(0.0) as usize;
                }
                Some("done") => {
                    s.done = Some(js_sys::Uint8Array::new(&get(&data, "bytes")).to_vec());
                }
                Some("error") => {
                    s.error = Some(get(&data, "message").as_string().unwrap_or_default());
                }
                _ => {}
            }
            if let Some(waker) = s.waker.take() {
                waker.wake();
            }
        });
        worker.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        let on_error = Closure::<dyn FnMut(web_sys::Event)>::new({
            let state = shared.clone();
            move |_| {
                let mut s = state.borrow_mut();
                s.error = Some("The encoder worker failed to start.".into());
                if let Some(waker) = s.waker.take() {
                    waker.wake();
                }
            }
        });
        worker.set_onerror(Some(on_error.as_ref().unchecked_ref()));
        on_error.forget();

        let this = Self {
            worker,
            shared,
            sent: 0,
            _on_message: on_message,
        };
        this.wait(|s| s.ready.then_some(())).await?;
        Ok(this)
    }

    /// Waits until `ready` yields a value, failing if the worker reports an error.
    async fn wait<T>(&self, ready: impl Fn(&Shared) -> Option<T>) -> Result<T, String> {
        poll_fn(|cx| {
            let mut s = self.shared.borrow_mut();
            if let Some(e) = &s.error {
                return Poll::Ready(Err(e.clone()));
            }
            match ready(&s) {
                Some(v) => Poll::Ready(Ok(v)),
                None => {
                    s.waker = Some(cx.waker().clone());
                    Poll::Pending
                }
            }
        })
        .await
    }

    fn post(
        &self,
        message: &js_sys::Object,
        transfer: Option<&js_sys::ArrayBuffer>,
    ) -> Result<(), String> {
        match transfer {
            Some(buffer) => self
                .worker
                .post_message_with_transfer(message, &js_sys::Array::of1(buffer)),
            None => self.worker.post_message(message),
        }
        .map_err(js_error)
    }

    fn message(kind: &str) -> js_sys::Object {
        let message = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&message, &"kind".into(), &kind.into());
        message
    }

    /// Begins a new GIF. Any earlier one is discarded.
    pub fn start(&self, s: &ConvertSettings) -> Result<(), String> {
        let (width, height) = s.output_size();
        let quality = Quality::ALL
            .iter()
            .position(|q| *q == s.quality)
            .unwrap_or(1);
        let message = Self::message("start");
        for (key, value) in [
            ("width", width as f64),
            ("height", height as f64),
            ("delay", delay_cs(s.fps) as f64),
            ("quality", quality as f64),
        ] {
            let _ = js_sys::Reflect::set(&message, &key.into(), &value.into());
        }
        let _ = js_sys::Reflect::set(&message, &"looping".into(), &s.looping.into());
        self.post(&message, None)
    }

    pub fn send_frame(&mut self, frame: RgbaFrame) -> Result<(), String> {
        let data = js_sys::Uint8Array::from(frame.data.as_slice()).buffer();
        let message = Self::message("frame");
        let _ = js_sys::Reflect::set(&message, &"data".into(), &data);
        self.post(&message, Some(&data))?;
        self.sent += 1;
        Ok(())
    }

    /// Waits until at most `limit` frames are still being encoded; returns the encoded size so far.
    pub async fn wait_backlog(&self, limit: usize) -> Result<usize, String> {
        let target = self.sent.saturating_sub(limit);
        self.wait(|s| (s.acked >= target).then_some(s.size)).await
    }

    pub async fn finish(&self) -> Result<Vec<u8>, String> {
        self.post(&Self::message("finish"), None)?;
        self.wait(|s| s.done.clone()).await
    }
}

impl Drop for EncoderWorker {
    fn drop(&mut self) {
        self.worker.terminate();
    }
}
