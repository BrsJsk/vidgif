use egui::{Color32, CornerRadius, Pos2, Rect, Sense, Stroke, StrokeKind, TextureHandle, Vec2};
use vidgif_core::CropRect;

/// Largest rectangle with ratio `rw:rh` that fits the frame, centered.
pub fn centered_ratio(frame_w: u32, frame_h: u32, rw: f32, rh: f32) -> CropRect {
    let (mut w, mut h) = (frame_w as f32, frame_w as f32 * rh / rw);
    if h > frame_h as f32 {
        h = frame_h as f32;
        w = h * rw / rh;
    }
    let (w, h) = ((w as u32).max(2) & !1, (h as u32).max(2) & !1);
    CropRect {
        x: (frame_w - w) / 2,
        y: (frame_h - h) / 2,
        w,
        h,
    }
}

/// Largest rect with the given size's aspect ratio that fits in `avail`, centered.
pub fn fit_rect(avail: Rect, size: Vec2) -> Rect {
    let scale = (avail.width() / size.x)
        .min(avail.height() / size.y)
        .max(0.01);
    Rect::from_center_size(avail.center(), size * scale)
}

const MIN_CROP: f32 = 16.0;
const HANDLE_HIT: f32 = 14.0;
const HANDLE_DRAW: f32 = 9.0;

/// Handle positions as fractions of the selection, in (x, y).
const HANDLES: [(f32, f32); 8] = [
    (0.0, 0.0),
    (0.5, 0.0),
    (1.0, 0.0),
    (0.0, 0.5),
    (1.0, 0.5),
    (0.0, 1.0),
    (0.5, 1.0),
    (1.0, 1.0),
];

/// Shows `texture` with a draggable crop rectangle. `source` is the real video size in pixels;
/// `crop` is in those pixels, so the texture may be a downscaled preview.
pub fn crop_editor(
    ui: &mut egui::Ui,
    texture: &TextureHandle,
    source: Vec2,
    crop: &mut CropRect,
    height: f32,
) {
    let area = Rect::from_min_size(ui.cursor().min, Vec2::new(ui.available_width(), height));
    ui.allocate_rect(area, Sense::hover());
    let image = fit_rect(area, source);
    let scale = image.width() / source.x;
    let painter = ui.painter_at(area);
    painter.image(
        texture.id(),
        image,
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        Color32::WHITE,
    );

    let to_screen = |x: f32, y: f32| image.min + Vec2::new(x, y) * scale;
    let (mut left, mut top) = (crop.x as f32, crop.y as f32);
    let (mut right, mut bottom) = ((crop.x + crop.w) as f32, (crop.y + crop.h) as f32);
    let selection = Rect::from_min_max(to_screen(left, top), to_screen(right, bottom));

    let shade = Color32::from_black_alpha(140);
    let outside = [
        Rect::from_min_max(image.min, Pos2::new(image.max.x, selection.min.y)),
        Rect::from_min_max(Pos2::new(image.min.x, selection.max.y), image.max),
        Rect::from_min_max(
            Pos2::new(image.min.x, selection.min.y),
            Pos2::new(selection.min.x, selection.max.y),
        ),
        Rect::from_min_max(
            Pos2::new(selection.max.x, selection.min.y),
            Pos2::new(image.max.x, selection.max.y),
        ),
    ];
    for rect in outside {
        painter.rect_filled(rect, CornerRadius::ZERO, shade);
    }
    painter.rect_stroke(
        selection,
        CornerRadius::ZERO,
        Stroke::new(1.5, Color32::WHITE),
        StrokeKind::Inside,
    );

    let body = ui.interact(
        selection.shrink(6.0),
        ui.id().with("crop_body"),
        Sense::drag(),
    );
    if body.dragged() {
        let d = body.drag_delta() / scale;
        let (w, h) = (right - left, bottom - top);
        left = (left + d.x).clamp(0.0, source.x - w);
        top = (top + d.y).clamp(0.0, source.y - h);
        right = left + w;
        bottom = top + h;
    }

    for (i, (fx, fy)) in HANDLES.into_iter().enumerate() {
        let center = Pos2::new(
            selection.min.x + selection.width() * fx,
            selection.min.y + selection.height() * fy,
        );
        let hit = Rect::from_center_size(center, Vec2::splat(HANDLE_HIT));
        let response = ui.interact(hit, ui.id().with(("crop_handle", i)), Sense::drag());
        painter.rect_filled(
            Rect::from_center_size(center, Vec2::splat(HANDLE_DRAW)),
            CornerRadius::same(2),
            Color32::WHITE,
        );
        if response.dragged() {
            let d = response.drag_delta() / scale;
            if fx == 0.0 {
                left = (left + d.x).clamp(0.0, right - MIN_CROP);
            } else if fx == 1.0 {
                right = (right + d.x).clamp(left + MIN_CROP, source.x);
            }
            if fy == 0.0 {
                top = (top + d.y).clamp(0.0, bottom - MIN_CROP);
            } else if fy == 1.0 {
                bottom = (bottom + d.y).clamp(top + MIN_CROP, source.y);
            }
        }
    }

    *crop = CropRect {
        x: left.round() as u32,
        y: top.round() as u32,
        w: ((right - left).round() as u32).max(2),
        h: ((bottom - top).round() as u32).max(2),
    };
}
