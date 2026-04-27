use alloc::vec::Vec;

use bootloader_api::info::{FrameBuffer, FrameBufferInfo, PixelFormat};

const FULL_SHADOW_HEAP_BUDGET_BYTES: usize = 4 * 1024 * 1024;
const MIN_HEAP_HEADROOM_AFTER_SHADOW_BYTES: usize = 2 * 1024 * 1024;

/// Low-level framebuffer access helper backed by the bootloader framebuffer mapping.
pub struct Framebuffer {
    buffer: &'static mut [u8],
    shadow: Option<Vec<u8>>,
    info: FrameBufferInfo,
    batch_depth: usize,
    dirty_start_row: Option<usize>,
    dirty_end_row: usize,
}

impl Framebuffer {
    /// Construct a framebuffer wrapper from the bootloader framebuffer object.
    ///
    /// Early boot must not allocate from the heap before the allocator is online,
    /// so the framebuffer starts in direct-write mode and can be promoted later.
    #[must_use]
    pub fn new(framebuffer: &'static mut FrameBuffer) -> Self {
        let info = framebuffer.info();
        let buffer = framebuffer.buffer_mut();
        Self {
            buffer,
            shadow: None,
            info,
            batch_depth: 0,
            dirty_start_row: None,
            dirty_end_row: 0,
        }
    }

    /// Return the framebuffer metadata.
    #[must_use]
    pub fn info(&self) -> FrameBufferInfo {
        self.info
    }

    /// Upgrade to shadow-buffer rendering once the kernel heap is ready.
    pub fn enable_shadow_buffer(&mut self) -> Result<(), &'static str> {
        if self.shadow.is_some() {
            return Ok(());
        }

        let required_bytes = self.buffer.len();
        crate::memory::heap::log_named_request("framebuffer-shadow-full", required_bytes);
        if required_bytes > FULL_SHADOW_HEAP_BUDGET_BYTES {
            crate::serial_println!(
                "[FB] shadow skipped bytes={} budget={} reason=budget",
                required_bytes,
                FULL_SHADOW_HEAP_BUDGET_BYTES
            );
            return Err("framebuffer shadow exceeds boot heap budget");
        }

        let heap_stats = crate::memory::heap::stats();
        if required_bytes
            .saturating_add(MIN_HEAP_HEADROOM_AFTER_SHADOW_BYTES)
            > heap_stats.free
        {
            crate::serial_println!(
                "[FB] shadow skipped bytes={} free={} reserve={} reason=headroom",
                required_bytes,
                heap_stats.free,
                MIN_HEAP_HEADROOM_AFTER_SHADOW_BYTES
            );
            return Err("framebuffer shadow would leave insufficient boot heap headroom");
        }

