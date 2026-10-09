use super::dom::{document, first_event, js_error};
use vidgif_core::{CropRect, RgbaFrame};
use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, HtmlVideoElement};

const LOAD_TIMEOUT_MS: i32 = 20_000;
const SEEK_TIMEOUT_MS: i32 = 10_000;

/// Reads frames out of a video by seeking a hidden `<video>` element and drawing it to a canvas.
/// The browser does the decoding, so whatever it can play (MP4/H.264, WebM, ...) works.
pub struct Grabber {
    video: HtmlVideoElement,
    canvas: HtmlCanvasElement,
    context: CanvasRenderingContext2d,
}

impl Grabber {
    pub async fn open(url: &str) -> Result<Self, String> {
        let doc = document();
        let video: HtmlVideoElement = doc
            .create_element("video")
            .map_err(js_error)?
            .unchecked_into();
        video.set_muted(true);
        video.set_preload("auto");
        let _ = video.set_attribute("playsinline", "");
        // Kept in the page (but invisible) because some browsers won't decode detached videos.
        let _ = video.set_attribute(
            "style",
            "position:fixed;left:0;top:0;width:1px;height:1px;opacity:0;pointer-events:none",
        );
        doc.body()
            .expect("body")
            .append_child(&video)
            .map_err(js_error)?;

        let loaded = first_event(&video, &["loadeddata", "error"], LOAD_TIMEOUT_MS);
        video.set_src(url);
        match loaded.await.as_str() {
            "loadeddata" => {}
            "error" => return Err("This browser can't play that video format.".into()),
            _ => return Err("Timed out while loading the video.".into()),
        }
        if !video.duration().is_finite() || video.video_width() == 0 {
            return Err("Couldn't read the video's length or size.".into());
        }

        let canvas: HtmlCanvasElement = doc
            .create_element("canvas")
            .map_err(js_error)?
            .unchecked_into();
        let options = js_sys::Object::new();
        js_sys::Reflect::set(&options, &"willReadFrequently".into(), &true.into()).ok();
        js_sys::Reflect::set(&options, &"alpha".into(), &false.into()).ok();
        let context: CanvasRenderingContext2d = canvas
            .get_context_with_context_options("2d", &options)
            .map_err(js_error)?
            .ok_or("canvas 2d context unavailable")?
            .unchecked_into();
        js_sys::Reflect::set(&context, &"imageSmoothingQuality".into(), &"high".into()).ok();
        Ok(Self {
            video,
            canvas,
            context,
        })
    }

    pub fn duration(&self) -> f64 {
        self.video.duration()
    }

    pub fn size(&self) -> (u32, u32) {
        (self.video.video_width(), self.video.video_height())
    }

    /// The frame at `time` seconds, cropped to `crop` (source pixels) and scaled to `out`.
    pub async fn grab(
        &self,
        time: f64,
        crop: CropRect,
        out: (u32, u32),
    ) -> Result<RgbaFrame, String> {
        let time = time.clamp(0.0, (self.video.duration() - 0.001).max(0.0));
        // `seeked` doesn't fire when the position doesn't change.
        if (self.video.current_time() - time).abs() > 1e-4 {
            let seeked = first_event(&self.video, &["seeked"], SEEK_TIMEOUT_MS);
            self.video.set_current_time(time);
            if seeked.await != "seeked" {
                return Err(format!("Timed out seeking to {time:.2}s."));
            }
        }
        let (w, h) = out;
        if self.canvas.width() != w || self.canvas.height() != h {
            self.canvas.set_width(w);
            self.canvas.set_height(h);
        }
        self.context
            .draw_image_with_html_video_element_and_sw_and_sh_and_dx_and_dy_and_dw_and_dh(
                &self.video,
                crop.x as f64,
                crop.y as f64,
                crop.w as f64,
                crop.h as f64,
                0.0,
                0.0,
                w as f64,
                h as f64,
            )
            .map_err(js_error)?;
        let pixels = self
            .context
            .get_image_data(0.0, 0.0, w as f64, h as f64)
            .map_err(js_error)?;
        Ok(RgbaFrame::new(w, h, pixels.data().0))
    }
}

impl Drop for Grabber {
    fn drop(&mut self) {
        self.video.remove();
        self.video.set_src("");
    }
}
