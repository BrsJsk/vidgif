use crate::settings::ConvertSettings;

/// GIF frame delay in centiseconds for a frame rate.
pub fn delay_cs(fps: f32) -> u16 {
    (100.0 / fps).round().clamp(2.0, 100.0) as u16
}

/// Source timestamps to sample. A GIF frame lasts `delay_cs`, so at speed `s`
/// consecutive samples are `s * delay` source-seconds apart.
pub fn sample_times(s: &ConvertSettings) -> Vec<f64> {
    let step = s.speed * delay_cs(s.fps) as f64 / 100.0;
    let span = (s.end - s.start).max(0.0);
    let n = ((span / step).floor() as usize).max(1);
    (0..n).map(|i| s.start + i as f64 * step).collect()
}

pub fn output_duration(s: &ConvertSettings) -> f64 {
    sample_times(s).len() as f64 * delay_cs(s.fps) as f64 / 100.0
}