        let mut shadow = Vec::new();
        shadow
            .try_reserve_exact(required_bytes)
            .map_err(|_| "framebuffer shadow allocation failed")?;
        shadow.resize(required_bytes, 0);
        shadow.copy_from_slice(self.buffer);
        self.shadow = Some(shadow);
        self.mark_dirty_rows(0, self.info.height);
        self.flush_dirty();
        crate::serial_println!(
            "[FB] shadow enabled bytes={} width={} height={} stride={} bpp={}",
            required_bytes,
            self.info.width,
            self.info.height,
            self.info.stride,
            self.info.bytes_per_pixel
        );
        crate::memory::heap::log_usage("framebuffer-shadow-enabled");
        Ok(())
    }

    /// Fill the whole framebuffer with a single ARGB color.
    pub fn clear(&mut self, color: u32) {
        let bytes_per_pixel = self.bytes_per_pixel();
        let len = self.active_buffer().len();
        for offset in (0..len).step_by(bytes_per_pixel) {
            self.write_color_at_offset(offset, color);
        }
    }

    /// Scroll the framebuffer content upward by `pixel_rows`, filling the bottom with `color`.
    pub fn scroll_up(&mut self, pixel_rows: usize, color: u32) {
        let bytes_per_row = self.bytes_per_row();
        if bytes_per_row == 0 {
            return;
        }
        let byte_rows = pixel_rows.saturating_mul(bytes_per_row);
        let buffer_len = self.active_buffer().len();
        if byte_rows >= buffer_len {
            self.clear(color);
            return;
        }

        if let Some(shadow) = self.shadow.as_mut() {
            shadow.copy_within(byte_rows.., 0);
        } else {
            self.buffer.copy_within(byte_rows.., 0);
        }

        for offset in ((buffer_len - byte_rows)..buffer_len).step_by(self.bytes_per_pixel()) {
            self.write_color_at_offset(offset, color);
        }

        if self.shadow.is_some() {
            self.mark_dirty_rows(0, self.info.height);
            if self.batch_depth == 0 {
                self.flush_dirty();
            }
        }
    }

    /// Write one ARGB pixel to the framebuffer.
    pub fn write_pixel(&mut self, x: usize, y: usize, color: u32) {
        if x >= self.info.width || y >= self.info.height {
            return;
        }

        let pixel_index = y.saturating_mul(self.info.stride.max(self.info.width)) + x;
        let byte_index = pixel_index.saturating_mul(self.bytes_per_pixel());
        self.write_color_at_offset(byte_index, color);
    }

    pub fn begin_batch(&mut self) {
        self.batch_depth = self.batch_depth.saturating_add(1);
    }

    pub fn end_batch(&mut self) {
        if self.batch_depth == 0 {
            return;
        }
        self.batch_depth -= 1;
        if self.batch_depth == 0 {
            self.flush_dirty();
        }
    }

    fn active_buffer(&self) -> &[u8] {
        self.shadow.as_deref().unwrap_or(self.buffer)
    }

    fn bytes_per_pixel(&self) -> usize {
        self.info.bytes_per_pixel.max(1)
    }

    fn bytes_per_row(&self) -> usize {
        self.info
            .stride
            .max(self.info.width)
            .saturating_mul(self.bytes_per_pixel())
    }

    fn write_color_at_offset(&mut self, byte_index: usize, color: u32) {
        let bytes_per_pixel = self.bytes_per_pixel();
        let target_len = self.active_buffer().len();
        if byte_index + bytes_per_pixel > target_len {
            return;
        }

        let [_, red, green, blue] = color.to_be_bytes();

        if self.shadow.is_some() {
            self.mark_dirty_offset(byte_index);
        }

        if let Some(shadow) = self.shadow.as_mut() {
            write_color_bytes(
                shadow,
                byte_index,
                bytes_per_pixel,
                self.info.pixel_format,
                red,
                green,
                blue,
            );
            if self.batch_depth == 0 {
                self.flush_dirty();
            }
        } else {
            write_color_bytes(
                self.buffer,
                byte_index,
                bytes_per_pixel,
                self.info.pixel_format,
                red,
                green,
                blue,
            );
        }
    }

    fn mark_dirty_offset(&mut self, byte_index: usize) {
        let bytes_per_row = self.bytes_per_row();
        if bytes_per_row == 0 {
            return;
        }
        let row = byte_index / bytes_per_row;
        self.mark_dirty_rows(row, 1);
    }

    fn mark_dirty_rows(&mut self, start_row: usize, row_count: usize) {
        if self.shadow.is_none() || row_count == 0 {
            return;
        }
        let end_row = start_row.saturating_add(row_count).min(self.info.height);
        self.dirty_end_row = self.dirty_end_row.max(end_row);
        self.dirty_start_row = Some(
            self.dirty_start_row
                .map(|row| row.min(start_row))
                .unwrap_or(start_row),
        );
    }

    fn flush_dirty(&mut self) {
        let Some(shadow) = self.shadow.as_ref() else {
            return;
        };
        let Some(start_row) = self.dirty_start_row.take() else {
            return;
        };
        let end_row = self.dirty_end_row.min(self.info.height);
        self.dirty_end_row = 0;
        if start_row >= end_row {
            return;
        }

        let bytes_per_row = self.bytes_per_row();
        if bytes_per_row == 0 {
            return;
        }
        let start = start_row.saturating_mul(bytes_per_row);
        let end = end_row.saturating_mul(bytes_per_row).min(self.buffer.len());
        self.buffer[start..end].copy_from_slice(&shadow[start..end]);
    }
}

fn write_color_bytes(
    buffer: &mut [u8],
    byte_index: usize,
    bytes_per_pixel: usize,
    pixel_format: PixelFormat,
    red: u8,
    green: u8,
    blue: u8,
) {
    match pixel_format {
        PixelFormat::Rgb => {
            buffer[byte_index] = red;
            if bytes_per_pixel > 1 {
                buffer[byte_index + 1] = green;
            }
            if bytes_per_pixel > 2 {
                buffer[byte_index + 2] = blue;
            }
        }
        PixelFormat::Bgr => {
            buffer[byte_index] = blue;
            if bytes_per_pixel > 1 {
                buffer[byte_index + 1] = green;
            }
            if bytes_per_pixel > 2 {
                buffer[byte_index + 2] = red;
            }
        }
        PixelFormat::U8 => {
            let luminance = ((u16::from(red) + u16::from(green) + u16::from(blue)) / 3) as u8;
            buffer[byte_index] = luminance;
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
            let count = bytes_per_pixel.min(raw_bytes.len());
            buffer[byte_index..byte_index + count].copy_from_slice(&raw_bytes[..count]);
        }
        _ => {
            buffer[byte_index] = red;
            if bytes_per_pixel > 1 {
                buffer[byte_index + 1] = green;
            }
            if bytes_per_pixel > 2 {
                buffer[byte_index + 2] = blue;
            }
        }
    }

    if bytes_per_pixel == 4 {
        buffer[byte_index + 3] = 0;
    }
}
