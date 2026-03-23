use super::framebuffer::Color;

pub struct Theme;

impl Theme {
    pub const DESKTOP_BG: Color = Color::from_hex(0x0D1117);
    pub const DESKTOP_BG_BOTTOM: Color = Color::from_hex(0x0A0E14);
    pub const DESKTOP_PATTERN: Color = Color::from_hex(0x161B22);
    pub const DESKTOP_WATERMARK: Color = Color::from_hex(0x161B22);

    pub const TASKBAR_BG: Color = Color::from_hex(0x010409);
    pub const TASKBAR_BORDER: Color = Color::from_hex(0x21262D);
    pub const TASKBAR_TEXT: Color = Color::from_hex(0xE6EDF3);
    pub const TASKBAR_ACCENT: Color = Color::from_hex(0x3FB950);
    pub const TASKBAR_HEIGHT: usize = 34;

    pub const WINDOW_BG: Color = Color::from_hex(0x0D1117);
    pub const WINDOW_BORDER: Color = Color::from_hex(0x21262D);
    pub const WINDOW_BORDER_GLOW: Color = Color::from_hex(0x1F6FEB);
    pub const WINDOW_BORDER_FOCUSED: Color = Color::from_hex(0x58A6FF);
    pub const WINDOW_TITLE_BG: Color = Color::from_hex(0x161B22);
    pub const WINDOW_TITLE_BG_TOP: Color = Color::from_hex(0x1C2128);
    pub const WINDOW_TITLE_BG_BOTTOM: Color = Color::from_hex(0x161B22);
    pub const WINDOW_SEPARATOR: Color = Color::from_hex(0x30363D);
    pub const WINDOW_TITLE_TEXT: Color = Color::from_hex(0xE6EDF3);
    pub const WINDOW_TITLE_HEIGHT: usize = 30;
    pub const WINDOW_CLOSE_BG: Color = Color::from_hex(0xFF7B72);
    pub const WINDOW_CLOSE_HOVER: Color = Color::from_hex(0xFF938A);
    pub const WINDOW_PADDING: usize = 8;
    pub const WINDOW_RESIZE_HANDLE: Color = Color::from_hex(0x484F58);

    pub const TERMINAL_BG: Color = Color::from_hex(0x0D1117);
    pub const TERMINAL_TEXT: Color = Color::from_hex(0xE6EDF3);
    pub const TERMINAL_OUTPUT: Color = Color::from_hex(0xC9D1D9);
    pub const TERMINAL_ERROR: Color = Color::from_hex(0xFF7B72);
    pub const TERMINAL_CURSOR: Color = Color::from_hex(0x3FB950);
    pub const TERMINAL_PROMPT: Color = Color::from_hex(0x56D4DD);
    pub const TERMINAL_SCROLL_TRACK: Color = Color::from_hex(0x30363D);
    pub const TERMINAL_SCROLL_THUMB: Color = Color::from_hex(0x484F58);

    pub const BUTTON_BG: Color = Color::from_hex(0x21262D);
    pub const BUTTON_HOVER: Color = Color::from_hex(0x30363D);
    pub const BUTTON_ACTIVE: Color = Color::from_hex(0x0D419D);
    pub const BUTTON_TEXT: Color = Color::from_hex(0xE6EDF3);
    pub const BUTTON_TEXT_DIM: Color = Color::from_hex(0x8B949E);
    pub const BUTTON_ACCENT: Color = Color::from_hex(0x238636);
    pub const BUTTON_ACCENT_HOVER: Color = Color::from_hex(0x2EA043);

    pub const CURSOR_COLOR: Color = Color::from_hex(0xFFFFFF);
    pub const CURSOR_BORDER: Color = Color::from_hex(0x000000);
    pub const CURSOR_SHADOW: Color = Color::from_hex(0x30363D);

    pub const TEXT_PRIMARY: Color = Color::from_hex(0xE6EDF3);
    pub const TEXT_SECONDARY: Color = Color::from_hex(0x8B949E);
    pub const TEXT_ACCENT: Color = Color::from_hex(0x58A6FF);
    pub const TEXT_MUTED: Color = Color::from_hex(0x6E7681);

    pub const QUANTUM_GATE: Color = Color::from_hex(0x79C0FF);
    pub const QUANTUM_WIRE: Color = Color::from_hex(0x56D4DD);
    pub const QUANTUM_RESULT: Color = Color::from_hex(0x3FB950);

    pub const CONTEXT_MENU_BG: Color = Color::from_hex(0x161B22);
    pub const CONTEXT_MENU_BORDER: Color = Color::from_hex(0x30363D);
    pub const CONTEXT_MENU_HOVER: Color = Color::from_hex(0x21262D);
}
