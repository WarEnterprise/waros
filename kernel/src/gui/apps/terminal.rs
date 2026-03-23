use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::auth::session;
use crate::arch::x86_64::interrupts;
use crate::display::console;
use crate::shell::commands::execute_command;

use super::super::font;
use super::super::framebuffer::Surface;
use super::super::theme::Theme;

const MAX_LINES: usize = 256;

pub struct TerminalState {
    lines: Vec<String>,
    input: String,
}

impl TerminalState {
    #[must_use]
    pub fn new(_window_width: usize, _window_height: usize) -> Self {
        let mut lines = Vec::new();
        lines.push("WarOS GUI Terminal".to_string());
        lines.push("Press ESC to return to the text shell.".to_string());
        Self {
            lines,
            input: String::new(),
        }
    }

    #[must_use]
    pub fn handle_key(&mut self, key: u8) -> bool {
        match key {
            b'\n' => {
                let command = self.input.trim().to_string();
                let prompt = gui_prompt();
                self.push_line(format!("{prompt}{}", self.input));
                if command == "clear" {
                    self.lines.clear();
                } else if !command.is_empty() {
                    console::begin_capture();
                    execute_command(&command);
                    let output = console::end_capture();
                    self.consume_output(&output);
                }
                self.input.clear();
                true
            }
            0x08 => {
                self.input.pop();
                true
            }
            byte if byte.is_ascii_graphic() || byte == b' ' => {
                self.input.push(char::from(byte));
                true
            }
            _ => false,
        }
    }

    pub fn render(&mut self, buffer: &mut [u32], width: usize, height: usize) {
        let mut surface = Surface::new(buffer, width, height);
        surface.clear(Theme::TERMINAL_BG);

        let line_height = font::line_height(1);
        let visible_lines = height.saturating_sub(line_height + 12) / line_height;
        let start = self.lines.len().saturating_sub(visible_lines);
        for (row, line) in self.lines.iter().skip(start).enumerate() {
            let y = 6 + row * line_height;
            font::draw_text(
                &mut surface,
                8,
                y,
                &truncate_for_width(line, width.saturating_sub(16)),
                Theme::TERMINAL_TEXT,
            );
        }

        let prompt = gui_prompt();
        let input_y = height.saturating_sub(line_height + 6);
        font::draw_text(&mut surface, 8, input_y, &prompt, Theme::TERMINAL_PROMPT);
        let prompt_width = font::text_width(&prompt, 1);
        font::draw_text(
            &mut surface,
            8 + prompt_width,
            input_y,
            &self.input,
            Theme::TERMINAL_TEXT,
        );

        let blink_on = (interrupts::tick_count() / 25).is_multiple_of(2);
        if blink_on {
            let cursor_x = 8 + prompt_width + font::text_width(&self.input, 1);
            surface.fill_rect(
                cursor_x,
                input_y + 2,
                8,
                line_height.saturating_sub(4),
                Theme::TERMINAL_CURSOR,
            );
        }
    }

    fn consume_output(&mut self, output: &str) {
        let normalized = output.replace('\r', "");
        for line in normalized.split('\n') {
            if !line.is_empty() {
                self.push_line(line.to_string());
            }
        }
    }

    fn push_line(&mut self, line: String) {
        if self.lines.len() >= MAX_LINES {
            self.lines.remove(0);
        }
        self.lines.push(line);
    }
}

fn gui_prompt() -> String {
    let username = session::current_user()
        .map(|user| user.username)
        .unwrap_or_else(|| "guest".to_string());
    format!("[WarOS {} {}]$ ", username, session::current_prompt_path())
}

fn truncate_for_width(text: &str, max_width_px: usize) -> String {
    let max_chars = max_width_px / crate::display::font::FONT_WIDTH;
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    text.chars().take(max_chars.saturating_sub(1)).collect()
}
