use alloc::vec;
use alloc::vec::Vec;

use super::apps::{AppKind, AppType};
use super::cursor;
use super::desktop;
use super::framebuffer::{flush_regions_to_screen, make_buffer, Rect, Surface};
use super::mouse;
use super::taskbar;
use super::theme::Theme;
use super::window::Window;

const CURSOR_DIRTY_SIZE: usize = 18;
const SNAP_DISTANCE: i32 = 10;

const DESKTOP_MENU: [ContextAction; 5] = [
    ContextAction::NewTerminal,
    ContextAction::SystemInfo,
    ContextAction::QuantumMonitor,
    ContextAction::AboutWarOS,
    ContextAction::ReturnToShell,
];

const TERMINAL_MENU: [ContextAction; 4] = [
    ContextAction::Copy,
    ContextAction::Paste,
    ContextAction::ClearTerminal,
    ContextAction::ReturnToShell,
];

pub struct Compositor {
    windows: Vec<Window>,
    next_id: u32,
    focused_id: Option<u32>,
    back_buffer: Vec<u32>,
    screen_width: usize,
    screen_height: usize,
    needs_redraw: bool,
    dirty_regions: Vec<Rect>,
    context_menu: Option<ContextMenu>,
    last_cursor: Option<(i32, i32)>,
    exit_to_shell: bool,
}

#[derive(Clone, Copy)]
enum ContextAction {
    NewTerminal,
    SystemInfo,
    QuantumMonitor,
    AboutWarOS,
    ReturnToShell,
    Copy,
    Paste,
    ClearTerminal,
}

struct ContextMenu {
    x: usize,
    y: usize,
    width: usize,
    item_height: usize,
    separator_before: Option<usize>,
    actions: &'static [ContextAction],
    target: ContextMenuTarget,
}

