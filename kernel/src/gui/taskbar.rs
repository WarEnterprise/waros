use alloc::string::ToString;
use alloc::vec::Vec;

use crate::arch::x86_64::{interrupts, pit};
use crate::net;

use super::apps::AppType;
use super::font;
use super::framebuffer::{Rect, Surface};
use super::theme::Theme;
use super::widgets;

const LAUNCHERS: [AppType; 4] = [
    AppType::Terminal,
    AppType::Quantum,
    AppType::FileBrowser,
    AppType::SystemInfo,
];

pub fn render_taskbar(surface: &mut Surface<'_>, width: usize, active_apps: &[AppType]) {
    let height = Theme::TASKBAR_HEIGHT;
    surface.fill_rect(0, 0, width, height, Theme::TASKBAR_BG);
    surface.draw_hline(0, height.saturating_sub(1), width, Theme::WINDOW_BORDER);
    font::draw_text(surface, 8, 8, "WarOS", Theme::TASKBAR_ACCENT);

    for launcher in launcher_layout() {
        let active = active_apps.contains(&launcher.app);
        widgets::draw_button(
            surface,
            Rect {
                x: launcher.x,
                y: 4,
                width: launcher.width,
                height: height - 8,
            },
            launcher.app.launcher_label(),
            active,
        );
    }

    let ip = net::network_config()
        .map(|config| config.ip.to_string())
        .unwrap_or_else(|| "No network".into());
    let uptime = interrupts::tick_count() / u64::from(pit::PIT_FREQUENCY_HZ);
    let clock = alloc::format!(
        "{:02}:{:02}:{:02}",
        uptime / 3600,
        (uptime % 3600) / 60,
        uptime % 60
    );

    let clock_width = font::text_width(&clock, 1);
    let ip_width = font::text_width(&ip, 1);
    font::draw_text(
        surface,
        width.saturating_sub(clock_width + 8),
        8,
        &clock,
        Theme::TASKBAR_TEXT,
    );
    font::draw_text(
        surface,
        width.saturating_sub(clock_width + ip_width + 24),
        8,
        &ip,
        Theme::TEXT_SECONDARY,
    );
}

#[must_use]
pub fn launcher_hit_test(x: i32, y: i32) -> Option<AppType> {
    if y < 0 || y as usize >= Theme::TASKBAR_HEIGHT {
        return None;
    }

    for launcher in launcher_layout() {
        if x >= launcher.x as i32
            && x < (launcher.x + launcher.width) as i32
            && y >= 4
            && y < (Theme::TASKBAR_HEIGHT - 4) as i32
        {
            return Some(launcher.app);
        }
    }

    None
}

#[derive(Clone, Copy)]
struct LauncherRect {
    app: AppType,
    x: usize,
    width: usize,
}

fn launcher_layout() -> Vec<LauncherRect> {
    let mut layout = Vec::with_capacity(LAUNCHERS.len());
    let mut x = 80usize;
    for app in LAUNCHERS {
        let width = font::text_width(app.launcher_label(), 1) + 18;
        layout.push(LauncherRect { app, x, width });
        x += width + 8;
    }
    layout
}
