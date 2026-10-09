use super::ffmpeg;
use crate::platform::ConvertMsg;
use anyhow::Result;
use rayon::prelude::*;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Sender};
use vidgif_core::{
    estimate::{self, SampleSize, SAMPLE_FRAMES},
    quantize,
    timing::{delay_cs, sample_times},
    ConvertSettings, NeuQuantEncoder, RgbaFrame,
};

fn new_encoder(s: &ConvertSettings) -> Result<NeuQuantEncoder> {
    let (w, h) = s.output_size();
    Ok(NeuQuantEncoder::new(
        w,
        h,
        delay_cs(s.fps),
        s.quality,
        s.looping,
    )?)
}

pub fn convert(
    path: &Path,
    s: &ConvertSettings,
    cancel: &AtomicBool,
    tx: &Sender<ConvertMsg>,
    on_update: impl Fn(),
) {
    let total = sample_times(s).len();
    let run = || -> Result<Vec<u8>> {
        let mut enc = new_encoder(s)?;
        let quality = enc.quality();
        let batch = rayon::current_num_threads().max(1) * 2;
        // ffmpeg decodes on a reader thread while earlier frames are quantized in parallel.
        // Only the diff and the final write have to happen in frame order.
        let (ftx, frx) = sync_channel::<RgbaFrame>(batch * 2);
        let mut done = 0;
        let (read_result, write_result) = std::thread::scope(|sc| {
            let reader = sc
                .spawn(move || ffmpeg::extract(path, s, None, cancel, |f| Ok(ftx.send(f).is_ok())));
            let write_result = (|| -> Result<()> {
                let mut frames: Vec<RgbaFrame> = Vec::with_capacity(batch);
                loop {
                    frames.clear();
                    if let Ok(f) = frx.recv() {
                        frames.push(f);
                    }
                    while frames.len() < batch {
                        match frx.try_recv() {
                            Ok(f) => frames.push(f),
                            Err(_) => break,
                        }
                    }
                    if frames.is_empty() {
                        return Ok(());
                    }
                    let quantized: Vec<_> =
                        frames.par_iter().map(|f| quantize(f, quality)).collect();
                    let mut prepared: Vec<_> = frames
                        .iter()
                        .zip(quantized)
                        .map(|(f, q)| enc.prepare(f, q))
                        .collect();
                    prepared
                        .par_iter_mut()
                        .for_each(|f| f.make_lzw_pre_encoded());
                    for f in &prepared {
                        enc.write_compressed(f)?;
                        done += 1;
                    }
                    let _ = tx.send(ConvertMsg::Progress { done, total });
                    on_update();
                }
            })();
            // Dropping the receiver makes the reader's `send` fail, which stops ffmpeg early on error.
            drop(frx);
            (reader.join().expect("reader thread panicked"), write_result)
        });
        write_result?;
        read_result?;
        Ok(enc.finish()?)
    };
    let msg = match run() {
        _ if cancel.load(Ordering::Relaxed) => ConvertMsg::Cancelled,
        Ok(bytes) => ConvertMsg::Done(bytes),
        Err(e) => ConvertMsg::Failed(format!("{e:#}")),
    };
    let _ = tx.send(msg);
    on_update();
}

/// Encodes a few frames at several points of the trim range and extrapolates the total size.
pub fn estimate_size(path: &Path, s: &ConvertSettings, cancel: &AtomicBool) -> Result<u64> {
    let samples: Vec<Option<SampleSize>> = estimate::sample_windows(s)
        .into_par_iter()
        .map(|window| -> Result<Option<SampleSize>> {
            if cancel.load(Ordering::Relaxed) {
                return Ok(None);
            }
            let mut enc = new_encoder(&window)?;
            let (mut first, mut frames) = (0, 0);
            ffmpeg::extract(path, &window, Some(SAMPLE_FRAMES), cancel, |f| {
                enc.push(&f)?;
                frames += 1;
                if frames == 1 {
                    first = enc.size_so_far();
                }
                Ok(true)
            })?;
            Ok((frames >= 1).then(|| SampleSize {
                first: first as u64,
                rest: (enc.size_so_far() - first) as u64,
                rest_frames: frames as u64 - 1,
            }))
        })
        .collect::<Result<_>>()?;
    let samples: Vec<_> = samples.into_iter().flatten().collect();
    Ok(estimate::estimate_bytes(s, &samples))
}
