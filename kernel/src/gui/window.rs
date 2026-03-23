use alloc::string::{String, ToString};

use super::apps::AppKind;
use super::font;
use super::framebuffer::{make_buffer, Surface};
use super::theme::Theme;

pub struct Window {
    pub id: u32,
    pub title: String,
    pub x: i32,
    pub y: i32,
    pub width: usize,
    pub height: usize,
    pub focused: bool,
    pub visible: bool,
    pub dragging: bool,
    pub drag_offset_x: i32,
    pub drag_offset_y: i32,
    pub content: alloc::vec::Vec<u32>,
    pub app: AppKind,
}

impl Window {
    #[must_use]
    pub fn new(id: u32, title: &str, x: i32, y: i32, width: usize, height: usize, app: AppKind) -> Self {
        let content_width = width.saturating_sub(2);
        let content_height = height
            .saturating_sub(Theme::WINDOW_TITLE_HEIGHT)
            .saturating_sub(2);
        Self {
            id,
            title: title.to_string(),
            x,
            y,
            width,
            height,
            focused: false,
            visible: true,
            dragging: false,
            drag_offset_x: 0,
            drag_offset_y: 0,
            content: make_buffer(content_width, content_height),
            app,
        }
    }

    #[must_use]
    pub fn app_type(&self) -> super::apps::AppType {
        self.app.app_type()
    }

    #[must_use]
    pub fn content_width(&self) -> usize {
        self.width.saturating_sub(2)
    }

    #[must_use]
    pub fn content_height(&self) -> usize {
        self.height
            .saturating_sub(Theme::WINDOW_TITLE_HEIGHT)
            .saturating_sub(2)
    }

    pub fn render(&mut self, surface: &mut Surface<'_>) {
        if !self.visible {
            return;
        }

        let x = self.x.max(0) as usize;
        let y = self.y.max(0) as usize;
        let border = if self.focused {
            Theme::WINDOW_BORDER_FOCUSED
        } else {
            Theme::WINDOW_BORDER
        };

        surface.draw_rect(x, y, self.width, self.height, border);
        surface.fill_rect(
            x + 1,
            y + 1,
            self.width.saturating_sub(2),
            Theme::WINDOW_TITLE_HEIGHT,
            Theme::WINDOW_TITLE_BG,
        );
        surface.fill_rect(
            x + 1,
            y + Theme::WINDOW_TITLE_HEIGHT + 1,
            self.width.saturating_sub(2),
            self.height.saturating_sub(Theme::WINDOW_TITLE_HEIGHT + 2),
            Theme::WINDOW_BG,
        );
        surface.fill_rect(x + 8, y + 8, 12, 12, Theme::WINDOW_CLOSE_BG);
        font::draw_text(surface, x + 30, y + 7, &self.title, Theme::WINDOW_TITLE_TEXT);

        let content_width = self.content_width();
        let content_height = self.content_height();
        self.app
            .render(&mut self.content, content_width, content_height);
        surface.blit(
            &self.content,
            content_width,
            content_height,
            x + 1,
            y + Theme::WINDOW_TITLE_HEIGHT + 1,
        );
    }

    #[must_use]
    pub fn contains(&self, mouse_x: i32, mouse_y: i32) -> bool {
        mouse_x >= self.x
            && mouse_x < self.x + self.width as i32
            && mouse_y >= self.y
            && mouse_y < self.y + self.height as i32
    }

    #[must_use]
    pub fn title_bar_contains(&self, mouse_x: i32, mouse_y: i32) -> bool {
        mouse_x >= self.x
            && mouse_x < self.x + self.width as i32
            && mouse_y >= self.y
            && mouse_y < self.y + Theme::WINDOW_TITLE_HEIGHT as i32
    }

    #[must_use]
    pub fn close_button_contains(&self, mouse_x: i32, mouse_y: i32) -> bool {
        mouse_x >= self.x + 8
            && mouse_x < self.x + 20
            && mouse_y >= self.y + 8
            && mouse_y < self.y + 20
    }
}
