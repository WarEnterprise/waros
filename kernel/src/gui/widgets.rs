use super::font;
use super::framebuffer::{Rect, Surface};
use super::theme::Theme;

#[derive(Clone, Copy)]
pub struct ButtonStyle {
    pub background: super::framebuffer::Color,
    pub border: super::framebuffer::Color,
    pub text: super::framebuffer::Color,
    pub underline: Option<super::framebuffer::Color>,
}

pub fn draw_button(surface: &mut Surface<'_>, rect: Rect, label: &str, style: ButtonStyle) {
    surface.fill_rounded_rect(rect.x, rect.y, rect.width, rect.height, 3, style.background);
    surface.draw_rounded_rect(rect.x, rect.y, rect.width, rect.height, 3, style.border);

    if let Some(underline) = style.underline {
        surface.fill_rect(rect.x + 2, rect.y + rect.height.saturating_sub(3), rect.width.saturating_sub(4), 2, underline);
    }

    let text_width = font::text_width(label, 1);
    let text_x = rect.x + rect.width.saturating_sub(text_width) / 2;
    let text_y = rect.y + rect.height.saturating_sub(font::line_height(1)) / 2;
    font::draw_text(surface, text_x, text_y, label, style.text);
}

#[must_use]
pub fn button_style(
    active: bool,
    hovered: bool,
    accent: bool,
    underline: bool,
) -> ButtonStyle {
    let background = if accent {
        if hovered {
            Theme::BUTTON_ACCENT_HOVER
        } else {
            Theme::BUTTON_ACCENT
        }
    } else if active {
        Theme::BUTTON_ACTIVE
    } else if hovered {
        Theme::BUTTON_HOVER
    } else {
        Theme::BUTTON_BG
    };

    let text = if accent || active || hovered {
        Theme::BUTTON_TEXT
    } else {
        Theme::BUTTON_TEXT_DIM
    };

    ButtonStyle {
        background,
        border: Theme::WINDOW_BORDER,
        text,
        underline: if underline { Some(Theme::TASKBAR_ACCENT) } else { None },
    }
}

#[must_use]
pub fn button_width(label: &str) -> usize {
    font::text_width(label, 1) + 20
}
