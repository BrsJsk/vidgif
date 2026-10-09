//! Desktop: video decoding goes through an ffmpeg executable, work runs on threads.

pub mod ffmpeg;
pub mod jobs;

use super::{ConvertMsg, Loaded};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use vidgif_core::{ConvertSettings, RgbaFrame};

pub const MAX_OUTPUT_WIDTH: u32 = 1920;

pub type Input = PathBuf;

#[derive(Clone)]
pub struct Video {
    path: PathBuf,
}

/// Set when the app can't work at all, e.g. because ffmpeg is missing.
pub fn startup_error() -> Option<String> {
    ffmpeg::find_ffmpeg().err().map(|e| e.to_string())
}

pub fn pick_file(_ctx: &egui::Context) -> Receiver<Input> {
    let (tx, rx) = channel();
    let picked = rfd::FileDialog::new()
        .add_filter("Video", &["mp4", "mov", "m4v", "mkv", "webm", "avi", "gif"])
        .pick_file();
    if let Some(path) = picked {
        let _ = tx.send(path);
    }
    rx
}

pub fn dropped_input(ctx: &egui::Context) -> Option<Input> {
    ctx.input(|i| i.raw.dropped_files.first().map(|f| f.path().to_path_buf()))
}

pub fn load(input: Input, ctx: &egui::Context) -> Receiver<Loaded> {
    let (tx, rx) = channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let loaded = ffmpeg::probe(&input)
            .map(|(info, frame)| (Video { path: input }, info, frame))
            .map_err(|e| format!("{e:#}"));
        let _ = tx.send(loaded);
        ctx.request_repaint();
    });
    rx
}

/// Asks where to save and writes the file. Returns a status line, or `None` if cancelled.
pub fn save_gif(file_name: &str, bytes: &[u8]) -> Option<String> {
    let path = rfd::FileDialog::new()
        .set_file_name(file_name)
        .add_filter("GIF", &["gif"])
        .save_file()?;
    Some(match std::fs::write(&path, bytes) {
        Ok(()) => format!("Saved {}", path.display()),
        Err(e) => format!("Save failed: {e}"),
    })
}

impl Video {
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    }

    pub fn preview_frame(
        &self,
        time: f64,
        size: (u32, u32),
        ctx: &egui::Context,
    ) -> Receiver<Option<RgbaFrame>> {
        let (tx, rx) = channel();
        let (path, ctx) = (self.path.clone(), ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(ffmpeg::frame_at(&path, time, size).ok());
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
        let (path, settings, ctx) = (self.path.clone(), settings.clone(), ctx.clone());
        std::thread::spawn(move || {
            let bytes = jobs::estimate_size(&path, &settings, &cancel)
                .ok()
                .filter(|_| !cancel.load(Ordering::Relaxed));
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
        let (path, settings, ctx) = (self.path.clone(), settings.clone(), ctx.clone());
        std::thread::spawn(move || {
            jobs::convert(&path, &settings, &cancel, &tx, || ctx.request_repaint())
        });
        rx
    }
}
