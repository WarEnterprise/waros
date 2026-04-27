use alloc::string::String;
use core::fmt::{self, Write};
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU8, Ordering};

use bootloader_api::info::FrameBuffer;
use spin::{Lazy, Mutex};
use x86_64::instructions::interrupts;

use crate::display::font;
use crate::display::framebuffer::Framebuffer;

/// Shared framebuffer palette for WarOS text UI.
pub struct Colors;

impl Colors {
    pub const BG: u32 = 0x0D1117;
    pub const FG: u32 = 0xE6EDF3;
    pub const GREEN: u32 = 0x3FB950;
    pub const RED: u32 = 0xFF7B72;
    pub const BLUE: u32 = 0x79C0FF;
    pub const YELLOW: u32 = 0xD29922;
    pub const PURPLE: u32 = 0xD2A8FF;
    pub const DIM: u32 = 0x8B949E;
    pub const CYAN: u32 = 0x56D4DD;
}

pub static CONSOLE: Mutex<Option<FramebufferConsole>> = Mutex::new(None);
static RENDERING_ENABLED: AtomicBool = AtomicBool::new(true);
static CAPTURE_ACTIVE: AtomicBool = AtomicBool::new(false);
static CAPTURE_BUFFER: Lazy<Mutex<String>> = Lazy::new(|| Mutex::new(String::new()));
static INPUT_CURSOR_ENABLED: AtomicBool = AtomicBool::new(false);
static RAW_CONSOLE_PTR: AtomicPtr<FramebufferConsole> = AtomicPtr::new(core::ptr::null_mut());
static SCREEN_OWNER: AtomicU8 = AtomicU8::new(ScreenOwner::Boot as u8);
static SCREEN_TRACE_BUDGET: AtomicU32 = AtomicU32::new(128);
static CURSOR_TRACE_BUDGET: AtomicU32 = AtomicU32::new(192);
static SHELL_CLOBBER_COUNT: AtomicU32 = AtomicU32::new(0);
static SHELL_CLOBBER_OWNER: AtomicU8 = AtomicU8::new(ScreenOwner::Unknown as u8);
static SHELL_CLOBBER_KIND: AtomicU8 = AtomicU8::new(ShellClobberKind::None as u8);
const INPUT_CURSOR_BLINK_TICKS: u64 = 25;
const INPUT_CURSOR_BLINK_ENABLED: bool = false;
const SCREEN_TRACE_ENABLED: bool = true;
const CURSOR_TRACE_ENABLED: bool = true;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ScreenOwner {
    Unknown = 0,
    Boot = 1,
    Auth = 2,
    Shell = 3,
    Gui = 4,
    Panic = 5,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ShellClobberKind {
    None = 0,
    OwnerClaim = 1,
    ClearAttempt = 2,
}

#[derive(Clone, Copy, Debug)]
pub struct InputCursorSnapshot {
    pub row: usize,
    pub col: usize,
    pub enabled: bool,
    pub visible: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct ShellClobberSnapshot {
    pub count: u32,
    pub owner: ScreenOwner,
    pub kind: ShellClobberKind,
}

/// Text console that renders glyphs into the framebuffer.
pub struct FramebufferConsole {
    framebuffer: Framebuffer,
    cursor_col: usize,
    cursor_row: usize,
    cols: usize,
    rows: usize,
    fg_color: u32,
    bg_color: u32,
    input_cursor_visible: bool,
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
            fg_color: Colors::FG,
            bg_color: Colors::BG,
            input_cursor_visible: false,
        }
    }

    /// Initialize the framebuffer console singleton.
    pub fn init(framebuffer: &'static mut FrameBuffer) {
        let mut console = Self::new(framebuffer);
        console.clear_screen();
        let mut guard = CONSOLE.lock();
        *guard = Some(console);
        if let Some(console) = guard.as_mut() {
            RAW_CONSOLE_PTR.store(console as *mut _, Ordering::Release);
        }
    }

    /// Set the active text foreground color.
    pub fn set_color(&mut self, fg_color: u32) {
        self.fg_color = fg_color;
    }

    /// Restore the default foreground color.
    pub fn reset_color(&mut self) {
        self.set_color(Colors::FG);
    }

    #[must_use]
    pub fn width_pixels(&self) -> usize {
        self.framebuffer.info().width
    }

    #[must_use]
    pub fn height_pixels(&self) -> usize {
        self.framebuffer.info().height
    }

    pub fn write_pixel(&mut self, x: usize, y: usize, color: u32) {
        self.framebuffer.write_pixel(x, y, color);
    }

    pub(crate) fn raw_begin_batch(&mut self) {
        self.framebuffer.begin_batch();
    }

    pub(crate) fn raw_end_batch(&mut self) {
        self.framebuffer.end_batch();
    }

    pub(crate) fn raw_clear_screen_with_color(&mut self, color: u32) {
        self.framebuffer.clear(color);
    }

    /// Clear the full screen and reset the cursor to the origin.
    pub fn clear_screen(&mut self) {
        self.hide_input_cursor();
        self.framebuffer.begin_batch();
        self.framebuffer.clear(self.bg_color);
        self.framebuffer.end_batch();
        self.cursor_col = 0;
        self.cursor_row = 0;
    }

    /// Erase one character cell to support line editing.
    pub fn backspace(&mut self) {
        self.hide_input_cursor();
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
        } else if self.cursor_row > 0 {
            self.cursor_row -= 1;
            self.cursor_col = self.cols.saturating_sub(1);
        } else {
            return;
        }

        self.framebuffer.begin_batch();
        self.clear_cell(self.cursor_col, self.cursor_row);
        self.framebuffer.end_batch();
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
        self.hide_input_cursor();
        self.cursor_col = 0;
        if self.cursor_row + 1 >= self.rows {
            self.framebuffer
                .scroll_up(font::FONT_HEIGHT_PIXELS, self.bg_color);
        } else {
            self.cursor_row += 1;
        }
    }

    fn draw_character(&mut self, character: char) {
        self.hide_input_cursor();
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

    fn show_input_cursor(&mut self) {
        if self.input_cursor_visible || self.cursor_row >= self.rows {
            return;
        }

        self.framebuffer.begin_batch();
        let x_offset = self.cursor_col * font::FONT_WIDTH;
        let baseline = (self.cursor_row + 1) * font::FONT_HEIGHT_PIXELS;
        let y_start = baseline.saturating_sub(3);
        for y in y_start..baseline {
            for x in x_offset..(x_offset + font::FONT_WIDTH) {
                self.framebuffer.write_pixel(x, y, Colors::GREEN);
            }
        }
        self.framebuffer.end_batch();
        self.input_cursor_visible = true;
    }

    fn hide_input_cursor(&mut self) {
        if !self.input_cursor_visible || self.cursor_row >= self.rows {
            self.input_cursor_visible = false;
            return;
        }

        self.framebuffer.begin_batch();
        self.clear_cell(self.cursor_col, self.cursor_row);
        self.framebuffer.end_batch();
        self.input_cursor_visible = false;
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
        self.framebuffer.begin_batch();
        for byte in text.bytes() {
            self.write_byte(byte);
        }
        self.framebuffer.end_batch();
        Ok(())
    }
}