#[derive(Clone, Copy)]
enum ContextMenuTarget {
    Desktop,
    Terminal(u32),
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
            dirty_regions: vec![Rect {
                x: 0,
                y: 0,
                width,
                height,
            }],
            context_menu: None,
            last_cursor: None,
            exit_to_shell: false,
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
        let width = width.clamp(200, self.screen_width.saturating_sub(16));
        let height = height.clamp(150, self.screen_height.saturating_sub(Theme::TASKBAR_HEIGHT + 16));
        let mut window = Window::new(
            self.next_id,
            app_type.title(),
            x,
            y.max(Theme::TASKBAR_HEIGHT as i32 + 4),
            width,
            height,
            AppKind::new(app_type, width, height),
        );
        window.focused = true;
        window.flash_frames = 2;
        for existing in &mut self.windows {
            existing.focused = false;
        }
        let id = window.id;
        self.next_id += 1;
        self.focused_id = Some(id);
        self.windows.push(window);
        self.context_menu = None;
        self.invalidate();
        id
    }

    pub fn handle_mouse_down(&mut self, mouse_x: i32, mouse_y: i32) {
        if self.activate_context_menu(mouse_x, mouse_y) {
            return;
        }

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
                self.context_menu = None;
                self.focus_window(window_id);
                if let Some(window) = self.windows.iter_mut().find(|window| window.id == window_id) {
                    if window.title_bar_contains(mouse_x, mouse_y) {
                        window.dragging = true;
                        window.drag_offset_x = mouse_x - window.x;
                        window.drag_offset_y = mouse_y - window.y;
                    }
                }
                self.invalidate();
                return;
            }
        }

        self.context_menu = None;
        self.invalidate();
    }

    pub fn handle_right_click(&mut self, mouse_x: i32, mouse_y: i32) {
        let mut target = ContextMenuTarget::Desktop;

        for index in (0..self.windows.len()).rev() {
            let window = &self.windows[index];
            if window.contains(mouse_x, mouse_y) {
                if window.app.is_terminal() {
                    target = ContextMenuTarget::Terminal(window.id);
                } else {
                    return;
                }
                break;
            }
        }

        let actions = match target {
            ContextMenuTarget::Desktop => &DESKTOP_MENU[..],
            ContextMenuTarget::Terminal(_) => &TERMINAL_MENU[..],
        };
        let separator_before = match target {
            ContextMenuTarget::Desktop => Some(3),
            ContextMenuTarget::Terminal(_) => Some(2),
        };
        self.context_menu = Some(ContextMenu {
            x: mouse_x.max(0) as usize,
            y: mouse_y.max(Theme::TASKBAR_HEIGHT as i32) as usize,
            width: 176,
            item_height: 20,
            separator_before,
            actions,
            target,
        });
        self.invalidate();
    }

    pub fn handle_mouse_move(&mut self, mouse_x: i32, mouse_y: i32) {
        let mut moved_window = false;
        for window in &mut self.windows {
            if window.dragging {
                let max_x = (self.screen_width.saturating_sub(window.width)) as i32;
                let max_y = (self.screen_height.saturating_sub(window.height)) as i32;
                let mut x = (mouse_x - window.drag_offset_x).clamp(0, max_x);
                let mut y = (mouse_y - window.drag_offset_y).clamp(Theme::TASKBAR_HEIGHT as i32, max_y);
                if x <= SNAP_DISTANCE {
                    x = 0;
                }
                if y <= Theme::TASKBAR_HEIGHT as i32 + SNAP_DISTANCE {
                    y = Theme::TASKBAR_HEIGHT as i32;
                }
                if (max_x - x).abs() <= SNAP_DISTANCE {
                    x = max_x;
                }
                if (max_y - y).abs() <= SNAP_DISTANCE {
                    y = max_y;
                }
                window.x = x;
                window.y = y;
                moved_window = true;
            }
        }

        self.mark_cursor_dirty(mouse_x, mouse_y);
        if moved_window || self.context_menu.is_some() || mouse_y <= Theme::TASKBAR_HEIGHT as i32 {
            self.invalidate();
        }
    }

    pub fn handle_mouse_up(&mut self) {
        for window in &mut self.windows {
            window.dragging = false;
        }
    }

    pub fn route_key_to_focused(&mut self, key: u8) {
        if let Some(focused_id) = self.focused_id {
            let mut dirty = None;
            if let Some(window) = self.windows.iter_mut().find(|window| window.id == focused_id) {
                if window.app.handle_key(key) {
                    dirty = Some(window_rect(window));
                }
            }
            if let Some(region) = dirty {
                self.mark_dirty(region);
                return;
            }
        }
        self.invalidate();
    }

    pub fn invalidate(&mut self) {
        self.needs_redraw = true;
        self.dirty_regions.clear();
        self.dirty_regions.push(Rect {
            x: 0,
            y: 0,
            width: self.screen_width,
            height: self.screen_height,
        });
    }

    #[must_use]
    pub fn needs_redraw(&self) -> bool {
        self.needs_redraw || !self.dirty_regions.is_empty()
    }

    #[must_use]
    pub fn should_exit_to_shell(&self) -> bool {
        self.exit_to_shell
    }

    pub fn render(&mut self) {
        if !self.needs_redraw() {
            return;
        }

        let mouse = mouse::current_snapshot();
        let mut surface = Surface::new(&mut self.back_buffer, self.screen_width, self.screen_height);
        desktop::render_desktop(&mut surface);

        for window in &mut self.windows {
            window.render(&mut surface, mouse.x, mouse.y);
        }

        let active_apps: Vec<AppType> = self.windows.iter().map(Window::app_type).collect();
        taskbar::render_taskbar(&mut surface, self.screen_width, &active_apps);

        if let Some(context_menu) = &self.context_menu {
            render_context_menu(&mut surface, context_menu, mouse.x, mouse.y);
        }

        cursor::render_cursor(&mut surface, mouse.x.max(0) as usize, mouse.y.max(0) as usize);

        flush_regions_to_screen(surface.pixels(), self.screen_width, self.screen_height, &self.dirty_regions);
        self.dirty_regions.clear();
        self.needs_redraw = false;
    }

    fn focus_window(&mut self, id: u32) {
        for window in &mut self.windows {
            window.focused = window.id == id;
        }
        self.focused_id = Some(id);
        if let Some(index) = self.windows.iter().position(|window| window.id == id) {
            let mut window = self.windows.remove(index);
            window.flash_frames = 2;
            self.windows.push(window);
        }
        self.invalidate();
    }

    fn close_window(&mut self, id: u32) {
        self.windows.retain(|window| window.id != id);
        self.context_menu = None;
        self.focused_id = self.windows.last().map(|window| window.id);
        if let Some(focused_id) = self.focused_id {
            for window in &mut self.windows {
                window.focused = window.id == focused_id;
            }
        }
        self.invalidate();
    }

    fn mark_dirty(&mut self, region: Rect) {
        self.needs_redraw = true;
        self.dirty_regions.push(region);
    }

    fn mark_cursor_dirty(&mut self, mouse_x: i32, mouse_y: i32) {
        if let Some((old_x, old_y)) = self.last_cursor {
            self.dirty_regions.push(cursor_rect(old_x, old_y));
        }
        self.dirty_regions.push(cursor_rect(mouse_x, mouse_y));
        self.last_cursor = Some((mouse_x, mouse_y));
    }

    fn activate_context_menu(&mut self, mouse_x: i32, mouse_y: i32) -> bool {
        let Some(menu) = &self.context_menu else {
            return false;
        };

        let action = context_menu_hit_test(menu, mouse_x, mouse_y);
        if let Some(action) = action {
            let target = menu.target;
            self.context_menu = None;
            self.apply_context_action(action, target);
            return true;
        }

        if point_in_menu(menu, mouse_x, mouse_y) {
            self.context_menu = None;
            self.invalidate();
            return true;
        }

        false
    }

    fn apply_context_action(&mut self, action: ContextAction, target: ContextMenuTarget) {
        match action {
            ContextAction::NewTerminal => {
                self.open_app(AppType::Terminal);
            }
            ContextAction::SystemInfo | ContextAction::AboutWarOS => {
                self.open_app(AppType::SystemInfo);
            }
            ContextAction::QuantumMonitor => {
                self.open_app(AppType::Quantum);
            }
            ContextAction::ReturnToShell => {
                self.exit_to_shell = true;
            }
            ContextAction::ClearTerminal => {
                if let ContextMenuTarget::Terminal(window_id) = target {
                    if let Some(window) = self.windows.iter_mut().find(|window| window.id == window_id) {
                        window.app.clear_terminal();
                    }
                    self.invalidate();
                }
            }
            ContextAction::Copy | ContextAction::Paste => {}
        }
    }
}

