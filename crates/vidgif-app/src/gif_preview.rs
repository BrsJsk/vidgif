use egui::{ColorImage, Context, TextureHandle, TextureOptions};
use std::time::Duration;
use vidgif_core::RgbaFrame;

pub fn to_image(f: &RgbaFrame) -> ColorImage {
    ColorImage::from_rgba_unmultiplied([f.width as usize, f.height as usize], &f.data)
}

/// The finished GIF, decoded into one texture per frame so it can be played back in the UI.
pub struct GifPreview {
    pub bytes: Vec<u8>,
    frames: Vec<TextureHandle>,
    delays: Vec<Duration>,
    total: Duration,
}

impl GifPreview {
    pub fn decode(ctx: &Context, bytes: Vec<u8>) -> Result<Self, gif::DecodingError> {
        let mut opts = gif::DecodeOptions::new();
        opts.set_color_output(gif::ColorOutput::RGBA);
        let mut decoder = opts.read_info(bytes.as_slice())?;
        let (w, h) = (decoder.width() as usize, decoder.height() as usize);
        // Our frames are partial and use transparency, so rebuild the full image as we go.
        let mut canvas = vec![0u8; w * h * 4];
        let (mut frames, mut delays) = (Vec::new(), Vec::new());
        while let Some(frame) = decoder.read_next_frame()? {
            let (left, top) = (frame.left as usize, frame.top as usize);
            let fw = frame.width as usize;
            for (row, pixels) in frame.buffer.chunks_exact(fw * 4).enumerate() {
                for (col, px) in pixels.as_chunks::<4>().0.iter().enumerate() {
                    let (x, y) = (left + col, top + row);
                    if px[3] != 0 && x < w && y < h {
                        let at = (y * w + x) * 4;
                        canvas[at..at + 4].copy_from_slice(px);
                    }
                }
            }
            let image = ColorImage::from_rgba_unmultiplied([w, h], &canvas);
            let name = format!("gif-frame-{}", frames.len());
            frames.push(ctx.load_texture(name, image, TextureOptions::LINEAR));
            delays.push(Duration::from_millis(frame.delay as u64 * 10));
        }
        let total = delays
            .iter()
            .sum::<Duration>()
            .max(Duration::from_millis(10));
        Ok(Self {
            bytes,
            frames,
            delays,
            total,
        })
    }

    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    /// The frame to show `elapsed` after playback started, and how long until the next one.
    pub fn frame_at(&self, elapsed: Duration) -> (&TextureHandle, Duration) {
        let mut t = Duration::from_nanos((elapsed.as_nanos() % self.total.as_nanos()) as u64);
        let mut index = 0;
        for (i, delay) in self.delays.iter().enumerate() {
            index = i;
            if t < *delay {
                break;
            }
            t -= *delay;
        }
        let remaining = self.delays[index].saturating_sub(t);
        (&self.frames[index], remaining.max(Duration::from_millis(5)))
    }
}
