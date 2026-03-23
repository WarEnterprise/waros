use alloc::vec::Vec;

use super::apps::{AppKind, AppType};
use super::cursor;
use super::desktop;
use super::framebuffer::{flush_to_screen, make_buffer, Surface};
use super::mouse;
use super::taskbar;
use super::theme::Theme;
use super::window::Window;

pub struct Compositor {
    windows: Vec<Window>,
    next_id: u32,
    focused_id: Option<u32>,
    back_buffer: Vec<u32>,
    screen_width: usize,
    screen_height: usize,
    needs_redraw: bool,
}

impl Compositor {
    #[must_use]
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            windows: Vec::new(),
            next_id: 1,
            focused_id: None,
            back_buffer: make_buffer(width, height),
            screen_width: width,
            screen_height: height,
            needs_redraw: true,
        }
    }

    pub fn open_app(&mut self, app_type: AppType) -> u32 {
        if let Some(existing_id) = self
            .windows
            .iter()
            .find(|window| window.app_type() == app_type)
            .map(|window| window.id)
        {
            self.focus_window(existing_id);
            return existing_id;
        }

        let (x, y, width, height) = app_type.default_geometry();
        let mut window = Window::new(
            self.next_id,
            app_type.title(),
            x,
            y.max(Theme::TASKBAR_HEIGHT as i32 + 4),
            width.min(self.screen_width.saturating_sub(16)),
            height.min(self.screen_height.saturating_sub(Theme::TASKBAR_HEIGHT + 16)),
            AppKind::new(app_type, width, height),
        );
        window.focused = true;
        for existing in &mut self.windows {
            existing.focused = false;
        }
        let id = window.id;
        self.next_id += 1;
        self.focused_id = Some(id);
        self.windows.push(window);
        self.needs_redraw = true;
        id
    }

    pub fn handle_mouse_down(&mut self, mouse_x: i32, mouse_y: i32) {
        if let Some(app) = taskbar::launcher_hit_test(mouse_x, mouse_y) {
            self.open_app(app);
            return;
        }

        for index in (0..self.windows.len()).rev() {
            let window_id = self.windows[index].id;
            if self.windows[index].close_button_contains(mouse_x, mouse_y) {
                self.close_window(window_id);
                return;
            }
            if self.windows[index].contains(mouse_x, mouse_y) {
                self.focus_window(window_id);
                if let Some(window) = self.windows.iter_mut().find(|window| window.id == window_id) {
                    if window.title_bar_contains(mouse_x, mouse_y) {
                        window.dragging = true;
                        window.drag_offset_x = mouse_x - window.x;
                        window.drag_offset_y = mouse_y - window.y;
                    }
                }
                self.needs_redraw = true;
                return;
            }
        }
    }

    pub fn handle_mouse_move(&mut self, mouse_x: i32, mouse_y: i32) {
        let mut moved = false;
        for window in &mut self.windows {
            if window.dragging {
                window.x = (mouse_x - window.drag_offset_x)
                    .clamp(0, (self.screen_width.saturating_sub(window.width)) as i32);
                window.y = (mouse_y - window.drag_offset_y).clamp(
                    Theme::TASKBAR_HEIGHT as i32,
                    (self.screen_height.saturating_sub(window.height)) as i32,
                );
                moved = true;
            }
        }
        self.needs_redraw |= moved;
    }

    pub fn handle_mouse_up(&mut self) {
        for window in &mut self.windows {
            window.dragging = false;
        }
    }

    pub fn route_key_to_focused(&mut self, key: u8) {
        if let Some(focused_id) = self.focused_id {
            if let Some(window) = self.windows.iter_mut().find(|window| window.id == focused_id) {
                if window.app.handle_key(key) {
                    self.needs_redraw = true;
                }
            }
        }
    }

    pub fn invalidate(&mut self) {
        self.needs_redraw = true;
    }

    #[must_use]
    pub fn needs_redraw(&self) -> bool {
        self.needs_redraw
    }

    pub fn render(&mut self) {
        if !self.needs_redraw {
            return;
        }

        let mut surface = Surface::new(&mut self.back_buffer, self.screen_width, self.screen_height);
        desktop::render_desktop(&mut surface);

        for window in &mut self.windows {
            window.render(&mut surface);
        }

        let active_apps: Vec<AppType> = self.windows.iter().map(Window::app_type).collect();
        taskbar::render_taskbar(&mut surface, self.screen_width, &active_apps);

        let mouse = mouse::take_snapshot();
        cursor::render_cursor(&mut surface, mouse.x.max(0) as usize, mouse.y.max(0) as usize);

        flush_to_screen(surface.pixels(), self.screen_width, self.screen_height);
        self.needs_redraw = false;
    }

    fn focus_window(&mut self, id: u32) {
        for window in &mut self.windows {
            window.focused = window.id == id;
        }
        self.focused_id = Some(id);
        if let Some(index) = self.windows.iter().position(|window| window.id == id) {
            let window = self.windows.remove(index);
            self.windows.push(window);
        }
    }

    fn close_window(&mut self, id: u32) {
        self.windows.retain(|window| window.id != id);
        self.focused_id = self.windows.last().map(|window| window.id);
        if let Some(focused_id) = self.focused_id {
            for window in &mut self.windows {
                window.focused = window.id == focused_id;
            }
        }
        self.needs_redraw = true;
    }
}
