use crate::{
    settings::ConvertSettings,
    timing::{delay_cs, sample_times},
};

/// Frames encoded at each sample point.
pub const SAMPLE_FRAMES: usize = 3;
const MAX_SAMPLE_POINTS: usize = 5;

/// Rough size guess in bytes, used until a sampled estimate is ready.
pub fn heuristic_bytes(s: &ConvertSettings) -> u64 {
    let (w, h) = s.output_size();
    let frames = sample_times(s).len() as f64;
    // Empirically ~0.35 bytes per pixel per frame after diffing and LZW.
    (w as f64 * h as f64 * frames * 0.35) as u64
}

/// Short trim windows spread across the clip. Encoding [`SAMPLE_FRAMES`] frames of each and
/// feeding the sizes to [`estimate_bytes`] is much cheaper than encoding everything.
pub fn sample_windows(s: &ConvertSettings) -> Vec<ConvertSettings> {
    let total = sample_times(s).len();
    let step = s.speed * delay_cs(s.fps) as f64 / 100.0;
    let span = (s.end - s.start).max(0.0);
    let points = MAX_SAMPLE_POINTS.min(total.div_ceil(SAMPLE_FRAMES)).max(1);
    (0..points)
        .map(|k| {
            let at = s.start
                + (span - SAMPLE_FRAMES as f64 * step).max(0.0) * k as f64
                    / (points - 1).max(1) as f64;
            let mut window = s.clone();
            window.start = at.min(s.end);
            window.end = (window.start + (SAMPLE_FRAMES as f64 + 0.5) * step)
                .min(s.end.max(window.start + step));
            window
        })
        .collect()
}

/// Size of one encoded sample window: the first frame is full-size, the rest are mostly diffs.
pub struct SampleSize {
    pub first: u64,
    pub rest: u64,
    pub rest_frames: u64,
}

/// Extrapolates the size of the whole GIF from its sampled windows.
pub fn estimate_bytes(s: &ConvertSettings, samples: &[SampleSize]) -> u64 {
    if samples.is_empty() {
        return heuristic_bytes(s);
    }
    let total = sample_times(s).len() as u64;
    let first_avg = samples.iter().map(|x| x.first).sum::<u64>() / samples.len() as u64;
    let (rest, rest_frames) = samples
        .iter()
        .fold((0, 0), |a, x| (a.0 + x.rest, a.1 + x.rest_frames));
    let rest_avg = rest.checked_div(rest_frames).unwrap_or(first_avg);
    first_avg + rest_avg * total.saturating_sub(1) + 20
}
