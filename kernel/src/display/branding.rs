use crate::display::console::{self, Colors};
use crate::{kprint_colored, kprintln, BUILD_TAG, KERNEL_VERSION};

const MAJOR_SEPARATOR: &str =
    "================================================================================";
const MINOR_SEPARATOR: &str =
    "--------------------------------------------------------------------------------";

/// Render the WarOS boot banner.
pub fn show_banner() {
    kprint_colored!(Colors::DIM, "{}\n", MAJOR_SEPARATOR);
    kprint_colored!(Colors::GREEN, "  WAR ENTERPRISE\n");
    kprint_colored!(Colors::CYAN, "  WarOS v{}\n", KERNEL_VERSION);
    kprint_colored!(Colors::DIM, "  Hybrid Quantum-Classical Operating System\n");
    kprint_colored!(Colors::DIM, "  Build: {}\n", BUILD_TAG);
    kprintln!();
    kprint_colored!(Colors::GREEN, "       _       __           ____  _____\n");
    kprint_colored!(Colors::GREEN, "      | |     / /___ ______/ __ \\/ ___/\n");
    kprint_colored!(Colors::GREEN, "      | | /| / / __ `/ ___/ / / /\\__ \\\n");
    kprint_colored!(Colors::GREEN, "      | |/ |/ / /_/ / /  / /_/ /___/ /\n");
    kprint_colored!(Colors::GREEN, "      |__/|__/\\__,_/_/   \\____//____/\n");
    kprintln!();
    kprint_colored!(Colors::CYAN, "  Boot pipeline starting. Diagnostics follow below.\n");
    kprint_colored!(Colors::DIM, "  Florianopolis, Brazil | warenterprise.com/waros\n");
    kprint_colored!(Colors::DIM, "{}\n", MINOR_SEPARATOR);
    kprintln!();
}

/// Render the standard dim separator between boot phases or command sections.
pub fn show_separator() {
    kprint_colored!(Colors::DIM, "{}\n", MINOR_SEPARATOR);
}

pub fn boot_complete_animation() {
    let Some((width, height)) =
        console::with_console(|console| (console.width_pixels(), console.height_pixels()))
    else {
        return;
    };

    let y = height.saturating_sub(4);
    let mut tick_stalled = false;
    for start in (0..width).step_by(24) {
        let end = (start + 24).min(width);
        let _ = console::with_console(|console| {
            for x in start..end {
                console.write_pixel(x, y, Colors::GREEN);
                console.write_pixel(x, y + 1, Colors::GREEN);
            }
        });
        if !wait_one_tick() {
            tick_stalled = true;
            break;
        }
    }

    if !tick_stalled {
        let _ = wait_one_tick();
    } else {
        crate::serial_println!(
            "[TRACE] boot animation: timer tick stalled; skipping remaining delay"
        );
    }
    let _ = console::with_console(|console| {
        for x in 0..width {
            console.write_pixel(x, y, Colors::BG);
            console.write_pixel(x, y + 1, Colors::BG);
        }
    });
}

fn wait_one_tick() -> bool {
    let start_tick = crate::arch::x86_64::interrupts::tick_count();
    crate::arch::x86_64::pit::wait_for_tick_advance(start_tick, 2)
}