/// Initialize the global framebuffer console.
pub fn init(framebuffer: &'static mut FrameBuffer) {
    FramebufferConsole::init(framebuffer);
}

pub fn enable_shadow_buffer() -> Result<(), &'static str> {
    interrupts::without_interrupts(|| {
        let mut guard = CONSOLE.lock();
        let Some(console) = guard.as_mut() else {
            return Err("framebuffer console not initialized");
        };
        console.framebuffer.enable_shadow_buffer()
    })
}

/// Access the framebuffer console if it has been initialized.
pub fn with_console<R>(function: impl FnOnce(&mut FramebufferConsole) -> R) -> Option<R> {
    let mut guard = CONSOLE.lock();
    guard.as_mut().map(function)
}

pub(crate) fn with_raw_console<R>(function: impl FnOnce(&mut FramebufferConsole) -> R) -> Option<R> {
    interrupts::without_interrupts(|| {
        let pointer = RAW_CONSOLE_PTR.load(Ordering::Acquire);
        if pointer.is_null() {
            None
        } else {
            // SAFETY: WarOS runs a single CPU and stage tracing calls this path with
            // interrupts disabled, so nothing can concurrently move or mutate the
            // console while this raw diagnostic draw is active.
            Some(unsafe { function(&mut *pointer) })
        }
    })
}

