use crate::arch::x86_64::interrupts;
use crate::arch::x86_64::port;
use crate::display::console;
use crate::memory;
use crate::println;

/// Execute a built-in shell command.
pub fn execute_command(command_line: &str) {
    let command_line = command_line.trim();
    let mut parts = command_line.split_whitespace();
    let Some(command) = parts.next() else {
        return;
    };

    match command {
        "help" => {
            println!("WarOS Shell v0.1.0 - Available commands:");
            println!("  help      Show this message");
            println!("  clear     Clear the screen");
            println!("  info      System information");
            println!("  mem       Memory statistics");
            println!("  uptime    System uptime");
            println!("  echo      Echo text");
            println!("  reboot    Reboot the system");
            println!("  halt      Halt the CPU");
            println!("  waros     About WarOS");
        }
        "clear" => {
            console::clear_screen();
        }
        "info" => {
            println!("WarOS v0.1.0 - Quantum-Classical Hybrid OS");
            println!("Architecture: x86_64");
            println!("Kernel: WarKernel (microkernel bootstrap)");
            println!("Toolchain: Rust nightly");
            println!("War Enterprise (c) 2026");
        }
        "mem" => {
            let stats = memory::stats();
            let used_frames = stats.total_frames.saturating_sub(stats.free_frames);
            println!("Physical memory:");
            println!(
                "  Total: {} MiB ({} frames)",
                (stats.total_frames * 4) / 1024,
                stats.total_frames
            );
            println!(
                "  Used:  {} MiB ({} frames)",
                (used_frames * 4) / 1024,
                used_frames
            );
            println!(
                "  Free:  {} MiB ({} frames)",
                (stats.free_frames * 4) / 1024,
                stats.free_frames
            );
        }
        "uptime" => {
            let ticks = interrupts::tick_count();
            let seconds = ticks / 100;
            println!("Uptime: {}s ({} ticks)", seconds, ticks);
        }
        "echo" => {
            let text = command_line
                .split_once(char::is_whitespace)
                .map_or("", |(_, text)| text);
            println!("{text}");
        }
        "reboot" => {
            println!("Rebooting system.");
            port::outb(0x64, 0xFE);
            crate::arch::x86_64::hlt_loop();
        }
        "halt" => {
            println!("Halting CPU.");
            crate::arch::x86_64::hlt_loop();
        }
        "waros" => {
            println!("WarOS - The first OS for the post-quantum era.");
            println!("Quantum-native. Classically complete. Open source.");
            println!("github.com/WarEnterprise/waros");
        }
        unknown => {
            println!("Unknown command: '{}'. Type 'help' for commands.", unknown);
        }
    }
}
