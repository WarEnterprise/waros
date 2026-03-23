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

    pub fn render(&mut self, buffer: &mut [u32], width: usize, height: usize, focused: bool) {
        let mut surface = Surface::new(buffer, width, height);
        surface.clear(Theme::TERMINAL_BG);

        let line_height = 18usize;
        let left_padding = 8usize;
        let top_padding = 4usize;
        let scrollbar_width = 4usize;
        let content_width = width.saturating_sub(left_padding * 2 + scrollbar_width + 6);
        let visible_lines = height.saturating_sub(line_height + top_padding + 10) / line_height;
        let start = self.lines.len().saturating_sub(visible_lines);
        for (row, line) in self.lines.iter().skip(start).enumerate() {
            let y = top_padding + row * line_height;
            render_terminal_line(&mut surface, left_padding, y, line, content_width);
        }

        let prompt = gui_prompt();
        let input_y = height.saturating_sub(line_height + 6);
        font::draw_text(&mut surface, left_padding, input_y, &prompt, Theme::TERMINAL_PROMPT);
        let prompt_width = font::text_width(&prompt, 1);
        font::draw_text(
            &mut surface,
            left_padding + prompt_width,
            input_y,
            &self.input,
            Theme::TERMINAL_TEXT,
        );

        let blink_on = focused && (interrupts::tick_count() / 50).is_multiple_of(2);
        if blink_on {
            let cursor_x = left_padding + prompt_width + font::text_width(&self.input, 1);
            surface.fill_rect(
                cursor_x,
                input_y + 2,
                8,
                line_height.saturating_sub(4),
                Theme::TERMINAL_CURSOR,
            );
        }

        if self.lines.len() > visible_lines && visible_lines > 0 {
            let track_x = width.saturating_sub(scrollbar_width + 4);
            surface.fill_rounded_rect(
                track_x,
                top_padding,
                scrollbar_width,
                height.saturating_sub(line_height + 12),
                2,
                Theme::TERMINAL_SCROLL_TRACK,
            );
            let thumb_height = ((visible_lines * (height.saturating_sub(line_height + 12)))
                / self.lines.len())
                .max(18);
            let scrollable = self.lines.len().saturating_sub(visible_lines).max(1);
            let thumb_y = top_padding + ((start * height.saturating_sub(line_height + 12)) / scrollable);
            surface.fill_rounded_rect(
                track_x,
                thumb_y,
                scrollbar_width,
                thumb_height.min(height.saturating_sub(line_height + 12)),
                2,
                Theme::TERMINAL_SCROLL_THUMB,
            );
        }
    }

    pub fn clear(&mut self) {
        self.lines.clear();
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

fn render_terminal_line(surface: &mut Surface<'_>, x: usize, y: usize, line: &str, max_width_px: usize) {
    if let Some(split) = line.find("]$ ") {
        let prompt = &line[..split + 3];
        let command = &line[split + 3..];
        font::draw_text(surface, x, y, &truncate_for_width(prompt, max_width_px), Theme::TERMINAL_PROMPT);
        let prompt_width = font::text_width(prompt, 1).min(max_width_px);
        font::draw_text(
            surface,
            x + prompt_width,
            y,
            &truncate_for_width(command, max_width_px.saturating_sub(prompt_width)),
            Theme::TERMINAL_TEXT,
        );
        return;
    }

    let color = if is_error_line(line) {
        Theme::TERMINAL_ERROR
    } else {
        Theme::TERMINAL_OUTPUT
    };
    font::draw_text(surface, x, y, &truncate_for_width(line, max_width_px), color);
}

fn is_error_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.contains("[err]") || lower.contains("error") || lower.contains("failed")
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