pub fn raw_paint_debug_stage(stage: u8) {
    let pointer = RAW_CONSOLE_PTR.load(Ordering::Acquire);
    if pointer.is_null() {
        return;
    }

    let console = unsafe { &mut *pointer };
    let framebuffer = &mut console.framebuffer;
    let info = framebuffer.info();
    let color = match stage {
        1 => 0x00FF0000,
        2 => 0x0000FF00,
        _ => 0x000000FF,
    };
    let pattern_x = match stage {
        1 => 24usize,
        2 => 72usize,
        _ => 120usize,
    };

    framebuffer.begin_batch();
    framebuffer.clear(color);
    let max_x = info.width.min(pattern_x.saturating_add(16));
    let max_y = info.height.min(40);
    for y in 24usize..max_y {
        for x in pattern_x..max_x {
            framebuffer.write_pixel(x, y, 0x00FFFFFF);
        }
    }
    framebuffer.end_batch();
}

pub fn raw_paint_login_entry_test() {
    let pointer = RAW_CONSOLE_PTR.load(Ordering::Acquire);
    if pointer.is_null() {
        return;
    }

    let console = unsafe { &mut *pointer };
    let framebuffer = &mut console.framebuffer;
    let info = framebuffer.info();
    const BACKGROUND: u32 = 0x001F4A3A;
    const FOREGROUND: u32 = 0x00FFF6D5;
    const TEXT: &str = "LOGIN_ENTRY_TEST";
    const SCALE: usize = 5;
    let text_width = TEXT
        .as_bytes()
        .len()
        .saturating_mul(font::FONT_WIDTH)
        .saturating_mul(SCALE);
    let text_height = font::FONT_HEIGHT_PIXELS.saturating_mul(SCALE);
    let start_x = info.width.saturating_sub(text_width) / 2;
    let start_y = info.height.saturating_sub(text_height) / 2;

    framebuffer.begin_batch();
    framebuffer.clear(BACKGROUND);
    for y in 24usize..info.height.min(72) {
        for x in 24usize..info.width.min(72) {
            framebuffer.write_pixel(x, y, FOREGROUND);
        }
    }

    let mut x_cursor = start_x;
    for byte in TEXT.bytes() {
        let glyph = font::glyph(char::from(byte));
        for (row_index, row) in glyph.raster().iter().enumerate() {
            for (column_index, intensity) in row.iter().copied().enumerate() {
                if intensity == 0 {
                    continue;
                }

                let pixel_x = x_cursor.saturating_add(column_index.saturating_mul(SCALE));
                let pixel_y = start_y.saturating_add(row_index.saturating_mul(SCALE));
                for dy in 0..SCALE {
                    for dx in 0..SCALE {
                        framebuffer.write_pixel(
                            pixel_x.saturating_add(dx),
                            pixel_y.saturating_add(dy),
                            FOREGROUND,
                        );
                    }
                }
            }
        }
        x_cursor = x_cursor.saturating_add(font::FONT_WIDTH.saturating_mul(SCALE));
    }
    framebuffer.end_batch();
}

pub fn raw_paint_debug_main_after_auth() {
    let pointer = RAW_CONSOLE_PTR.load(Ordering::Acquire);
    if pointer.is_null() {
        return;
    }

    let console = unsafe { &mut *pointer };
    let framebuffer = &mut console.framebuffer;
    let info = framebuffer.info();
    const BACKGROUND: u32 = 0x00411018;
    const FOREGROUND: u32 = 0x00F7F1FF;
    const TEXT: &str = "MAIN_AFTER_AUTH";
    const SCALE: usize = 5;
    let text_width = TEXT
        .as_bytes()
        .len()
        .saturating_mul(font::FONT_WIDTH)
        .saturating_mul(SCALE);
    let text_height = font::FONT_HEIGHT_PIXELS.saturating_mul(SCALE);
    let start_x = info.width.saturating_sub(text_width) / 2;
    let start_y = info.height.saturating_sub(text_height) / 2;

    framebuffer.begin_batch();
    framebuffer.clear(BACKGROUND);
    for y in 24usize..info.height.min(72) {
        for x in info.width.saturating_sub(72)..info.width.saturating_sub(24) {
            framebuffer.write_pixel(x, y, FOREGROUND);
        }
    }

    let mut x_cursor = start_x;
    for byte in TEXT.bytes() {
        let glyph = font::glyph(char::from(byte));
        for (row_index, row) in glyph.raster().iter().enumerate() {
            for (column_index, intensity) in row.iter().copied().enumerate() {
                if intensity == 0 {
                    continue;
                }

                let pixel_x = x_cursor.saturating_add(column_index.saturating_mul(SCALE));
                let pixel_y = start_y.saturating_add(row_index.saturating_mul(SCALE));
                for dy in 0..SCALE {
                    for dx in 0..SCALE {
                        framebuffer.write_pixel(
                            pixel_x.saturating_add(dx),
                            pixel_y.saturating_add(dy),
                            FOREGROUND,
                        );
                    }
                }
            }
        }
        x_cursor = x_cursor.saturating_add(font::FONT_WIDTH.saturating_mul(SCALE));
    }
    framebuffer.end_batch();
}

