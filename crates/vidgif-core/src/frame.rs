/// A tightly packed RGBA8 image.
#[derive(Clone)]
pub struct RgbaFrame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl RgbaFrame {
    pub fn new(width: u32, height: u32, data: Vec<u8>) -> Self {
        debug_assert_eq!(data.len(), (width * height * 4) as usize);
        Self {
            width,
            height,
            data,
        }
    }
}
