//! Decoding through an `ffmpeg` executable. ffmpeg does the trim, speed, fps, crop and scale in
//! one pass and streams raw RGBA frames.

use crate::platform::VideoInfo;
use anyhow::{anyhow, bail, Context, Result};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use vidgif_core::{timing::delay_cs, ConvertSettings, RgbaFrame};

/// Looks for ffmpeg in `$VIDGIF_FFMPEG`, next to our executable, on `PATH`, then in the usual
/// install dirs (apps launched from Finder don't get the shell's `PATH`).
pub fn find_ffmpeg() -> Result<PathBuf> {
    let exe = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(p) = std::env::var_os("VIDGIF_FFMPEG") {
        candidates.push(p.into());
    }
    if let Ok(me) = std::env::current_exe() {
        if let Some(dir) = me.parent() {
            candidates.push(dir.join(exe));
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|d| d.join(exe)));
    }
    for d in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"] {
        candidates.push(Path::new(d).join(exe));
    }
    candidates.into_iter().find(|p| p.is_file()).ok_or_else(|| {
        anyhow!("ffmpeg not found. Install it (e.g. `brew install ffmpeg`) or set VIDGIF_FFMPEG.")
    })
}

fn ffmpeg() -> Result<Command> {
    let mut c = Command::new(find_ffmpeg()?);
    c.args(["-hide_banner", "-loglevel", "error", "-nostdin"]);
    c.stdin(Stdio::null());
    Ok(c)
}

/// Duration comes from `ffmpeg -i`; the size comes from a decoded frame, so rotation is accounted for.
pub fn probe(path: &Path) -> Result<(VideoInfo, RgbaFrame)> {
    let out = Command::new(find_ffmpeg()?)
        .args(["-hide_banner", "-nostdin", "-i"])
        .arg(path)
        .stdin(Stdio::null())
        .output()?;
    let text = String::from_utf8_lossy(&out.stderr);
    let duration = text
        .split("Duration: ")
        .nth(1)
        .and_then(|rest| parse_hms(rest.split(',').next()?))
        .ok_or_else(|| anyhow!("could not read video duration (is this a video file?)"))?;
    if !text.contains("Video:") {
        bail!("no video stream found");
    }
    let frame = first_frame_png(path)?;
    Ok((
        VideoInfo {
            duration,
            width: frame.width,
            height: frame.height,
        },
        frame,
    ))
}

/// Parses `HH:MM:SS.xx` into seconds.
fn parse_hms(s: &str) -> Option<f64> {
    let mut parts = s.trim().split(':').map(|p| p.parse::<f64>().ok());
    Some(parts.next()?? * 3600.0 + parts.next()?? * 60.0 + parts.next()??)
}

/// First frame at full size. PNG so the dimensions come with the data.
fn first_frame_png(path: &Path) -> Result<RgbaFrame> {
    let out = ffmpeg()?
        .arg("-i")
        .arg(path)
        .args(["-frames:v", "1", "-f", "image2pipe", "-vcodec", "png", "-"])
        .output()?;
    if out.stdout.is_empty() {
        bail!(
            "could not decode the first frame: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let img = image::load_from_memory_with_format(&out.stdout, image::ImageFormat::Png)?.to_rgba8();
    Ok(RgbaFrame::new(img.width(), img.height(), img.into_raw()))
}

/// The frame at `t` seconds, scaled to `size`.
pub fn frame_at(path: &Path, t: f64, size: (u32, u32)) -> Result<RgbaFrame> {
    let (w, h) = size;
    let out = ffmpeg()?
        .arg("-ss")
        .arg(format!("{t:.3}"))
        .arg("-i")
        .arg(path)
        .args(["-frames:v", "1", "-vf"])
        .arg(format!("scale={w}:{h}:flags=area,format=rgba"))
        .args(["-f", "rawvideo", "-pix_fmt", "rgba", "-"])
        .output()?;
    if out.stdout.len() != (w * h * 4) as usize {
        bail!(
            "could not decode a frame at {t:.2}s: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(RgbaFrame::new(w, h, out.stdout))
}

/// Streams the frames described by `s`. `on_frame` returns `false` to stop early; `max_frames`
/// caps how many are read.
pub fn extract(
    path: &Path,
    s: &ConvertSettings,
    max_frames: Option<usize>,
    cancel: &AtomicBool,
    mut on_frame: impl FnMut(RgbaFrame) -> Result<bool>,
) -> Result<()> {
    let (w, h) = s.output_size();
    let fps_expr = format!("100/{}", delay_cs(s.fps));
    let vf = format!(
        "setpts=PTS/{speed},fps={fps_expr},crop={}:{}:{}:{},scale={w}:{h}:flags=area,format=rgba",
        s.crop.w,
        s.crop.h,
        s.crop.x,
        s.crop.y,
        speed = s.speed,
    );
    let mut cmd = ffmpeg()?;
    cmd.arg("-ss").arg(format!("{:.3}", s.start));
    cmd.arg("-t")
        .arg(format!("{:.3}", (s.end - s.start).max(0.05)));
    cmd.arg("-i").arg(path);
    cmd.args([
        "-an", "-sn", "-vf", &vf, "-f", "rawvideo", "-pix_fmt", "rgba", "-",
    ]);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().context("starting ffmpeg")?;
    let mut stdout = child.stdout.take().unwrap();
    // Read stderr on its own thread so ffmpeg can't block on a full pipe.
    let mut stderr = child.stderr.take().unwrap();
    let err_thread = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });

    let frame_bytes = (w * h * 4) as usize;
    let mut count = 0usize;
    let mut result = Ok(());
    loop {
        if cancel.load(Ordering::Relaxed) || max_frames.is_some_and(|m| count >= m) {
            break;
        }
        let mut buf = vec![0u8; frame_bytes];
        if let Err(e) = stdout.read_exact(&mut buf) {
            if e.kind() != std::io::ErrorKind::UnexpectedEof {
                result = Err(e.into());
            }
            break;
        }
        count += 1;
        match on_frame(RgbaFrame::new(w, h, buf)) {
            Ok(true) => {}
            Ok(false) => break,
            Err(e) => {
                result = Err(e);
                break;
            }
        }
    }
    drop(stdout);
    let _ = child.kill();
    let _ = child.wait();
    let err = err_thread.join().unwrap_or_default();
    result?;
    if count == 0 && !cancel.load(Ordering::Relaxed) {
        bail!("ffmpeg produced no frames: {}", err.trim());
    }
    Ok(())
}