pub fn set_rendering_enabled(enabled: bool) {
    RENDERING_ENABLED.store(enabled, Ordering::Relaxed);
}

#[must_use]
pub fn rendering_enabled() -> bool {
    RENDERING_ENABLED.load(Ordering::Relaxed)
}

#[must_use]
pub fn current_screen_owner() -> ScreenOwner {
    ScreenOwner::from_raw(SCREEN_OWNER.load(Ordering::Relaxed))
}

#[must_use]
pub fn input_cursor_snapshot() -> InputCursorSnapshot {
    let enabled = INPUT_CURSOR_ENABLED.load(Ordering::Relaxed);
    let mut snapshot = InputCursorSnapshot {
        row: 0,
        col: 0,
        enabled,
        visible: false,
    };
    let mut guard = CONSOLE.lock();
    if let Some(console) = guard.as_mut() {
        snapshot.row = console.cursor_row;
        snapshot.col = console.cursor_col;
        snapshot.visible = console.input_cursor_visible;
    }
    snapshot
}

pub fn reset_shell_clobber_debug() {
    SHELL_CLOBBER_COUNT.store(0, Ordering::Relaxed);
    SHELL_CLOBBER_OWNER.store(ScreenOwner::Unknown as u8, Ordering::Relaxed);
    SHELL_CLOBBER_KIND.store(ShellClobberKind::None as u8, Ordering::Relaxed);
}

#[must_use]
pub fn shell_clobber_snapshot() -> ShellClobberSnapshot {
    ShellClobberSnapshot {
        count: SHELL_CLOBBER_COUNT.load(Ordering::Relaxed),
        owner: ScreenOwner::from_raw(SHELL_CLOBBER_OWNER.load(Ordering::Relaxed)),
        kind: ShellClobberKind::from_raw(SHELL_CLOBBER_KIND.load(Ordering::Relaxed)),
    }
}

#[must_use]
pub fn screen_owner_is(owner: ScreenOwner) -> bool {
    current_screen_owner() == owner
}

pub fn claim_screen_owner(owner: ScreenOwner, reason: &str) {
    let previous = ScreenOwner::from_raw(SCREEN_OWNER.swap(owner as u8, Ordering::Relaxed));
    if previous == ScreenOwner::Shell && owner != ScreenOwner::Shell {
        record_shell_clobber(ShellClobberKind::OwnerClaim, owner, reason);
    }
    if previous != owner {
        trace_screen_event(format_args!(
            "[SCREEN] owner {} -> {} reason={}",
            previous.label(),
            owner.label(),
            reason
        ));
    }
}

#[must_use]
pub fn clear_screen_for(owner: ScreenOwner, reason: &str) -> bool {
    let active = current_screen_owner();
    if active == ScreenOwner::Shell && owner != ScreenOwner::Shell {
        record_shell_clobber(ShellClobberKind::ClearAttempt, owner, reason);
    }
    if !owner_can_clear(owner, active) {
        trace_screen_event(format_args!(
            "[SCREEN] clear blocked requester={} active={} reason={}",
            owner.label(),
            active.label(),
            reason
        ));
        return false;
    }
    let _ = with_console(FramebufferConsole::clear_screen);
    true
}

fn owner_can_clear(requester: ScreenOwner, active: ScreenOwner) -> bool {
    if requester == ScreenOwner::Panic {
        return true;
    }
    if requester == active {
        return true;
    }
    match requester {
        ScreenOwner::Unknown => !matches!(active, ScreenOwner::Shell | ScreenOwner::Gui),
        _ => false,
    }
}

pub fn begin_capture() {
    CAPTURE_BUFFER.lock().clear();
    CAPTURE_ACTIVE.store(true, Ordering::Relaxed);
}

#[must_use]
pub fn end_capture() -> String {
    CAPTURE_ACTIVE.store(false, Ordering::Relaxed);
    core::mem::take(&mut *CAPTURE_BUFFER.lock())
}

/// Clear the screen using the active background color.
pub fn clear_screen() {
    if rendering_enabled() {
        let _ = clear_screen_for(ScreenOwner::Unknown, "legacy-clear");
    } else {
        let _ = with_console(|console| {
            console.cursor_col = 0;
            console.cursor_row = 0;
        });
    }
}

