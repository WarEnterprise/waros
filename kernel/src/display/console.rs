use core::fmt::{self, Write};

use bootloader_api::info::FrameBuffer;
use spin::Mutex;
use x86_64::instructions::interrupts;

use crate::display::font;
use crate::display::framebuffer::Framebuffer;

pub const BACKGROUND_COLOR: u32 = 0x001a1a2e;
pub const FOREGROUND_COLOR: u32 = 0x0000ff41;
pub const ACCENT_COLOR: u32 = 0x00e94560;
pub const OK_COLOR: u32 = 0x0016c60c;
pub const ERROR_COLOR: u32 = 0x00ff0000;
pub const INFO_COLOR: u32 = 0x000f3460;

pub static CONSOLE: Mutex<Option<FramebufferConsole>> = Mutex::new(None);

/// Text console that renders glyphs into the framebuffer.
pub struct FramebufferConsole {
    framebuffer: Framebuffer,
    cursor_col: usize,
    cursor_row: usize,
    cols: usize,
    rows: usize,
    fg_color: u32,
    bg_color: u32,
}

impl FramebufferConsole {
    /// Construct a new framebuffer-backed text console.
    #[must_use]
    pub fn new(framebuffer: &'static mut FrameBuffer) -> Self {
        let framebuffer = Framebuffer::new(framebuffer);
        let info = framebuffer.info();
        Self {
            cols: info.width / font::FONT_WIDTH,
            rows: info.height / font::FONT_HEIGHT_PIXELS,
            framebuffer,
            cursor_col: 0,
            cursor_row: 0,
            fg_color: FOREGROUND_COLOR,
            bg_color: BACKGROUND_COLOR,
        }
    }

    /// Initialize the framebuffer console singleton.
    pub fn init(framebuffer: &'static mut FrameBuffer) {
        let mut console = Self::new(framebuffer);
        console.clear_screen();
        *CONSOLE.lock() = Some(console);
    }

    /// Set the active text colors.
    pub fn set_colors(&mut self, fg_color: u32, bg_color: u32) {
        self.fg_color = fg_color;
        self.bg_color = bg_color;
    }

    /// Restore the default WarOS terminal palette.
    pub fn reset_colors(&mut self) {
        self.set_colors(FOREGROUND_COLOR, BACKGROUND_COLOR);
    }

    /// Clear the full screen and reset the cursor to the origin.
    pub fn clear_screen(&mut self) {
        self.framebuffer.clear(self.bg_color);
        self.cursor_col = 0;
        self.cursor_row = 0;
    }

    /// Erase one character cell to support line editing.
    pub fn backspace(&mut self) {
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
        } else if self.cursor_row > 0 {
            self.cursor_row -= 1;
            self.cursor_col = self.cols.saturating_sub(1);
        } else {
            return;
        }

        self.clear_cell(self.cursor_col, self.cursor_row);
    }

    fn clear_cell(&mut self, col: usize, row: usize) {
        let x_start = col * font::FONT_WIDTH;
        let y_start = row * font::FONT_HEIGHT_PIXELS;
        for y in y_start..(y_start + font::FONT_HEIGHT_PIXELS) {
            for x in x_start..(x_start + font::FONT_WIDTH) {
                self.framebuffer.write_pixel(x, y, self.bg_color);
            }
        }
    }

    fn newline(&mut self) {
        self.cursor_col = 0;
        if self.cursor_row + 1 >= self.rows {
            self.framebuffer.scroll_up(font::FONT_HEIGHT_PIXELS, self.bg_color);
        } else {
            self.cursor_row += 1;
        }
    }

    fn draw_character(&mut self, character: char) {
        if self.cursor_col >= self.cols {
            self.newline();
        }

        let glyph = font::glyph(character);
        let x_offset = self.cursor_col * font::FONT_WIDTH;
        let y_offset = self.cursor_row * font::FONT_HEIGHT_PIXELS;

        for (row_index, row) in glyph.raster().iter().enumerate() {
            for (column_index, intensity) in row.iter().copied().enumerate() {
                let color = blend(self.bg_color, self.fg_color, intensity);
                self.framebuffer
                    .write_pixel(x_offset + column_index, y_offset + row_index, color);
            }
        }

        self.cursor_col += 1;
        if self.cursor_col >= self.cols {
            self.newline();
        }
    }

    fn write_byte(&mut self, byte: u8) {
        match byte {
            b'\n' => self.newline(),
            b'\r' => self.cursor_col = 0,
            0x08 => self.backspace(),
            byte if byte.is_ascii_graphic() || byte == b' ' => {
                self.draw_character(char::from(byte));
            }
            _ => self.draw_character(' '),
        }
    }
}

impl Write for FramebufferConsole {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        for byte in text.bytes() {
            self.write_byte(byte);
        }
        Ok(())
    }
}

/// Initialize the global framebuffer console.
pub fn init(framebuffer: &'static mut FrameBuffer) {
    FramebufferConsole::init(framebuffer);
}

/// Access the framebuffer console if it has been initialized.
pub fn with_console<R>(function: impl FnOnce(&mut FramebufferConsole) -> R) -> Option<R> {
    let mut guard = CONSOLE.lock();
    guard.as_mut().map(function)
}

/// Attempt to print without blocking if the console is currently locked.
pub fn try_write_fmt(args: fmt::Arguments<'_>) {
    if let Some(mut guard) = CONSOLE.try_lock() {
        if let Some(console) = guard.as_mut() {
            let _ = console.write_fmt(args);
        }
    }
}

/// Clear the screen using the active background color.
pub fn clear_screen() {
    let _ = with_console(FramebufferConsole::clear_screen);
}

/// Remove one character from the visible line buffer.
pub fn backspace() {
    let _ = with_console(FramebufferConsole::backspace);
}

/// Set the active text colors for subsequent output.
pub fn set_colors(fg_color: u32, bg_color: u32) {
    let _ = with_console(|console| console.set_colors(fg_color, bg_color));
}

/// Restore the default text colors.
pub fn reset_colors() {
    let _ = with_console(FramebufferConsole::reset_colors);
}

/// Print formatted text to the framebuffer console.
pub fn _print(args: fmt::Arguments<'_>) {
    interrupts::without_interrupts(|| {
        let mut guard = CONSOLE.lock();
        if let Some(console) = guard.as_mut() {
            let _ = console.write_fmt(args);
        }
    });
}

fn blend(background: u32, foreground: u32, alpha: u8) -> u32 {
    let background_bytes = background.to_be_bytes();
    let foreground_bytes = foreground.to_be_bytes();
    let alpha = u16::from(alpha);
    let inverse_alpha = u16::from(u8::MAX) - alpha;

    let red =
        ((u16::from(background_bytes[1]) * inverse_alpha) + (u16::from(foreground_bytes[1]) * alpha))
            / u16::from(u8::MAX);
    let green =
        ((u16::from(background_bytes[2]) * inverse_alpha) + (u16::from(foreground_bytes[2]) * alpha))
            / u16::from(u8::MAX);
    let blue =
        ((u16::from(background_bytes[3]) * inverse_alpha) + (u16::from(foreground_bytes[3]) * alpha))
            / u16::from(u8::MAX);

    u32::from_be_bytes([0, red as u8, green as u8, blue as u8])
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        $crate::display::console::_print(format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! println {
    () => {
        $crate::print!("\n")
    };
    ($($arg:tt)*) => {
        $crate::display::console::_print(format_args!("{}\n", format_args!($($arg)*)))
    };
}
