use crate::frame::RgbaFrame;
use crate::settings::Quality;
use color_quant::NeuQuant;
use gif::{DisposalMethod, Encoder, Frame, Repeat};
use std::borrow::Cow;

/// A frame reduced to a palette and per-pixel indices. This is the expensive part of encoding and
/// doesn't depend on neighbouring frames, so callers can run [`quantize`] on many frames at once.
pub struct Quantized {
    pal: Vec<[u8; 3]>,
    idx: Vec<u8>,
}

/// NeuQuant palette (`colors - 1` entries; the last slot is kept free for transparency) plus
/// optional Floyd–Steinberg dithering.
pub fn quantize(frame: &RgbaFrame, quality: Quality) -> Quantized {
    let (w, h) = (frame.width as usize, frame.height as usize);
    let ncolors = quality.colors() - 1;
    let nq = NeuQuant::new(quality.sample_factor(w * h), ncolors, &frame.data);
    let pal: Vec<[u8; 3]> = nq
        .color_map_rgba()
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| [c[0], c[1], c[2]])
        .collect();

    // NeuQuant's nearest-colour search is slow per pixel, so cache it on a 5-bit/channel grid
    // (32 KB, fits in L1). Dithering feeds the rounding error forward, so the coarse grid costs
    // little quality. Palette indices are < 255; 255 marks an empty slot.
    let mut lut = [255u8; 1 << 15];
    let mut lookup = |r: u8, g: u8, b: u8| -> u8 {
        let key = ((r as usize >> 3) << 10) | ((g as usize >> 3) << 5) | (b as usize >> 3);
        let v = lut[key];
        if v != 255 {
            return v;
        }
        let v = nq.index_of(&[r | 4, g | 4, b | 4, 255]) as u8;
        lut[key] = v;
        v
    };

    let mut idx = vec![0u8; w * h];
    if quality.dither() {
        // Floyd–Steinberg in 1/16 fixed point; the error rows have a 1px border so edges need no branches.
        let mut cur = vec![[0i32; 3]; w + 2];
        let mut next = vec![[0i32; 3]; w + 2];
        for (src_row, idx_row) in frame.data.chunks_exact(w * 4).zip(idx.chunks_exact_mut(w)) {
            for (x, (px, out)) in src_row
                .as_chunks::<4>()
                .0
                .iter()
                .zip(idx_row.iter_mut())
                .enumerate()
            {
                let e = cur[x + 1];
                let v = [
                    ((px[0] as i32) * 16 + e[0]).clamp(0, 4080),
                    ((px[1] as i32) * 16 + e[1]).clamp(0, 4080),
                    ((px[2] as i32) * 16 + e[2]).clamp(0, 4080),
                ];
                let k = lookup((v[0] >> 4) as u8, (v[1] >> 4) as u8, (v[2] >> 4) as u8);
                *out = k;
                let c = pal[k as usize];
                for ch in 0..3 {
                    let err = v[ch] - ((c[ch] as i32) << 4);
                    cur[x + 2][ch] += (err * 7) >> 4;
                    next[x][ch] += (err * 3) >> 4;
                    next[x + 1][ch] += (err * 5) >> 4;
                    next[x + 2][ch] += err >> 4;
                }
            }
            std::mem::swap(&mut cur, &mut next);
            next.fill([0; 3]);
        }
    } else {
        for (k, px) in idx.iter_mut().zip(frame.data.as_chunks::<4>().0) {
            *k = lookup(px[0], px[1], px[2]);
        }
    }
    Quantized { pal, idx }
}

/// Writes a GIF with a palette per frame. Pixels that didn't change since the previous frame
/// become transparent and each frame is cropped to the area that did.
///
/// Per frame: [`quantize`] (parallelisable), then [`Self::prepare`] (diff, sequential),
/// `Frame::make_lzw_pre_encoded` (parallelisable), then [`Self::write_compressed`] (in order).
pub struct NeuQuantEncoder {
    enc: Encoder<Vec<u8>>,
    width: u32,
    height: u32,
    delay: u16,
    quality: Quality,
    /// RGB a viewer currently sees, after all frames written so far.
    canvas: Option<Vec<u8>>,
    frames: usize,
}

impl NeuQuantEncoder {
    pub fn new(
        width: u32,
        height: u32,
        delay_cs: u16,
        quality: Quality,
        looping: bool,
    ) -> Result<Self, gif::EncodingError> {
        let mut enc = Encoder::new(Vec::new(), width as u16, height as u16, &[])?;
        // `Finite(0)` makes the gif crate omit the loop extension, so the GIF plays once.
        enc.set_repeat(if looping {
            Repeat::Infinite
        } else {
            Repeat::Finite(0)
        })?;
        Ok(Self {
            enc,
            width,
            height,
            delay: delay_cs,
            quality,
            canvas: None,
            frames: 0,
        })
    }

    pub fn quality(&self) -> Quality {
        self.quality
    }

    /// Convenience for one frame at a time: quantize, diff, compress and write.
    pub fn push(&mut self, frame: &RgbaFrame) -> Result<(), gif::EncodingError> {
        let qz = quantize(frame, self.quality);
        let mut out = self.prepare(frame, qz);
        out.make_lzw_pre_encoded();
        self.write_compressed(&out)
    }

    pub fn size_so_far(&self) -> usize {
        self.enc.get_ref().len()
    }

    pub fn finish(self) -> Result<Vec<u8>, gif::EncodingError> {
        self.enc.into_inner()
    }

    /// Write a frame that went through [`Self::prepare`] and `Frame::make_lzw_pre_encoded`.
    /// Frames must be written in playback order.
    pub fn write_compressed(&mut self, frame: &Frame<'_>) -> Result<(), gif::EncodingError> {
        self.enc.write_lzw_pre_encoded_frame(frame)
    }