/// Remove one character from the visible line buffer.
pub fn backspace() {
    let _ = with_console(FramebufferConsole::backspace);
}

pub fn set_input_cursor_enabled(enabled: bool) {
    INPUT_CURSOR_ENABLED.store(enabled, Ordering::Relaxed);
    let mut guard = CONSOLE.lock();
    if let Some(console) = guard.as_mut() {
        trace_cursor_event(format_args!(
            "[CURSOR] enable={} owner={} row={} col={}",
            if enabled { "on" } else { "off" },
            current_screen_owner().label(),
            console.cursor_row,
            console.cursor_col
        ));
        if enabled {
            console.show_input_cursor();
        } else {
            console.hide_input_cursor();
        }
    }
}

pub fn force_input_cursor_visible() {
    if !INPUT_CURSOR_ENABLED.load(Ordering::Relaxed) || !rendering_enabled() {
        return;
    }

    let mut guard = CONSOLE.lock();
    if let Some(console) = guard.as_mut() {
        trace_cursor_event(format_args!(
            "[CURSOR] force-show owner={} row={} col={}",
            current_screen_owner().label(),
            console.cursor_row,
            console.cursor_col
        ));
        console.show_input_cursor();
    }
}

pub fn refresh_input_cursor() {
    if !rendering_enabled() {
        return;
    }

    let mut guard = CONSOLE.lock();
    if let Some(console) = guard.as_mut() {
        if !INPUT_CURSOR_ENABLED.load(Ordering::Relaxed) {
            trace_cursor_event(format_args!(
                "[CURSOR] refresh-hide owner={} reason=disabled row={} col={}",
                current_screen_owner().label(),
                console.cursor_row,
                console.cursor_col
            ));
            console.hide_input_cursor();
            return;
        }

        if !INPUT_CURSOR_BLINK_ENABLED {
            console.show_input_cursor();
            return;
        }

        let phase =
            (crate::arch::x86_64::interrupts::tick_count() / INPUT_CURSOR_BLINK_TICKS) % 2 == 0;
        if phase {
            trace_cursor_event(format_args!(
                "[CURSOR] blink-show owner={} row={} col={}",
                current_screen_owner().label(),
                console.cursor_row,
                console.cursor_col
            ));
            console.show_input_cursor();
        } else {
            trace_cursor_event(format_args!(
                "[CURSOR] blink-hide owner={} row={} col={}",
                current_screen_owner().label(),
                console.cursor_row,
                console.cursor_col
            ));
            console.hide_input_cursor();
        }
    }
}

/// Print formatted text to the framebuffer console.
pub fn _print(args: fmt::Arguments<'_>) {
    if CAPTURE_ACTIVE.load(Ordering::Relaxed) {
        let rendered = alloc::format!("{args}");
        capture_text(&rendered);
        if !rendering_enabled() {
            return;
        }
        let mut guard = CONSOLE.lock();
        if let Some(console) = guard.as_mut() {
            let _ = console.write_str(&rendered);
        }
        return;
    }

    if !rendering_enabled() {
        return;
    }
    let mut guard = CONSOLE.lock();
    if let Some(console) = guard.as_mut() {
        let _ = console.write_fmt(args);
    }
}

/// Print formatted text using a temporary foreground color.
pub fn _print_colored(color: u32, args: fmt::Arguments<'_>) {
    if CAPTURE_ACTIVE.load(Ordering::Relaxed) {
        let rendered = alloc::format!("{args}");
        capture_text(&rendered);
        if !rendering_enabled() {
            return;
        }
        let mut guard = CONSOLE.lock();
        if let Some(console) = guard.as_mut() {
            let previous = console.fg_color;
            console.set_color(color);
            let _ = console.write_str(&rendered);
            console.set_color(previous);
        }
        return;
    }

    if !rendering_enabled() {
        return;
    }
    let mut guard = CONSOLE.lock();
    if let Some(console) = guard.as_mut() {
        let previous = console.fg_color;
        console.set_color(color);
        let _ = console.write_fmt(args);
        console.set_color(previous);
    }
}

fn capture_text(text: &str) {
    if CAPTURE_ACTIVE.load(Ordering::Relaxed) {
        let mut buffer = CAPTURE_BUFFER.lock();
        // Cap capture at 64 KiB to prevent heap exhaustion from long-running captures
        if buffer.len() + text.len() <= 65_536 {
            buffer.push_str(text);
        }
    }
}

