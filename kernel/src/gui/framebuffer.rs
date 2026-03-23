use alloc::vec;
use alloc::vec::Vec;

use crate::display::console;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color(pub u32);

impl Color {
    pub const fn new(red: u8, green: u8, blue: u8) -> Self {
        Self(((red as u32) << 16) | ((green as u32) << 8) | (blue as u32))
    }

    pub const fn from_hex(hex: u32) -> Self {
        Self(hex & 0x00FF_FFFF)
    }

    #[must_use]
    pub const fn value(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

pub struct Surface<'a> {
    pixels: &'a mut [u32],
    width: usize,
    height: usize,
}

impl<'a> Surface<'a> {
    #[must_use]
    pub fn new(pixels: &'a mut [u32], width: usize, height: usize) -> Self {
        Self {
            pixels,
            width,
            height,
        }
    }

    #[must_use]
    pub fn width(&self) -> usize {
        self.width
    }

    #[must_use]
    pub fn height(&self) -> usize {
        self.height
    }

    pub fn clear(&mut self, color: Color) {
        self.pixels.fill(color.value());
    }

    pub fn set_pixel(&mut self, x: usize, y: usize, color: Color) {
        if x >= self.width || y >= self.height {
            return;
        }

        self.pixels[y * self.width + x] = color.value();
    }

    #[must_use]
    pub fn get_pixel(&self, x: usize, y: usize) -> u32 {
        if x >= self.width || y >= self.height {
            return 0;
        }
        self.pixels[y * self.width + x]
    }

    pub fn fill_rect(&mut self, x: usize, y: usize, width: usize, height: usize, color: Color) {
        let max_y = y.saturating_add(height).min(self.height);
        let max_x = x.saturating_add(width).min(self.width);
        for py in y..max_y {
            let row_offset = py * self.width;
            for px in x..max_x {
                self.pixels[row_offset + px] = color.value();
            }
        }
    }

    pub fn draw_hline(&mut self, x: usize, y: usize, width: usize, color: Color) {
        self.fill_rect(x, y, width, 1, color);
    }

    pub fn draw_vline(&mut self, x: usize, y: usize, height: usize, color: Color) {
        self.fill_rect(x, y, 1, height, color);
    }

    pub fn draw_rect(&mut self, x: usize, y: usize, width: usize, height: usize, color: Color) {
        if width == 0 || height == 0 {
            return;
        }
        self.draw_hline(x, y, width, color);
        self.draw_hline(x, y + height.saturating_sub(1), width, color);
        self.draw_vline(x, y, height, color);
        self.draw_vline(x + width.saturating_sub(1), y, height, color);
    }

    pub fn draw_line(
        &mut self,
        mut x0: i32,
        mut y0: i32,
        x1: i32,
        y1: i32,
        color: Color,
    ) {
        let dx = (x1 - x0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let dy = -(y1 - y0).abs();
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;

        loop {
            if x0 >= 0 && y0 >= 0 {
                self.set_pixel(x0 as usize, y0 as usize, color);
            }
            if x0 == x1 && y0 == y1 {
                break;
            }
            let doubled = err * 2;
            if doubled >= dy {
                err += dy;
                x0 += sx;
            }
            if doubled <= dx {
                err += dx;
                y0 += sy;
            }
        }
    }

    pub fn blit(
        &mut self,
        source: &[u32],
        src_width: usize,
        src_height: usize,
        dst_x: usize,
        dst_y: usize,
    ) {
        let max_y = src_height.min(self.height.saturating_sub(dst_y));
        let max_x = src_width.min(self.width.saturating_sub(dst_x));
        for row in 0..max_y {
            let src_offset = row * src_width;
            let dst_offset = (dst_y + row) * self.width + dst_x;
            self.pixels[dst_offset..dst_offset + max_x]
                .copy_from_slice(&source[src_offset..src_offset + max_x]);
        }
    }

    #[must_use]
    pub fn pixels(&self) -> &[u32] {
        self.pixels
    }
}

#[must_use]
pub fn make_buffer(width: usize, height: usize) -> Vec<u32> {
    vec![0; width * height]
}

pub fn flush_to_screen(buffer: &[u32], width: usize, height: usize) {
    let _ = console::with_console(|console| {
        let max_width = width.min(console.width_pixels());
        let max_height = height.min(console.height_pixels());
        for y in 0..max_height {
            let row_offset = y * width;
            for x in 0..max_width {
                console.write_pixel(x, y, buffer[row_offset + x]);
            }
        }
    });
}
