/// Crop rectangle in source-video pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CropRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl CropRect {
    pub fn full(w: u32, h: u32) -> Self {
        Self { x: 0, y: 0, w, h }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    Small,
    Balanced,
    High,
}

impl Quality {
    pub const ALL: [Quality; 3] = [Quality::Small, Quality::Balanced, Quality::High];

    pub fn label(self) -> &'static str {
        match self {
            Quality::Small => "Small",
            Quality::Balanced => "Balanced",
            Quality::High => "High",
        }
    }

    /// Palette size, including the slot reserved for transparency.
    pub fn colors(self) -> usize {
        match self {
            Quality::Small => 64,
            Quality::Balanced => 128,
            Quality::High => 256,
        }
    }

    pub fn dither(self) -> bool {
        !matches!(self, Quality::Small)
    }

    /// NeuQuant sampling factor (1 = best/slowest, 30 = fastest). NeuQuant's cost grows with the
    /// number of sampled pixels, and palette quality saturates, so aim for a fixed sample count
    /// per quality level instead of a fixed stride.
    pub fn sample_factor(self, pixels: usize) -> i32 {
        let target_samples = match self {
            Quality::Small => 12_000,
            Quality::Balanced => 24_000,
            Quality::High => 80_000,
        };
        (pixels / target_samples).clamp(1, 30) as i32
    }

    /// Largest per-channel difference at which a pixel still counts as unchanged.
    pub fn diff_tolerance(self) -> u8 {
        match self {
            Quality::Small => 10,
            Quality::Balanced => 6,
            Quality::High => 3,
        }
    }
}

/// Frame rates that map to a whole number of centiseconds per frame.
pub const FPS_OPTIONS: [f32; 8] = [5.0, 10.0, 12.5, 100.0 / 6.0, 20.0, 25.0, 100.0 / 3.0, 50.0];

#[derive(Clone, Debug, PartialEq)]
pub struct ConvertSettings {
    /// Trim range, in source seconds.
    pub start: f64,
    pub end: f64,
    pub crop: CropRect,
    pub speed: f64,
    pub fps: f32,
    pub width: u32,
    pub quality: Quality,
    pub looping: bool,
}

impl ConvertSettings {
    /// Output size: the crop scaled to `width`, aspect ratio kept.
    pub fn output_size(&self) -> (u32, u32) {
        let crop = self.crop;
        let w = self.width.clamp(1, crop.w.max(1));
        let h = (w as f64 * crop.h as f64 / crop.w.max(1) as f64).round() as u32;
        (w, h.max(1))
    }

    pub fn new(duration: f64, src_w: u32, src_h: u32) -> Self {
        Self {
            start: 0.0,
            end: duration,
            crop: CropRect::full(src_w, src_h),
            speed: 1.0,
            fps: 100.0 / 6.0,
            width: src_w.min(480),
            quality: Quality::Balanced,
            looping: true,
        }
    }
}