impl ScreenOwner {
    fn from_raw(raw: u8) -> Self {
        match raw {
            x if x == Self::Boot as u8 => Self::Boot,
            x if x == Self::Auth as u8 => Self::Auth,
            x if x == Self::Shell as u8 => Self::Shell,
            x if x == Self::Gui as u8 => Self::Gui,
            x if x == Self::Panic as u8 => Self::Panic,
            _ => Self::Unknown,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Boot => "boot",
            Self::Auth => "auth",
            Self::Shell => "shell",
            Self::Gui => "gui",
            Self::Panic => "panic",
        }
    }
}

impl ShellClobberKind {
    fn from_raw(raw: u8) -> Self {
        match raw {
            x if x == Self::OwnerClaim as u8 => Self::OwnerClaim,
            x if x == Self::ClearAttempt as u8 => Self::ClearAttempt,
            _ => Self::None,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "no",
            Self::OwnerClaim => "owner-claim",
            Self::ClearAttempt => "clear-attempt",
        }
    }
}

fn record_shell_clobber(kind: ShellClobberKind, owner: ScreenOwner, reason: &str) {
    SHELL_CLOBBER_COUNT.fetch_add(1, Ordering::Relaxed);
    SHELL_CLOBBER_OWNER.store(owner as u8, Ordering::Relaxed);
    SHELL_CLOBBER_KIND.store(kind as u8, Ordering::Relaxed);
    trace_screen_event(format_args!(
        "[SCREEN] shell-clobber kind={} owner={} reason={}",
        kind.label(),
        owner.label(),
        reason
    ));
}

fn trace_screen_event(args: fmt::Arguments<'_>) {
    if !SCREEN_TRACE_ENABLED {
        return;
    }
    let remaining = SCREEN_TRACE_BUDGET.load(Ordering::Relaxed);
    if remaining == 0 {
        return;
    }
    if SCREEN_TRACE_BUDGET
        .compare_exchange(remaining, remaining - 1, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
    {
        crate::serial_println!("{}", args);
    }
}

fn trace_cursor_event(args: fmt::Arguments<'_>) {
    if !CURSOR_TRACE_ENABLED {
        return;
    }
    let remaining = CURSOR_TRACE_BUDGET.load(Ordering::Relaxed);
    if remaining == 0 {
        return;
    }
    if CURSOR_TRACE_BUDGET
        .compare_exchange(remaining, remaining - 1, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
    {
        crate::serial_println!("{}", args);
    }
}

fn blend(background: u32, foreground: u32, alpha: u8) -> u32 {
    let background_bytes = background.to_be_bytes();
    let foreground_bytes = foreground.to_be_bytes();
    let alpha = u16::from(alpha);
    let inverse_alpha = u16::from(u8::MAX) - alpha;

    let red = ((u16::from(background_bytes[1]) * inverse_alpha)
        + (u16::from(foreground_bytes[1]) * alpha))
        / u16::from(u8::MAX);
    let green = ((u16::from(background_bytes[2]) * inverse_alpha)
        + (u16::from(foreground_bytes[2]) * alpha))
        / u16::from(u8::MAX);
    let blue = ((u16::from(background_bytes[3]) * inverse_alpha)
        + (u16::from(foreground_bytes[3]) * alpha))
        / u16::from(u8::MAX);

    u32::from_be_bytes([0, red as u8, green as u8, blue as u8])
}

#[macro_export]
macro_rules! kprint {
    ($($arg:tt)*) => {
        $crate::display::console::_print(format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! kprintln {
    () => {
        $crate::kprint!("\n")
    };
    ($($arg:tt)*) => {
        $crate::display::console::_print(format_args!("{}\n", format_args!($($arg)*)))
    };
}

#[macro_export]
macro_rules! kprint_colored {
    ($color:expr, $($arg:tt)*) => {
        $crate::display::console::_print_colored($color, format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_print {
    ($($arg:tt)*) => {{
        $crate::kprint!($($arg)*);
        $crate::serial_print!($($arg)*);
    }};
}

#[macro_export]
macro_rules! log_println {
    () => {
        $crate::log_print!("\n")
    };
    ($($arg:tt)*) => {{
        $crate::kprintln!($($arg)*);
        $crate::serial_println!($($arg)*);
    }};
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        $crate::kprint!($($arg)*)
    };
}

#[macro_export]
macro_rules! println {
    () => {
        $crate::kprintln!()
    };
    ($($arg:tt)*) => {
        $crate::kprintln!($($arg)*)
    };
}
