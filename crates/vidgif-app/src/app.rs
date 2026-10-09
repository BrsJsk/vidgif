use crate::crop::{centered_ratio, crop_editor, fit_rect};
use crate::gif_preview::{to_image, GifPreview};
use crate::platform::{self, ConvertMsg, Input, Loaded, Video, VideoInfo};
use egui::{Color32, CornerRadius, Pos2, Rect, TextureHandle, TextureOptions, Vec2};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::Duration;
use vidgif_core::{
    estimate,
    settings::FPS_OPTIONS,
    timing::{delay_cs, output_duration, sample_times},
    ConvertSettings, CropRect, Quality, RgbaFrame,
};
use web_time::Instant;

const MIN_OUTPUT_WIDTH: u32 = 160;
const LONG_GIF_SECS: f64 = 30.0;
/// How long settings must stay unchanged before the (costly) size estimate is recomputed.
const ESTIMATE_DELAY: Duration = Duration::from_millis(800);

/// The loaded video and everything the editor shows for it.
struct Editor {
    video: Video,
    info: VideoInfo,
    settings: ConvertSettings,
    preview: PreviewFrame,
    size: SizeEstimate,
}

/// The still frame behind the crop rectangle, and the scrubber position it should show.
struct PreviewFrame {
    wanted: f64,
    shown: f64,
    texture: TextureHandle,
    inflight: Option<(f64, Receiver<Option<RgbaFrame>>)>,
}

/// Output size estimate for the current settings, refreshed in the background.
struct SizeEstimate {
    bytes: Option<u64>,
    /// Settings `bytes` (or the running job) was computed for.
    computed_for: Option<ConvertSettings>,
    last_seen: ConvertSettings,
    changed_at: Instant,
    pending: Option<Receiver<Option<u64>>>,
    cancel: Arc<AtomicBool>,
}

