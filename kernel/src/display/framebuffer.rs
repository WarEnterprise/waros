use bootloader_api::info::{FrameBuffer, FrameBufferInfo, PixelFormat};

/// Low-level framebuffer access helper backed by the bootloader framebuffer mapping.
pub struct Framebuffer {
    buffer: &'static mut [u8],
    info: FrameBufferInfo,
}

impl Framebuffer {
    /// Construct a framebuffer wrapper from the bootloader framebuffer object.
    #[must_use]
    pub fn new(framebuffer: &'static mut FrameBuffer) -> Self {
        let info = framebuffer.info();
        let buffer = framebuffer.buffer_mut();
        Self {
            buffer,
            info,
        }
    }

    /// Return the framebuffer metadata.
    #[must_use]
    pub fn info(&self) -> FrameBufferInfo {
        self.info
    }

    /// Fill the whole framebuffer with a single ARGB color.
    pub fn clear(&mut self, color: u32) {
        for offset in (0..self.buffer.len()).step_by(self.info.bytes_per_pixel) {
            self.write_color_at_offset(offset, color);
        }
    }

    /// Scroll the framebuffer content upward by `pixel_rows`, filling the bottom with `color`.
    pub fn scroll_up(&mut self, pixel_rows: usize, color: u32) {
        let bytes_per_row = self.info.stride * self.info.bytes_per_pixel;
        let byte_rows = pixel_rows * bytes_per_row;
        if byte_rows >= self.buffer.len() {
            self.clear(color);
            return;
        }

        self.buffer.copy_within(byte_rows.., 0);
        let buffer_len = self.buffer.len();
        for offset in ((buffer_len - byte_rows)..buffer_len).step_by(self.info.bytes_per_pixel) {
            self.write_color_at_offset(offset, color);
        }
    }

    /// Write one ARGB pixel to the framebuffer.
    pub fn write_pixel(&mut self, x: usize, y: usize, color: u32) {
        if x >= self.info.width || y >= self.info.height {
            return;
        }

        let pixel_index = y * self.info.stride + x;
        let byte_index = pixel_index * self.info.bytes_per_pixel;
        self.write_color_at_offset(byte_index, color);
    }

    fn write_color_at_offset(&mut self, byte_index: usize, color: u32) {
        if byte_index + self.info.bytes_per_pixel > self.buffer.len() {
            return;
        }

        let [_, red, green, blue] = color.to_be_bytes();

        match self.info.pixel_format {
            PixelFormat::Rgb => {
                self.buffer[byte_index] = red;
                if self.info.bytes_per_pixel > 1 {
                    self.buffer[byte_index + 1] = green;
                }
                if self.info.bytes_per_pixel > 2 {
                    self.buffer[byte_index + 2] = blue;
                }
            }
            PixelFormat::Bgr => {
                self.buffer[byte_index] = blue;
                if self.info.bytes_per_pixel > 1 {
                    self.buffer[byte_index + 1] = green;
                }
                if self.info.bytes_per_pixel > 2 {
                    self.buffer[byte_index + 2] = red;
                }
            }
            PixelFormat::U8 => {
                let luminance = ((u16::from(red) + u16::from(green) + u16::from(blue)) / 3) as u8;
                self.buffer[byte_index] = luminance;
            }
            PixelFormat::Unknown {
                red_position,
                green_position,
                blue_position,
            } => {
                let mut raw = 0u32;
                raw |= u32::from(red) << red_position;
                raw |= u32::from(green) << green_position;
                raw |= u32::from(blue) << blue_position;
                let raw_bytes = raw.to_le_bytes();
                let count = self.info.bytes_per_pixel.min(raw_bytes.len());
                self.buffer[byte_index..byte_index + count].copy_from_slice(&raw_bytes[..count]);
            }
            _ => {
                self.buffer[byte_index] = red;
                if self.info.bytes_per_pixel > 1 {
                    self.buffer[byte_index + 1] = green;
                }
                if self.info.bytes_per_pixel > 2 {
                    self.buffer[byte_index + 2] = blue;
                }
            }
        }

        if self.info.bytes_per_pixel == 4 {
            self.buffer[byte_index + 3] = 0;
        }
    }
}
