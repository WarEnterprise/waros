use super::framebuffer::Surface;
use super::theme::Theme;

const CURSOR_WIDTH: usize = 12;
const CURSOR_HEIGHT: usize = 16;

const CURSOR_BITMAP: [[u8; CURSOR_WIDTH]; CURSOR_HEIGHT] = [
    [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [1, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [1, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0],
    [1, 2, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0],
    [1, 2, 2, 2, 2, 1, 0, 0, 0, 0, 0, 0],
    [1, 2, 2, 2, 2, 2, 1, 0, 0, 0, 0, 0],
    [1, 2, 2, 2, 2, 2, 2, 1, 0, 0, 0, 0],
    [1, 2, 2, 2, 2, 2, 2, 2, 1, 0, 0, 0],
    [1, 2, 2, 2, 2, 2, 2, 2, 2, 1, 0, 0],
    [1, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 0],
    [1, 2, 2, 1, 2, 2, 1, 0, 0, 0, 0, 0],
    [1, 2, 1, 0, 1, 2, 2, 1, 0, 0, 0, 0],
    [1, 1, 0, 0, 1, 2, 2, 1, 0, 0, 0, 0],
    [1, 0, 0, 0, 0, 1, 2, 2, 1, 0, 0, 0],
    [0, 0, 0, 0, 0, 1, 1, 1, 0, 0, 0, 0],
];

pub fn render_cursor(surface: &mut Surface<'_>, x: usize, y: usize) {
    for (row, line) in CURSOR_BITMAP.iter().enumerate() {
        for (column, pixel) in line.iter().copied().enumerate() {
            if pixel != 0 {
                surface.set_pixel(x + column + 1, y + row + 1, Theme::CURSOR_SHADOW);
            }
        }
    }

    for (row, line) in CURSOR_BITMAP.iter().enumerate() {
        for (column, pixel) in line.iter().copied().enumerate() {
            match pixel {
                1 => surface.set_pixel(x + column, y + row, Theme::CURSOR_BORDER),
                2 => surface.set_pixel(x + column, y + row, Theme::CURSOR_COLOR),
                _ => {}
            }
        }
    }
}
