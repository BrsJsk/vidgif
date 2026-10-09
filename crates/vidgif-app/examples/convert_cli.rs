//! Headless smoke test: `cargo run --release --example convert_cli -- in.mp4 out.gif [width]`
use std::sync::{atomic::AtomicBool, mpsc::channel};
use vidgif_app::platform::{ffmpeg, jobs, ConvertMsg};

fn main() -> anyhow::Result<()> {
    let mut a = std::env::args().skip(1);
    let (input, output) = (a.next().expect("input"), a.next().expect("output"));
    let width: u32 = a.next().map(|w| w.parse().unwrap()).unwrap_or(480);
    let (info, _) = ffmpeg::probe(input.as_ref())?;
    println!("{}x{} {:.2}s", info.width, info.height, info.duration);
    let mut s = vidgif_core::ConvertSettings::new(info.duration, info.width, info.height);
    s.width = width;
    let cancel = AtomicBool::new(false);
    let t0 = std::time::Instant::now();
    let est = jobs::estimate_size(input.as_ref(), &s, &cancel)?;
    println!("estimate: {} bytes in {:?}", est, t0.elapsed());
    let t0 = std::time::Instant::now();
    let (tx, rx) = channel();
    jobs::convert(input.as_ref(), &s, &cancel, &tx, || {});
    while let Ok(m) = rx.try_recv() {
        if let ConvertMsg::Done(b) = m {
            println!("actual:   {} bytes in {:?}", b.len(), t0.elapsed());
            std::fs::write(&output, b)?;
        } else if let ConvertMsg::Failed(e) = m {
            anyhow::bail!(e);
        }
    }
    Ok(())
}
