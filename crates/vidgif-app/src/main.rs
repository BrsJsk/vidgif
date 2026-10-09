#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    // Leave one core free so the UI stays responsive while encoding saturates the rest.
    let threads =
        std::thread::available_parallelism().map_or(1, |n| n.get().saturating_sub(1).max(1));
    let _ = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build_global();

    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/icon.png"))
        .expect("bundled icon is a valid PNG");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_icon(icon)
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([760.0, 520.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "vidgif",
        options,
        Box::new(|cc| Ok(Box::new(vidgif_app::app::App::new(cc)))),
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::wasm_bindgen::JsCast as _;

    let canvas = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("vidgif_canvas"))
        .and_then(|c| c.dyn_into::<web_sys::HtmlCanvasElement>().ok())
        .expect("index.html has a #vidgif_canvas element");

    wasm_bindgen_futures::spawn_local(async move {
        let started = eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|cc| Ok(Box::new(vidgif_app::app::App::new(cc)))),
            )
            .await;
        let loading = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("loading"));
        if let Some(el) = loading {
            match started {
                Ok(()) => el.remove(),
                Err(e) => el.set_text_content(Some(&format!("vidgif failed to start: {e:?}"))),
            }
        }
    });
}