    /// Diff `qz` (the quantized `frame`) against what a viewer currently sees and build the GIF
    /// frame. Depends on every earlier `prepare` call, so it must run in playback order.
    pub fn prepare(&mut self, frame: &RgbaFrame, qz: Quantized) -> Frame<'static> {
        debug_assert_eq!((frame.width, frame.height), (self.width, self.height));
        let (w, h) = (self.width as usize, self.height as usize);
        let transparent = (self.quality.colors() - 1) as u8;
        let Quantized { pal, idx } = qz;

        let tol = self.quality.diff_tolerance();
        let canvas = self.canvas.get_or_insert_with(|| vec![0; w * h * 3]);
        let first = self.frames == 0;
        let is_changed = |src: &[u8], shown: &[u8]| first || !within_tolerance(src, shown, tol);

        // Per-row span of changed pixels.
        let spans = map_rows(h, |y| {
            let src = &frame.data[y * w * 4..(y + 1) * w * 4];
            let shown = &canvas[y * w * 3..(y + 1) * w * 3];
            let changed_at =
                |x: usize| is_changed(&src[x * 4..x * 4 + 3], &shown[x * 3..x * 3 + 3]);
            let lo = (0..w).find(|&x| changed_at(x))?;
            let hi = (lo..w).rev().find(|&x| changed_at(x))?;
            Some((lo, hi))
        });
        let y0 = spans.iter().position(Option::is_some);
        let (x0, y0, x1, y1) = match y0 {
            None => (1, 1, 0, 0),
            Some(y0) => {
                let y1 = spans.iter().rposition(Option::is_some).unwrap_or(y0);
                let x0 = spans.iter().flatten().map(|s| s.0).min().unwrap_or(0);
                let x1 = spans.iter().flatten().map(|s| s.1).max().unwrap_or(0);
                (x0, y0, x1, y1)
            }
        };

        let mut palette: Vec<u8> = Vec::with_capacity(256 * 3);
        for c in &pal {
            palette.extend_from_slice(c);
        }
        palette.resize(transparent as usize * 3, 0);
        palette.extend_from_slice(&[0, 0, 0]); // transparent slot

        let out = if x0 > x1 {
            // Nothing changed: a 1x1 transparent frame keeps the timing.
            Frame {
                width: 1,
                height: 1,
                delay: self.delay,
                transparent: Some(transparent),
                dispose: DisposalMethod::Keep,
                palette: Some(palette),
                buffer: Cow::Owned(vec![transparent]),
                ..Frame::default()
            }
        } else {
            let (fw, fh) = (x1 - x0 + 1, y1 - y0 + 1);
            let mut buf = vec![transparent; fw * fh];
            // Copy changed pixels into the frame and the shown canvas.
            fill_rows(
                &mut buf,
                fw,
                &mut canvas[y0 * w * 3..(y1 + 1) * w * 3],
                w * 3,
                |ry, out, shown| {
                    let y = y0 + ry;
                    let src = &frame.data[y * w * 4..(y + 1) * w * 4];
                    for (dx, o) in out.iter_mut().enumerate() {
                        let x = x0 + dx;
                        let px = &mut shown[x * 3..x * 3 + 3];
                        if is_changed(&src[x * 4..x * 4 + 3], px) {
                            let k = idx[y * w + x];
                            px.copy_from_slice(&pal[k as usize]);
                            *o = k;
                        }
                    }
                },
            );
            Frame {
                left: x0 as u16,
                top: y0 as u16,
                width: fw as u16,
                height: fh as u16,
                delay: self.delay,
                transparent: Some(transparent),
                dispose: DisposalMethod::Keep,
                palette: Some(palette),
                buffer: Cow::Owned(buf),
                ..Frame::default()
            }
        };
        self.frames += 1;
        out
    }
}

/// `(0..n).map(f).collect()`, parallel with the `parallel` feature.
#[cfg(feature = "parallel")]
fn map_rows<R: Send>(n: usize, f: impl Fn(usize) -> R + Sync + Send) -> Vec<R> {
    use rayon::prelude::*;
    (0..n).into_par_iter().map(f).collect()
}

#[cfg(not(feature = "parallel"))]
fn map_rows<R>(n: usize, f: impl Fn(usize) -> R) -> Vec<R> {
    (0..n).map(f).collect()
}

/// Calls `f(row, out_row, canvas_row)` over matching rows of `out` and `canvas`, parallel with
/// the `parallel` feature.
#[cfg(feature = "parallel")]
fn fill_rows(
    out: &mut [u8],
    row_len: usize,
    canvas: &mut [u8],
    canvas_stride: usize,
    f: impl Fn(usize, &mut [u8], &mut [u8]) + Sync + Send,
) {
    use rayon::prelude::*;
    out.par_chunks_mut(row_len)
        .zip(canvas.par_chunks_mut(canvas_stride))
        .enumerate()
        .for_each(|(y, (o, c))| f(y, o, c));
}

#[cfg(not(feature = "parallel"))]
fn fill_rows(
    out: &mut [u8],
    row_len: usize,
    canvas: &mut [u8],
    canvas_stride: usize,
    f: impl Fn(usize, &mut [u8], &mut [u8]),
) {
    for (y, (o, c)) in out
        .chunks_mut(row_len)
        .zip(canvas.chunks_mut(canvas_stride))
        .enumerate()
    {
        f(y, o, c);
    }
}

fn within_tolerance(a: &[u8], b: &[u8], tol: u8) -> bool {
    a[0].abs_diff(b[0]) <= tol && a[1].abs_diff(b[1]) <= tol && a[2].abs_diff(b[2]) <= tol
}