impl SizeEstimate {
    fn new(settings: &ConvertSettings) -> Self {
        Self {
            bytes: None,
            computed_for: None,
            last_seen: settings.clone(),
            changed_at: Instant::now(),
            pending: None,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    fn update(&mut self, ctx: &egui::Context, video: &Video, settings: &ConvertSettings) {
        if *settings != self.last_seen {
            self.cancel.store(true, Ordering::Relaxed);
            self.last_seen = settings.clone();
            self.changed_at = Instant::now();
            self.bytes = None;
            self.computed_for = None;
            self.pending = None;
        }
        if let Some(rx) = &self.pending {
            if let Ok(result) = rx.try_recv() {
                self.bytes = result;
                self.pending = None;
            }
        }
        if self.computed_for.as_ref() == Some(settings) || self.pending.is_some() {
            return;
        }
        let wait = ESTIMATE_DELAY.saturating_sub(self.changed_at.elapsed());
        if !wait.is_zero() {
            ctx.request_repaint_after(wait);
            return;
        }
        self.computed_for = Some(settings.clone());
        self.cancel = Arc::new(AtomicBool::new(false));
        let rx = video.estimate(settings, self.cancel.clone(), ctx);
        self.pending = Some(rx);
    }
}

struct Job {
    cancel: Arc<AtomicBool>,
    rx: Receiver<ConvertMsg>,
    done: usize,
    total: usize,
}

enum Stage {
    Empty,
    Loading(Receiver<Loaded>),
    Edit,
    Converting(Job),
    Done { gif: GifPreview, started: Instant },
}

pub struct App {
    stage: Stage,
    editor: Option<Editor>,
    error: Option<String>,
    status: Option<String>,
    startup_error: Option<String>,
    picker: Option<Receiver<Input>>,
}

impl App {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self {
            stage: Stage::Empty,
            editor: None,
            error: None,
            status: None,
            startup_error: platform::startup_error(),
            picker: None,
        }
    }

    fn open(&mut self, ctx: &egui::Context, input: Input) {
        self.error = None;
        self.stage = Stage::Loading(platform::load(input, ctx));
    }

    fn poll_loading(&mut self, ctx: &egui::Context) {
        let Stage::Loading(rx) = &self.stage else {
            return;
        };
        let Ok(loaded) = rx.try_recv() else { return };
        match loaded {
            Ok((video, info, first_frame)) => {
                let settings = ConvertSettings::new(info.duration, info.width, info.height);
                let texture =
                    ctx.load_texture("preview", to_image(&first_frame), TextureOptions::LINEAR);
                self.editor = Some(Editor {
                    video,
                    size: SizeEstimate::new(&settings),
                    preview: PreviewFrame {
                        wanted: 0.0,
                        shown: 0.0,
                        texture,
                        inflight: None,
                    },
                    settings,
                    info,
                });
                self.stage = Stage::Edit;
            }
            Err(e) => {
                self.error = Some(e);
                self.stage = Stage::Empty;
            }
        }
    }

    fn start_convert(&mut self, ctx: &egui::Context) {
        let Some(editor) = &self.editor else { return };
        editor.size.cancel.store(true, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        let total = sample_times(&editor.settings).len();
        let rx = editor.video.convert(&editor.settings, cancel.clone(), ctx);
        self.error = None;
        self.stage = Stage::Converting(Job {
            cancel,
            rx,
            done: 0,
            total,
        });
    }

    fn poll_job(&mut self, ctx: &egui::Context) {
        let Stage::Converting(job) = &mut self.stage else {
            return;
        };
        let mut outcome = None;
        while let Ok(msg) = job.rx.try_recv() {
            match msg {
                ConvertMsg::Progress { done, total } => {
                    job.done = done;
                    job.total = total.max(done);
                }
                finished => outcome = Some(finished),
            }
        }
        let Some(outcome) = outcome else { return };
        self.stage = match outcome {
            ConvertMsg::Done(bytes) => match GifPreview::decode(ctx, bytes) {
                Ok(gif) => Stage::Done {
                    gif,
                    started: Instant::now(),
                },
                Err(e) => {
                    self.error = Some(format!("Could not preview the result: {e}"));
                    Stage::Edit
                }
            },
            ConvertMsg::Failed(e) => {
                self.error = Some(e);
                Stage::Edit
            }
            ConvertMsg::Cancelled | ConvertMsg::Progress { .. } => Stage::Edit,
        };
    }

    fn poll_editor(&mut self, ctx: &egui::Context) {
        let Some(editor) = &mut self.editor else {
            return;
        };
        let preview = &mut editor.preview;
        if let Some((time, rx)) = &preview.inflight {
            if let Ok(frame) = rx.try_recv() {
                if let Some(frame) = frame {
                    preview.texture =
                        ctx.load_texture("preview", to_image(&frame), TextureOptions::LINEAR);
                }
                preview.shown = *time;
                preview.inflight = None;
            }
        }
        if preview.inflight.is_none() && (preview.shown - preview.wanted).abs() > 1e-3 {
            let time = preview.wanted;
            let size = platform::preview_size(editor.info.width, editor.info.height);
            let rx = editor.video.preview_frame(time, size, ctx);
            preview.inflight = Some((time, rx));
        }
        editor.size.update(ctx, &editor.video, &editor.settings);
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1 << 20 {
        format!("{:.1} MB", bytes as f64 / (1 << 20) as f64)
    } else {
        format!("{:.0} KB", (bytes as f64 / 1024.0).max(1.0))
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let can_open = matches!(self.stage, Stage::Empty | Stage::Edit | Stage::Done { .. });
        if can_open {
            let picked = self.picker.as_ref().and_then(|rx| rx.try_recv().ok());
            if let Some(input) = picked.or_else(|| platform::dropped_input(ctx)) {
                self.picker = None;
                self.open(ctx, input);
            }
        }
        self.poll_loading(ctx);
        self.poll_job(ctx);
        if matches!(self.stage, Stage::Edit) {
            self.poll_editor(ctx);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        match self.stage {
            Stage::Empty => self.ui_empty(ui),
            Stage::Loading(_) => {
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.centered_and_justified(|ui| ui.spinner());
                });
            }
            Stage::Edit => self.ui_edit(ui, &ctx),
            Stage::Converting(_) => self.ui_converting(ui),
            Stage::Done { .. } => self.ui_done(ui, &ctx),
        }
        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            let screen = ctx.content_rect();
            let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("drop_overlay"));
            let painter = ctx.layer_painter(layer);
            painter.rect_filled(screen, CornerRadius::ZERO, Color32::from_black_alpha(190));
            painter.text(
                screen.center(),
                egui::Align2::CENTER_CENTER,
                "Drop video to open",
                egui::FontId::proportional(28.0),
                Color32::WHITE,
            );
        }
    }
}

