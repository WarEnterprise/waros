use x86_64::instructions::hlt;

use crate::arch::x86_64::interrupts;
use crate::display::console::{self, Colors};
use crate::{kprint_colored, kprintln, KERNEL_VERSION};

const MAJOR_SEPARATOR: &str =
    "================================================================================";
const MINOR_SEPARATOR: &str =
    "--------------------------------------------------------------------------------";

/// Render the WarOS boot banner.
pub fn show_banner() {
    kprint_colored!(Colors::DIM, "{}\n", MAJOR_SEPARATOR);
    kprint_colored!(Colors::GREEN, "     _       __           ____  _____\n");
    kprint_colored!(Colors::GREEN, "    | |     / /___ ______/ __ \\/ ___/\n");
    kprint_colored!(Colors::GREEN, "    | | /| / / __ `/ ___/ / / /\\__ \\\n");
    kprint_colored!(Colors::GREEN, "    | |/ |/ / /_/ / /  / /_/ /___/ /\n");
    kprint_colored!(Colors::GREEN, "    |__/|__/\\__,_/_/   \\____//____/\n");
    kprintln!();
    kprint_colored!(
        Colors::CYAN,
        "    WarOS v{} | Quantum-Classical Hybrid Operating System\n",
        KERNEL_VERSION
    );
    kprint_colored!(
        Colors::DIM,
        "    War Enterprise | Florianopolis, Brazil | 2026\n"
    );
    kprint_colored!(Colors::CYAN, "    warenterprise.com/waros\n");
    kprint_colored!(Colors::DIM, "{}\n\n", MAJOR_SEPARATOR);
}

/// Render the standard dim separator between boot phases or command sections.
pub fn show_separator() {
    kprint_colored!(Colors::DIM, "{}\n", MINOR_SEPARATOR);
}

pub fn boot_complete_animation() {
    let Some((width, height)) = console::with_console(|console| {
        (console.width_pixels(), console.height_pixels())
    }) else {
        return;
    };

    let y = height.saturating_sub(4);
    for start in (0..width).step_by(24) {
        let end = (start + 24).min(width);
        let _ = console::with_console(|console| {
            for x in start..end {
                console.write_pixel(x, y, Colors::GREEN);
                console.write_pixel(x, y + 1, Colors::GREEN);
            }
        });
        wait_one_tick();
    }

    wait_one_tick();
    let _ = console::with_console(|console| {
        for x in 0..width {
            console.write_pixel(x, y, Colors::BG);
            console.write_pixel(x, y + 1, Colors::BG);
        }
    });
}

fn wait_one_tick() {
    let target = interrupts::tick_count().saturating_add(1);
    while interrupts::tick_count() < target {
        hlt();
    }
}