fn render_context_menu(surface: &mut Surface<'_>, menu: &ContextMenu, mouse_x: i32, mouse_y: i32) {
    let height = context_menu_height(menu);
    surface.fill_rounded_rect(menu.x, menu.y, menu.width, height, 4, Theme::CONTEXT_MENU_BG);
    surface.draw_rounded_rect(menu.x, menu.y, menu.width, height, 4, Theme::CONTEXT_MENU_BORDER);

    for (index, action) in menu.actions.iter().enumerate() {
        let item_rect = context_menu_item_rect(menu, index);
        if let Some(separator) = menu.separator_before {
            if separator == index {
                surface.draw_hline(menu.x + 8, item_rect.y - 3, menu.width.saturating_sub(16), Theme::WINDOW_SEPARATOR);
            }
        }

        let hovered = mouse_x >= item_rect.x as i32
            && mouse_x < (item_rect.x + item_rect.width) as i32
            && mouse_y >= item_rect.y as i32
            && mouse_y < (item_rect.y + item_rect.height) as i32;
        if hovered {
            surface.fill_rounded_rect(
                item_rect.x + 4,
                item_rect.y + 1,
                item_rect.width.saturating_sub(8),
                item_rect.height.saturating_sub(2),
                3,
                Theme::CONTEXT_MENU_HOVER,
            );
        }

        super::font::draw_text(
            surface,
            item_rect.x + 10,
            item_rect.y + 2,
            context_action_label(*action),
            Theme::TEXT_PRIMARY,
        );
    }
}

fn context_action_label(action: ContextAction) -> &'static str {
    match action {
        ContextAction::NewTerminal => "New Terminal",
        ContextAction::SystemInfo => "System Info",
        ContextAction::QuantumMonitor => "Quantum Monitor",
        ContextAction::AboutWarOS => "About WarOS",
        ContextAction::ReturnToShell => "Return to Shell",
        ContextAction::Copy => "Copy",
        ContextAction::Paste => "Paste",
        ContextAction::ClearTerminal => "Clear Terminal",
    }
}

fn point_in_menu(menu: &ContextMenu, mouse_x: i32, mouse_y: i32) -> bool {
    mouse_x >= menu.x as i32
        && mouse_x < (menu.x + menu.width) as i32
        && mouse_y >= menu.y as i32
        && mouse_y < (menu.y + context_menu_height(menu)) as i32
}

fn context_menu_hit_test(menu: &ContextMenu, mouse_x: i32, mouse_y: i32) -> Option<ContextAction> {
    for (index, action) in menu.actions.iter().enumerate() {
        let rect = context_menu_item_rect(menu, index);
        if mouse_x >= rect.x as i32
            && mouse_x < (rect.x + rect.width) as i32
            && mouse_y >= rect.y as i32
            && mouse_y < (rect.y + rect.height) as i32
        {
            return Some(*action);
        }
    }
    None
}

fn context_menu_height(menu: &ContextMenu) -> usize {
    let separator = if menu.separator_before.is_some() { 6 } else { 0 };
    menu.actions.len() * menu.item_height + 8 + separator
}

fn context_menu_item_rect(menu: &ContextMenu, index: usize) -> Rect {
    let mut y = menu.y + 4 + index * menu.item_height;
    if let Some(separator) = menu.separator_before {
        if index >= separator {
            y += 6;
        }
    }
    Rect {
        x: menu.x,
        y,
        width: menu.width,
        height: menu.item_height,
    }
}

fn cursor_rect(x: i32, y: i32) -> Rect {
    Rect {
        x: x.max(0) as usize,
        y: y.max(0) as usize,
        width: CURSOR_DIRTY_SIZE,
        height: CURSOR_DIRTY_SIZE,
    }
}

fn window_rect(window: &Window) -> Rect {
    Rect {
        x: window.x.max(0) as usize,
        y: window.y.max(0) as usize,
        width: window.width + 2,
        height: window.height + 2,
    }
}