impl App {
    fn save(&self, bytes: &[u8]) -> Option<String> {
        let name = self
            .editor
            .as_ref()
            .map(|e| e.video.name())
            .unwrap_or_default();
        let stem = std::path::Path::new(&name)
            .file_stem()
            .map_or("output".into(), |s| s.to_string_lossy());
        platform::save_gif(&format!("{stem}.gif"), bytes)
    }

    fn ui_empty(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() / 3.0);
                ui.heading("Drop a video here");
                ui.add_space(8.0);
                if ui.button("Choose a video…").clicked() {
                    self.picker = Some(platform::pick_file(ui.ctx()));
                }
                ui.add_space(12.0);
                ui.weak("Everything runs locally on your machine.");
                if let Some(message) = &self.startup_error {
                    ui.add_space(8.0);
                    ui.colored_label(Color32::LIGHT_RED, message);
                }
                if let Some(error) = &self.error {
                    ui.colored_label(Color32::LIGHT_RED, error);
                }
            });
        });
    }

    fn ui_edit(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        let mut convert = false;
        let mut open_another = false;
        let duration = editor.info.duration;
        let (src_w, src_h) = (editor.info.width, editor.info.height);

        egui::Panel::right("settings")
            .default_size(290.0)
            .show(ui, |ui| {
                settings_panel(ui, &mut editor.settings, (src_w, src_h));
            });

        egui::Panel::bottom("bar").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                match editor.size.bytes {
                    Some(bytes) => ui.label(format!("Estimated size: ~{}", format_bytes(bytes))),
                    None => ui.label(format!(
                        "Estimated size: ~{} (refining…)",
                        format_bytes(estimate::heuristic_bytes(&editor.settings))
                    )),
                };
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let button = egui::Button::new("Convert to GIF");
                    convert = ui.add_sized([120.0, 30.0], button).clicked();
                    open_another = ui.button("Open another…").clicked();
                });
            });
            ui.add_space(4.0);
        });

        egui::CentralPanel::default().show(ui, |ui| {
            ui.label(editor.video.name());
            let editor_height = (ui.available_height() - 110.0).max(100.0);
            crop_editor(
                ui,
                &editor.preview.texture,
                Vec2::new(src_w as f32, src_h as f32),
                &mut editor.settings.crop,
                editor_height,
            );
            ui.add_space(8.0);
            let s = &mut editor.settings;
            let seconds = |n: f64, _: std::ops::RangeInclusive<usize>| format!("{n:.2}");
            egui::Grid::new("timeline")
                .num_columns(2)
                .spacing([8.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Start");
                    ui.add(
                        egui::Slider::new(&mut s.start, 0.0..=duration)
                            .suffix(" s")
                            .custom_formatter(seconds),
                    );
                    ui.end_row();
                    ui.label("End");
                    ui.add(
                        egui::Slider::new(&mut s.end, 0.0..=duration)
                            .suffix(" s")
                            .custom_formatter(seconds),
                    );
                    ui.end_row();
                    ui.label("Preview");
                    ui.add(
                        egui::Slider::new(&mut editor.preview.wanted, 0.0..=duration)
                            .suffix(" s")
                            .custom_formatter(seconds),
                    );
                    ui.end_row();
                });
            s.end = s.end.max(s.start + 0.1).min(duration);
            s.start = s.start.min(s.end - 0.1).max(0.0);
        });

        if open_another {
            self.picker = Some(platform::pick_file(ctx));
        }
        if convert {
            self.start_convert(ctx);
        }
    }

    fn ui_converting(&mut self, ui: &mut egui::Ui) {
        let Stage::Converting(job) = &self.stage else {
            return;
        };
        egui::CentralPanel::default().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() / 3.0);
                ui.label(format!("Encoding frame {} of ~{}", job.done, job.total));
                let progress = job.done as f32 / job.total.max(1) as f32;
                ui.add(egui::ProgressBar::new(progress).desired_width(360.0));
                ui.add_space(8.0);
                if ui.button("Cancel").clicked() {
                    job.cancel.store(true, Ordering::Relaxed);
                }
            });
        });
    }

    fn ui_done(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Stage::Done { gif, started } = &self.stage else {
            return;
        };
        let (frame, next_in) = gif.frame_at(started.elapsed());
        ctx.request_repaint_after(next_in);
        let mut back = false;
        let mut save_clicked = false;

        egui::Panel::bottom("done_bar").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let size = format_bytes(gif.bytes.len() as u64);
                ui.label(format!("{} frames, {size}", gif.frame_count()));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let save = egui::Button::new("Save GIF…");
                    if ui.add_sized([120.0, 30.0], save).clicked() {
                        save_clicked = true;
                    }
                    back = ui.button("Back to edit").clicked();
                    if let Some(status) = &self.status {
                        ui.label(status);
                    }
                });
            });
            ui.add_space(4.0);
        });
        egui::CentralPanel::default().show(ui, |ui| {
            let rect = fit_rect(ui.available_rect_before_wrap(), frame.size_vec2());
            let full = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
            ui.painter().image(frame.id(), rect, full, Color32::WHITE);
        });
        if save_clicked {
            self.status = self.save(&gif.bytes);
        }
        if back {
            self.status = None;
            self.stage = Stage::Edit;
        }
    }
}

