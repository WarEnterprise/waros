use alloc::string::ToString;
use alloc::vec::Vec;

use crate::arch::x86_64::{interrupts, pit};
use crate::net;

use super::apps::AppType;
use super::font;
use super::framebuffer::{Rect, Surface};
use super::mouse;
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
    surface.draw_hline(0, height.saturating_sub(1), width, Theme::TASKBAR_BORDER);
    font::draw_text(surface, 12, 9, "WarOS", Theme::TASKBAR_ACCENT);

    let mouse = mouse::current_snapshot();

    for launcher in launcher_layout() {
        let active = active_apps.contains(&launcher.app);
        let hovered = mouse.x >= launcher.x as i32
            && mouse.x < (launcher.x + launcher.width) as i32
            && mouse.y >= 4
            && mouse.y < (height - 4) as i32;
        widgets::draw_button(
            surface,
            Rect {
                x: launcher.x,
                y: 4,
                width: launcher.width,
                height: height - 8,
            },
            launcher.app.launcher_label(),
            widgets::button_style(active, hovered, false, active),
        );
    }

    let ip = net::network_config()
        .map(|config| config.ip.to_string())
        .unwrap_or_else(|| "No network".into());
    let uptime = interrupts::tick_count() / u64::from(pit::PIT_FREQUENCY_HZ);
    let clock = alloc::format!(
        "{:02}:{:02}",
        uptime / 3600,
        (uptime % 3600) / 60
    );

    let clock_width = font::text_width(&clock, 1);
    let separator_width = font::text_width(" · ", 1);
    let ip_width = font::text_width(&ip, 1);
    font::draw_text(
        surface,
        width.saturating_sub(clock_width + 12),
        9,
        &clock,
        Theme::TASKBAR_TEXT,
    );
    font::draw_text(
        surface,
        width.saturating_sub(clock_width + separator_width + 12),
        9,
        " · ",
        Theme::TEXT_MUTED,
    );
    font::draw_text(
        surface,
        width.saturating_sub(clock_width + separator_width + ip_width + 12),
        9,
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
    let mut x = 92usize;
    for app in LAUNCHERS {
        let width = widgets::button_width(app.launcher_label());
        layout.push(LauncherRect { app, x, width });
        x += width + 6;
    }
    layout
}
