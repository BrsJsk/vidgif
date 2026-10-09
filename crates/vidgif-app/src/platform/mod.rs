//! Everything that differs between the desktop and browser builds: picking and decoding videos,
//! running conversions, and saving the result. The UI only talks to this module's API:
//!
//! - `Video`: a loaded video. `preview_frame`, `estimate` and `convert` start background work
//!   and return a channel that is polled from the UI thread.
//! - `Input`, `pick_file`, `dropped_input`, `load`: getting a video in.
//! - `save_gif`, `startup_error`, `MAX_OUTPUT_WIDTH`.

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
pub use native::*;

#[cfg(target_arch = "wasm32")]
mod web;
#[cfg(target_arch = "wasm32")]
pub use web::*;

use vidgif_core::RgbaFrame;

pub struct VideoInfo {
    pub duration: f64,
    pub width: u32,
    pub height: u32,
}

pub enum ConvertMsg {
    Progress { done: usize, total: usize },
    Done(Vec<u8>),
    Failed(String),
    Cancelled,
}

pub type Loaded = Result<(Video, VideoInfo, RgbaFrame), String>;

/// Previews are capped at this width; crop coordinates stay in source pixels.
const PREVIEW_MAX_WIDTH: u32 = 1280;

pub fn preview_size(src_w: u32, src_h: u32) -> (u32, u32) {
    let w = src_w.clamp(1, PREVIEW_MAX_WIDTH);
    let h = ((src_h as f64 * w as f64 / src_w.max(1) as f64).round() as u32).max(1);
    (w, h)
}