fn settings_panel(ui: &mut egui::Ui, s: &mut ConvertSettings, (src_w, src_h): (u32, u32)) {
    ui.add_space(6.0);
    ui.heading("Settings");
    ui.separator();
    egui::Grid::new("settings_grid")
        .num_columns(2)
        .spacing([8.0, 8.0])
        .show(ui, |ui| {
            ui.label("Speed");
            let speed = egui::Slider::new(&mut s.speed, 0.25..=4.0)
                .logarithmic(true)
                .suffix("×");
            ui.add(speed);
            ui.end_row();

            ui.label("Frame rate");
            let fps_label = |fps: f32| format!("{:.1} fps", 100.0 / delay_cs(fps) as f32);
            egui::ComboBox::from_id_salt("fps")
                .selected_text(fps_label(s.fps))
                .show_ui(ui, |ui| {
                    for fps in FPS_OPTIONS {
                        ui.selectable_value(&mut s.fps, fps, fps_label(fps));
                    }
                });
            ui.end_row();

            ui.label("Width");
            let max_width = platform::MAX_OUTPUT_WIDTH.min(s.crop.w).max(1);
            let min_width = MIN_OUTPUT_WIDTH.min(max_width);
            s.width = s.width.clamp(min_width, max_width);
            ui.add(egui::Slider::new(&mut s.width, min_width..=max_width).suffix(" px"));
            ui.end_row();

            ui.label("Quality");
            egui::ComboBox::from_id_salt("quality")
                .selected_text(s.quality.label())
                .show_ui(ui, |ui| {
                    for quality in Quality::ALL {
                        ui.selectable_value(&mut s.quality, quality, quality.label());
                    }
                });
            ui.end_row();

            ui.label("Loop");
            ui.checkbox(&mut s.looping, "Repeat forever");
            ui.end_row();
        });

    ui.separator();
    ui.strong("Crop");
    ui.horizontal_wrapped(|ui| {
        if ui.small_button("Free").clicked() {
            s.crop = CropRect::full(src_w, src_h);
        }
        for (label, rw, rh) in [("1:1", 1.0, 1.0), ("16:9", 16.0, 9.0), ("9:16", 9.0, 16.0)] {
            if ui.small_button(label).clicked() {
                s.crop = centered_ratio(src_w, src_h, rw, rh);
            }
        }
    });
    let crop = &mut s.crop;
    ui.horizontal(|ui| {
        ui.label("x");
        ui.add(egui::DragValue::new(&mut crop.x).range(0..=src_w.saturating_sub(2)));
        ui.label("y");
        ui.add(egui::DragValue::new(&mut crop.y).range(0..=src_h.saturating_sub(2)));
    });
    ui.horizontal(|ui| {
        ui.label("w");
        ui.add(egui::DragValue::new(&mut crop.w).range(2..=src_w));
        ui.label("h");
        ui.add(egui::DragValue::new(&mut crop.h).range(2..=src_h));
    });
    crop.w = crop.w.min(src_w - crop.x.min(src_w - 2));
    crop.h = crop.h.min(src_h - crop.y.min(src_h - 2));

    ui.separator();
    let (out_w, out_h) = s.output_size();
    let seconds = output_duration(s);
    ui.label(format!("Output: {out_w}×{out_h}, {seconds:.1} s"));
    if seconds > LONG_GIF_SECS {
        ui.colored_label(
            Color32::YELLOW,
            "Long GIFs get very large. Consider trimming or lowering width/fps.",
        );
    }
}
