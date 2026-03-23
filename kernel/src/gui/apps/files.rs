use crate::fs;

use super::super::font;
use super::super::framebuffer::Surface;
use super::super::theme::Theme;

pub struct FileBrowserState;

impl FileBrowserState {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    pub fn render(&mut self, buffer: &mut [u32], width: usize, height: usize) {
        let mut surface = Surface::new(buffer, width, height);
        surface.clear(Theme::WINDOW_BG);

        match fs::list_current(None) {
            Ok((directory, entries)) => {
                font::draw_text(
                    &mut surface,
                    8,
                    8,
                    &alloc::format!("Directory: {}", fs::display_path(&directory)),
                    Theme::TEXT_ACCENT,
                );
                if entries.is_empty() {
                    font::draw_text(
                        &mut surface,
                        8,
                        32,
                        "No files in current home.",
                        Theme::TEXT_SECONDARY,
                    );
                    return;
                }

                for (row, entry) in entries
                    .iter()
                    .take((height / 18).saturating_sub(2))
                    .enumerate()
                {
                    let line = alloc::format!(
                        "{}  {}  {}",
                        entry.permissions.mode_string(),
                        crate::auth::username_for_uid(entry.owner_uid),
                        fs::basename(&entry.name)
                    );
                    font::draw_text(&mut surface, 8, 32 + row * 18, &line, Theme::TEXT_PRIMARY);
                }
            }
            Err(_) => {
                font::draw_text(
                    &mut surface,
                    8,
                    8,
                    "File browser unavailable.",
                    Theme::TEXT_SECONDARY,
                );
            }
        }
    }
}
