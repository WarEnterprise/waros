use crate::display::font as glyphs;

use super::framebuffer::{Color, Surface};

#[must_use]
pub fn line_height(scale: usize) -> usize {
    glyphs::FONT_HEIGHT_PIXELS * scale
}

#[must_use]
pub fn text_width(text: &str, scale: usize) -> usize {
    text.chars().count() * glyphs::FONT_WIDTH * scale
}

pub fn draw_text(surface: &mut Surface<'_>, x: usize, y: usize, text: &str, color: Color) {
    draw_text_scaled(surface, x, y, text, color, 1);
}

pub fn draw_text_scaled(
    surface: &mut Surface<'_>,
    mut x: usize,
    y: usize,
    text: &str,
    color: Color,
    scale: usize,
) {
    for character in text.chars() {
        draw_glyph(surface, x, y, character, color, scale);
        x += glyphs::FONT_WIDTH * scale;
    }
}

fn draw_glyph(surface: &mut Surface<'_>, x: usize, y: usize, character: char, color: Color, scale: usize) {
    let glyph = glyphs::glyph(character);
    for (row_index, row) in glyph.raster().iter().enumerate() {
        for (column_index, alpha) in row.iter().copied().enumerate() {
            if alpha == 0 {
                continue;
            }

            for sy in 0..scale {
                for sx in 0..scale {
                    let px = x + column_index * scale + sx;
                    let py = y + row_index * scale + sy;
                    let background = surface.get_pixel(px, py);
                    surface.set_pixel(px, py, Color(blend(background, color.value(), alpha)));
                }
            }
        }
    }
}

fn blend(background: u32, foreground: u32, alpha: u8) -> u32 {
    let [_, bg_red, bg_green, bg_blue] = background.to_be_bytes();
    let [_, fg_red, fg_green, fg_blue] = foreground.to_be_bytes();
    let alpha = u16::from(alpha);
    let inverse_alpha = u16::from(u8::MAX) - alpha;

    let red = ((u16::from(bg_red) * inverse_alpha) + (u16::from(fg_red) * alpha))
        / u16::from(u8::MAX);
    let green = ((u16::from(bg_green) * inverse_alpha) + (u16::from(fg_green) * alpha))
        / u16::from(u8::MAX);
    let blue = ((u16::from(bg_blue) * inverse_alpha) + (u16::from(fg_blue) * alpha))
        / u16::from(u8::MAX);

    ((red as u32) << 16) | ((green as u32) << 8) | blue as u32
}
