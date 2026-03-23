use super::font;
use super::framebuffer::Surface;
use super::theme::Theme;

pub fn render_desktop(surface: &mut Surface<'_>) {
    let width = surface.width();
    let height = surface.height();
    surface.fill_vertical_gradient(0, 0, width, height, Theme::DESKTOP_BG, Theme::DESKTOP_BG_BOTTOM);

    for y in (0..height).step_by(40) {
        for x in (0..width).step_by(40) {
            surface.set_pixel(x, y, Theme::DESKTOP_PATTERN);
        }
    }

    let label = "War Enterprise | warenterprise.com/waros";
    let label_width = font::text_width(label, 1);
    let x = width.saturating_sub(label_width + 16);
    let y = height.saturating_sub(font::line_height(1) + 8);
    font::draw_text(surface, x, y, label, Theme::DESKTOP_WATERMARK);
}
