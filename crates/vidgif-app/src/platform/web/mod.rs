//! Browser: a hidden `<video>` element decodes (so no ffmpeg), encoding runs in a Web Worker,
//! and everything async is driven by `spawn_local`.

mod dom;
mod encoder_client;
mod grabber;

use super::{preview_size, ConvertMsg, Loaded, VideoInfo};
use dom::{document, js_error};
use encoder_client::EncoderWorker;
use grabber::Grabber;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use vidgif_core::{
    estimate::{self, SampleSize, SAMPLE_FRAMES},
    timing::sample_times,
    ConvertSettings, CropRect, RgbaFrame,
};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Blob, BlobPropertyBag, File, HtmlAnchorElement, HtmlInputElement, Url};

/// Browsers have far less memory and no threads to spare, so keep outputs smaller than on desktop.
pub const MAX_OUTPUT_WIDTH: u32 = 1280;

/// Frames that may be waiting in the worker while the next one is being grabbed.
const FRAMES_IN_FLIGHT: usize = 2;

pub type Input = File;

#[derive(Clone)]
pub struct Video {
    inner: Rc<Inner>,
}

struct Inner {
    name: String,
    url: String,
    /// Used for the scrubber preview; conversions open their own so they never fight over seeks.
    preview: Grabber,
}

impl Drop for Inner {
    fn drop(&mut self) {
        let _ = Url::revoke_object_url(&self.url);
    }
}

pub fn startup_error() -> Option<String> {
    None
}

pub fn pick_file(ctx: &egui::Context) -> Receiver<Input> {
    let (tx, rx) = channel();
    let input: HtmlInputElement = document()
        .create_element("input")
        .expect("create input")
        .unchecked_into();
    input.set_type("file");
    input.set_accept("video/*,.mkv,.mov,.m4v");
    let (picker, ctx) = (input.clone(), ctx.clone());
    let on_change = wasm_bindgen::closure::Closure::once_into_js(move || {
        if let Some(file) = picker.files().and_then(|files| files.get(0)) {
            let _ = tx.send(file);
            ctx.request_repaint();
        }
    });
    input.set_onchange(Some(on_change.unchecked_ref()));
    input.click();
    rx
}

pub fn dropped_input(ctx: &egui::Context) -> Option<Input> {
    ctx.input(|i| {
        i.raw
            .dropped_files
            .first()
            .and_then(|f| f.web_file().cloned())
    })
}

pub fn load(file: Input, ctx: &egui::Context) -> Receiver<Loaded> {
    let (tx, rx) = channel();
    let ctx = ctx.clone();
    spawn_local(async move {
        let _ = tx.send(open(file).await);
        ctx.request_repaint();
    });
    rx
}

async fn open(file: File) -> Loaded {
    let url = Url::create_object_url_with_blob(&file).map_err(js_error)?;
    let preview = Grabber::open(&url).await?;
    let (width, height) = preview.size();
    let info = VideoInfo {
        duration: preview.duration(),
        width,
        height,
    };
    let first = preview
        .grab(
            0.0,
            CropRect::full(width, height),
            preview_size(width, height),
        )
        .await?;
    let video = Video {
        inner: Rc::new(Inner {
            name: file.name(),
            url,
            preview,
        }),
    };
    Ok((video, info, first))
}

pub fn save_gif(file_name: &str, bytes: &[u8]) -> Option<String> {
    let result = (|| -> Result<(), wasm_bindgen::JsValue> {
        let options = BlobPropertyBag::new();
        options.set_type("image/gif");
        let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
        let blob = Blob::new_with_u8_array_sequence_and_options(&parts, &options)?;
        let url = Url::create_object_url_with_blob(&blob)?;
        let link: HtmlAnchorElement = document().create_element("a")?.unchecked_into();
        link.set_href(&url);
        link.set_download(file_name);
        link.click();
        Url::revoke_object_url(&url)
    })();
    Some(match result {
        Ok(()) => format!("Downloaded {file_name}"),
        Err(e) => format!("Download failed: {}", js_error(e)),
    })
}

