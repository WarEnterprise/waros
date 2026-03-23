use super::font;
use super::framebuffer::{Rect, Surface};
use super::theme::Theme;

pub fn draw_button(surface: &mut Surface<'_>, rect: Rect, label: &str, accent: bool) {
    let background = if accent {
        Theme::BUTTON_ACCENT
    } else {
        Theme::BUTTON_BG
    };
    surface.fill_rect(rect.x, rect.y, rect.width, rect.height, background);
    surface.draw_rect(rect.x, rect.y, rect.width, rect.height, Theme::WINDOW_BORDER);

    let text_width = font::text_width(label, 1);
    let text_x = rect.x + rect.width.saturating_sub(text_width) / 2;
    let text_y = rect.y + rect.height.saturating_sub(font::line_height(1)) / 2;
    font::draw_text(surface, text_x, text_y, label, Theme::BUTTON_TEXT);
}
