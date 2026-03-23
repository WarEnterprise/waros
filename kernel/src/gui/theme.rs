use super::framebuffer::Color;

pub struct Theme;

impl Theme {
    pub const DESKTOP_BG: Color = Color::from_hex(0x0D1117);
    pub const DESKTOP_PATTERN: Color = Color::from_hex(0x161B22);

    pub const TASKBAR_BG: Color = Color::from_hex(0x010409);
    pub const TASKBAR_TEXT: Color = Color::from_hex(0xE6EDF3);
    pub const TASKBAR_ACCENT: Color = Color::from_hex(0x3FB950);
    pub const TASKBAR_HEIGHT: usize = 32;

    pub const WINDOW_BG: Color = Color::from_hex(0x0D1117);
    pub const WINDOW_BORDER: Color = Color::from_hex(0x30363D);
    pub const WINDOW_BORDER_FOCUSED: Color = Color::from_hex(0x58A6FF);
    pub const WINDOW_TITLE_BG: Color = Color::from_hex(0x161B22);
    pub const WINDOW_TITLE_TEXT: Color = Color::from_hex(0xE6EDF3);
    pub const WINDOW_TITLE_HEIGHT: usize = 28;
    pub const WINDOW_CLOSE_BG: Color = Color::from_hex(0xFF7B72);

    pub const TERMINAL_BG: Color = Color::from_hex(0x0D1117);
    pub const TERMINAL_TEXT: Color = Color::from_hex(0xE6EDF3);
    pub const TERMINAL_CURSOR: Color = Color::from_hex(0x3FB950);
    pub const TERMINAL_PROMPT: Color = Color::from_hex(0x56D4DD);

    pub const BUTTON_BG: Color = Color::from_hex(0x21262D);
    pub const BUTTON_TEXT: Color = Color::from_hex(0xE6EDF3);
    pub const BUTTON_ACCENT: Color = Color::from_hex(0x238636);

    pub const CURSOR_COLOR: Color = Color::from_hex(0xFFFFFF);
    pub const CURSOR_BORDER: Color = Color::from_hex(0x000000);

    pub const TEXT_PRIMARY: Color = Color::from_hex(0xE6EDF3);
    pub const TEXT_SECONDARY: Color = Color::from_hex(0x8B949E);
    pub const TEXT_ACCENT: Color = Color::from_hex(0x58A6FF);

    pub const QUANTUM_GATE: Color = Color::from_hex(0x79C0FF);
    pub const QUANTUM_WIRE: Color = Color::from_hex(0x56D4DD);
    pub const QUANTUM_RESULT: Color = Color::from_hex(0x3FB950);
}