impl Video {
    pub fn name(&self) -> String {
        self.inner.name.clone()
    }

    pub fn preview_frame(
        &self,
        time: f64,
        size: (u32, u32),
        ctx: &egui::Context,
    ) -> Receiver<Option<RgbaFrame>> {
        let (tx, rx) = channel();
        let (inner, ctx) = (self.inner.clone(), ctx.clone());
        spawn_local(async move {
            let (w, h) = inner.preview.size();
            let frame = inner.preview.grab(time, CropRect::full(w, h), size).await;
            let _ = tx.send(frame.ok());
            ctx.request_repaint();
        });
        rx
    }

    pub fn estimate(
        &self,
        settings: &ConvertSettings,
        cancel: Arc<AtomicBool>,
        ctx: &egui::Context,
    ) -> Receiver<Option<u64>> {
        let (tx, rx) = channel();
        let (url, settings, ctx) = (self.inner.url.clone(), settings.clone(), ctx.clone());
        let keep_alive = self.inner.clone();
        spawn_local(async move {
            let bytes = estimate_size(&url, &settings, &cancel).await.ok().flatten();
            drop(keep_alive);
            let _ = tx.send(bytes);
            ctx.request_repaint();
        });
        rx
    }

    pub fn convert(
        &self,
        settings: &ConvertSettings,
        cancel: Arc<AtomicBool>,
        ctx: &egui::Context,
    ) -> Receiver<ConvertMsg> {
        let (tx, rx) = channel();
        let (url, settings, ctx) = (self.inner.url.clone(), settings.clone(), ctx.clone());
        let keep_alive = self.inner.clone();
        spawn_local(async move {
            let outcome = run_convert(&url, &settings, &cancel, &tx, &ctx).await;
            drop(keep_alive);
            let _ = tx.send(match outcome {
                _ if cancel.load(Ordering::Relaxed) => ConvertMsg::Cancelled,
                Ok(bytes) => ConvertMsg::Done(bytes),
                Err(e) => ConvertMsg::Failed(e),
            });
            ctx.request_repaint();
        });
        rx
    }
}

async fn run_convert(
    url: &str,
    settings: &ConvertSettings,
    cancel: &AtomicBool,
    tx: &Sender<ConvertMsg>,
    ctx: &egui::Context,
) -> Result<Vec<u8>, String> {
    let grabber = Grabber::open(url).await?;
    let mut worker = EncoderWorker::spawn().await?;
    worker.start(settings)?;
    let out_size = settings.output_size();
    let times = sample_times(settings);
    let total = times.len();
    for (i, time) in times.into_iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        let frame = grabber.grab(time, settings.crop, out_size).await?;
        worker.send_frame(frame)?;
        worker.wait_backlog(FRAMES_IN_FLIGHT).await?;
        let _ = tx.send(ConvertMsg::Progress { done: i + 1, total });
        ctx.request_repaint();
    }
    worker.finish().await
}

async fn estimate_size(
    url: &str,
    settings: &ConvertSettings,
    cancel: &AtomicBool,
) -> Result<Option<u64>, String> {
    let grabber = Grabber::open(url).await?;
    let mut worker = EncoderWorker::spawn().await?;
    let out_size = settings.output_size();
    let mut samples = Vec::new();
    for window in estimate::sample_windows(settings) {
        worker.start(&window)?;
        let (mut first, mut last, mut frames) = (0, 0, 0u64);
        for time in sample_times(&window).into_iter().take(SAMPLE_FRAMES) {
            if cancel.load(Ordering::Relaxed) {
                return Ok(None);
            }
            worker.send_frame(grabber.grab(time, window.crop, out_size).await?)?;
            last = worker.wait_backlog(0).await?;
            frames += 1;
            if frames == 1 {
                first = last;
            }
        }
        samples.push(SampleSize {
            first: first as u64,
            rest: (last - first) as u64,
            rest_frames: frames.saturating_sub(1),
        });
    }
    Ok(Some(estimate::estimate_bytes(settings, &samples)))
}
