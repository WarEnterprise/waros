use alloc::string::ToString;

use crate::auth::session;
use crate::arch::x86_64::{interrupts, pit};
use crate::fs;
use crate::memory;
use crate::net;
use crate::quantum;
use crate::{boot_complete_ms, BUILD_DATE, KERNEL_VERSION};

use super::super::font;
use super::super::framebuffer::Surface;
use super::super::theme::Theme;

pub struct SystemInfoState;

impl SystemInfoState {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    pub fn render(&mut self, buffer: &mut [u32], width: usize, height: usize) {
        let mut surface = Surface::new(buffer, width, height);
        surface.clear(Theme::WINDOW_BG);
        let padding_x = Theme::WINDOW_PADDING;
        let padding_y = 10usize;
        let line_step = 20usize;

        let memory_stats = memory::stats();
        let uptime = interrupts::tick_count() / u64::from(pit::PIT_FREQUENCY_HZ);
        let hours = uptime / 3600;
        let minutes = (uptime % 3600) / 60;
        let seconds = uptime % 60;
        let network = net::network_config()
            .map(|config| config.cidr_string())
            .unwrap_or_else(|| "offline".into());
        let files = fs::file_count_for_user(session::current_uid());
        let quantum_state = quantum::active_register()
            .map(|(qubits, _)| alloc::format!("{qubits} qubits"))
            .unwrap_or_else(|| "idle".into());

        let lines = [
            alloc::format!("WarOS {}", KERNEL_VERSION),
            "War Enterprise desktop".to_string(),
            alloc::format!("Built: {BUILD_DATE}"),
            alloc::format!(
                "User: {}",
                session::current_user()
                    .map(|user| user.username)
                    .unwrap_or_else(|| "guest".into())
            ),
            alloc::format!("Uptime: {hours:02}:{minutes:02}:{seconds:02}"),
            alloc::format!("Boot: {} ms", boot_complete_ms()),
            alloc::format!("Memory: {} MiB free", (memory_stats.free_frames * 4) / 1024),
            alloc::format!("Files: {}", files),
            alloc::format!("Network: {}", network),
            alloc::format!("Quantum: {}", quantum_state),
            alloc::format!("Files: {}", files),
        ];

        let max_lines = height.saturating_sub(padding_y * 2) / line_step;
        for (index, line) in lines.iter().take(max_lines).enumerate() {
            let color = if index < 2 {
                Theme::TEXT_ACCENT
            } else {
                Theme::TEXT_PRIMARY
            };
            font::draw_text(&mut surface, padding_x, padding_y + index * line_step, line, color);
        }
    }
}
