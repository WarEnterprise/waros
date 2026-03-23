use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::quantum;

use super::super::font;
use super::super::framebuffer::{Rect, Surface};
use super::super::theme::Theme;
use super::super::widgets;

pub struct QuantumMonitorState;

impl QuantumMonitorState {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    pub fn render(&mut self, buffer: &mut [u32], width: usize, height: usize) {
        let mut surface = Surface::new(buffer, width, height);
        surface.clear(Theme::WINDOW_BG);

        font::draw_text(&mut surface, 8, 8, "Quantum Monitor", Theme::QUANTUM_GATE);
        if let Some(snapshot) = quantum::gui_snapshot() {
            font::draw_text(
                &mut surface,
                8,
                32,
                &alloc::format!("Qubits: {}", snapshot.num_qubits),
                Theme::TEXT_PRIMARY,
            );
            font::draw_text(
                &mut surface,
                8,
                48,
                &alloc::format!("State bytes: {}", snapshot.bytes_used),
                Theme::TEXT_SECONDARY,
            );

            let wires = snapshot.num_qubits.min(4).max(1);
            for qubit in 0..wires {
                let y = 86 + qubit * 26;
                font::draw_text(
                    &mut surface,
                    8,
                    y.saturating_sub(6),
                    &alloc::format!("q{}", qubit),
                    Theme::TEXT_SECONDARY,
                );
                surface.draw_hline(32, y, width.saturating_sub(48), Theme::QUANTUM_WIRE);
            }

            for (column, operation) in snapshot.operations.iter().take(6).enumerate() {
                draw_operation(&mut surface, operation, column, 32, 86, width);
            }

            if let Some(result_line) = snapshot
                .last_result_text
                .as_ref()
                .and_then(|text| text.lines().nth(1))
            {
                font::draw_text(
                    &mut surface,
                    8,
                    height.saturating_sub(56),
                    "Last result:",
                    Theme::TEXT_PRIMARY,
                );
                font::draw_text(
                    &mut surface,
                    8,
                    height.saturating_sub(40),
                    &truncate(result_line, width.saturating_sub(16)),
                    Theme::QUANTUM_RESULT,
                );
            }
        } else {
            font::draw_text(
                &mut surface,
                8,
                40,
                "No active quantum register.",
                Theme::TEXT_SECONDARY,
            );
            font::draw_text(
                &mut surface,
                8,
                60,
                "Use qalloc and qrun in the terminal window.",
                Theme::TEXT_SECONDARY,
            );
        }

        widgets::draw_button(
            &mut surface,
            Rect {
                x: 8,
                y: height.saturating_sub(28),
                width: 80,
                height: 20,
            },
            "Run",
            true,
        );
        widgets::draw_button(
            &mut surface,
            Rect {
                x: 96,
                y: height.saturating_sub(28),
                width: 80,
                height: 20,
            },
            "Reset",
            false,
        );
        widgets::draw_button(
            &mut surface,
            Rect {
                x: 184,
                y: height.saturating_sub(28),
                width: 96,
                height: 20,
            },
            "Measure",
            false,
        );
    }
}

fn draw_operation(
    surface: &mut Surface<'_>,
    operation: &str,
    column: usize,
    wire_x: usize,
    wire_y_start: usize,
    width: usize,
) {
    let x = wire_x + 40 + column * 52;
    if x + 40 >= width {
        return;
    }

    if let Some((control, target)) = parse_controlled(operation) {
        let y0 = wire_y_start + control * 26;
        let y1 = wire_y_start + target * 26;
        surface.draw_line(x as i32, y0 as i32, x as i32, y1 as i32, Theme::QUANTUM_WIRE);
        surface.fill_rect(x.saturating_sub(2), y0.saturating_sub(2), 5, 5, Theme::QUANTUM_GATE);
        surface.draw_rect(x.saturating_sub(6), y1.saturating_sub(6), 12, 12, Theme::QUANTUM_GATE);
        font::draw_text(
            surface,
            x.saturating_sub(8),
            y0.saturating_sub(18),
            "CX",
            Theme::QUANTUM_GATE,
        );
        return;
    }

    if let Some((gate, qubit)) = parse_single(operation) {
        let y = wire_y_start + qubit * 26;
        surface.fill_rect(
            x.saturating_sub(14),
            y.saturating_sub(10),
            28,
            18,
            Theme::QUANTUM_GATE,
        );
        surface.draw_rect(
            x.saturating_sub(14),
            y.saturating_sub(10),
            28,
            18,
            Theme::WINDOW_BG,
        );
        font::draw_text(
            surface,
            x.saturating_sub(10),
            y.saturating_sub(7),
            &gate,
            Theme::WINDOW_BG,
        );
    }
}

fn parse_single(operation: &str) -> Option<(String, usize)> {
    let gate = operation.split_whitespace().next()?.trim().trim_end_matches(';');
    if gate.eq_ignore_ascii_case("cx") || gate.eq_ignore_ascii_case("cnot") {
        return None;
    }
    let start = operation.find("q[")? + 2;
    let end = operation[start..].find(']')? + start;
    let qubit = operation[start..end].parse().ok()?;
    Some((gate.to_ascii_uppercase(), qubit))
}

fn parse_controlled(operation: &str) -> Option<(usize, usize)> {
    let lower = operation.to_ascii_lowercase();
    if !lower.starts_with("cx ") && !lower.starts_with("cnot ") {
        return None;
    }
    let mut indices = Vec::new();
    let mut remaining = operation;
    while let Some(start) = remaining.find("q[") {
        let start_index = start + 2;
        let rest = &remaining[start_index..];
        let end_index = rest.find(']')?;
        indices.push(rest[..end_index].parse().ok()?);
        remaining = &rest[end_index + 1..];
    }
    if indices.len() >= 2 {
        Some((indices[0], indices[1]))
    } else {
        None
    }
}

fn truncate(text: &str, width_px: usize) -> String {
    let max_chars = width_px / crate::display::font::FONT_WIDTH;
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    text.chars().take(max_chars.saturating_sub(1)).collect()
}
