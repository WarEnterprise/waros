use alloc::vec::Vec;

use core::arch::x86_64::__cpuid;
use core::str;

use crate::arch::x86_64::interrupts;
use crate::arch::x86_64::pit::PIT_FREQUENCY_HZ;
use crate::arch::x86_64::port;
use crate::display::branding;
use crate::display::console::{self, Colors};
use crate::memory;
use crate::shell::history;
use crate::{kprint, kprint_colored, kprintln, serial_println, KERNEL_VERSION};

/// Execute a built-in shell command.
pub fn execute_command(command_line: &str) {
    let command_line = command_line.trim();
    let parts: Vec<&str> = command_line.split_whitespace().collect();
    let Some(command) = parts.first().copied() else {
        return;
    };

    match command {
        "help" => cmd_help(),
        "clear" => {
            console::clear_screen();
        }
        "info" => cmd_info(),
        "version" => cmd_version(),
        "cpu" => cmd_cpu(),
        "mem" => cmd_mem(),
        "time" => cmd_time(),
        "uptime" => cmd_uptime(),
        "echo" => cmd_echo(command_line),
        "color" => cmd_color(),
        "hex" => cmd_hex(&parts[1..]),
        "history" => cmd_history(),
        "banner" => cmd_banner(),
        "quantum" => cmd_quantum(),
        "crypto" => cmd_crypto(),
        "panic" => cmd_panic(),
        "reboot" => cmd_reboot(),
        "halt" => cmd_halt(),
        "waros" => cmd_waros(),
        unknown => cmd_unknown(unknown),
    }
}

fn cmd_help() {
    kprint_colored!(Colors::CYAN, "WarOS Shell");
    kprintln!(" v{} - Available commands:\n", KERNEL_VERSION);

    kprint_colored!(Colors::PURPLE, "  System\n");
    kprintln!("    info        System information");
    kprintln!("    version     Detailed version information");
    kprintln!("    cpu         CPU vendor and feature flags");
    kprintln!("    mem         Physical memory statistics");
    kprintln!("    time        Uptime as HH:MM:SS");
    kprintln!("    uptime      Uptime in ticks and seconds");

    kprintln!();
    kprint_colored!(Colors::PURPLE, "  Quantum & Crypto\n");
    kprintln!("    quantum     Quantum subsystem status");
    kprintln!("    crypto      Post-quantum crypto status");

    kprintln!();
    kprint_colored!(Colors::PURPLE, "  Tools\n");
    kprintln!("    echo        Echo text");
    kprintln!("    hex         Hex dump memory: hex <addr> [len]");
    kprintln!("    color       Display color palette");
    kprintln!("    history     Show last 10 commands");

    kprintln!();
    kprint_colored!(Colors::PURPLE, "  Display\n");
    kprintln!("    clear       Clear screen");
    kprintln!("    banner      Show boot banner");

    kprintln!();
    kprint_colored!(Colors::PURPLE, "  Control\n");
    kprintln!("    halt        Halt CPU");
    kprintln!("    reboot      Restart system");
    kprintln!("    panic       Trigger test panic");
    kprintln!("    waros       About WarOS");

    kprintln!();
    kprint_colored!(Colors::DIM, "  Type a command and press Enter.\n");
}

fn cmd_info() {
    kprintln!("WarOS v{} - Quantum-Classical Hybrid OS", KERNEL_VERSION);
    kprintln!("Architecture: x86_64");
    kprintln!("Kernel: WarKernel (microkernel bootstrap)");
    kprintln!("Boot mode: BIOS via bootloader");
    kprintln!("Timer: {} Hz PIT", PIT_FREQUENCY_HZ);
    kprintln!("War Enterprise (c) 2026");
}

fn cmd_version() {
    kprintln!("WarOS kernel version information:");
    kprintln!("  Kernel:    v{}", KERNEL_VERSION);
    kprintln!("  Platform:  x86_64");
    kprintln!("  Toolchain: Rust nightly");
    kprintln!("  Boot:      bootloader BIOS/UEFI images");
    kprintln!("  Quantum:   SDK + simulator integrated in repository");
    kprintln!("  Crypto:    PQC suite available in workspace");
}

fn cmd_cpu() {
    let vendor_leaf = __cpuid(0);
    let feature_leaf = __cpuid(1);

    let vendor_bytes = vendor_string_bytes(vendor_leaf.ebx, vendor_leaf.edx, vendor_leaf.ecx);
    let vendor = match str::from_utf8(&vendor_bytes) {
        Ok(vendor) => vendor,
        Err(_) => "Unknown",
    };

    let base_family = (feature_leaf.eax >> 8) & 0x0F;
    let ext_family = (feature_leaf.eax >> 20) & 0xFF;
    let family = if base_family == 0x0F {
        base_family + ext_family
    } else {
        base_family
    };

    let base_model = (feature_leaf.eax >> 4) & 0x0F;
    let ext_model = (feature_leaf.eax >> 16) & 0x0F;
    let model = if base_family == 0x06 || base_family == 0x0F {
        base_model | (ext_model << 4)
    } else {
        base_model
    };

    kprintln!("CPU Information:");
    kprintln!("  Vendor:    {}", vendor);
    kprintln!("  Family:    {}", family);
    kprintln!("  Model:     {}", model);
    kprintln!("  Max CPUID: {}", vendor_leaf.eax);
    kprint!("  Features:  ");

    let mut any = false;
    emit_feature(feature_leaf.edx & (1 << 25) != 0, "SSE", &mut any);
    emit_feature(feature_leaf.edx & (1 << 26) != 0, "SSE2", &mut any);
    emit_feature(feature_leaf.ecx & (1 << 0) != 0, "SSE3", &mut any);
    emit_feature(feature_leaf.ecx & (1 << 19) != 0, "SSE4.1", &mut any);
    emit_feature(feature_leaf.ecx & (1 << 20) != 0, "SSE4.2", &mut any);
    emit_feature(feature_leaf.ecx & (1 << 25) != 0, "AES-NI", &mut any);
    emit_feature(feature_leaf.ecx & (1 << 28) != 0, "AVX", &mut any);

    if !any {
        kprintln!("none");
    } else {
        kprintln!();
    }
}

fn cmd_mem() {
    let stats = memory::stats();
    let used_frames = stats.total_frames.saturating_sub(stats.free_frames);
    kprintln!("Physical memory:");
    kprintln!(
        "  Total: {} MiB ({} frames)",
        (stats.total_frames * 4) / 1024,
        stats.total_frames
    );
    kprintln!(
        "  Used:  {} MiB ({} frames)",
        (used_frames * 4) / 1024,
        used_frames
    );
    kprintln!(
        "  Free:  {} MiB ({} frames)",
        (stats.free_frames * 4) / 1024,
        stats.free_frames
    );
}

fn cmd_time() {
    let ticks = interrupts::tick_count();
    let total_seconds = ticks / u64::from(PIT_FREQUENCY_HZ);
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    kprintln!(
        "Uptime: {:02}:{:02}:{:02} ({} ticks)",
        hours,
        minutes,
        seconds,
        ticks
    );
}

fn cmd_uptime() {
    let ticks = interrupts::tick_count();
    let seconds = ticks / u64::from(PIT_FREQUENCY_HZ);
    kprintln!("Uptime: {}s ({} ticks)", seconds, ticks);
}

fn cmd_echo(command_line: &str) {
    let text = command_line
        .split_once(char::is_whitespace)
        .map_or("", |(_, text)| text);
    kprintln!("{text}");
}

fn cmd_color() {
    kprintln!("Color palette test:");
    kprint_colored!(Colors::FG, "  Default text\n");
    kprint_colored!(Colors::GREEN, "  Green (success/OK)\n");
    kprint_colored!(Colors::RED, "  Red (errors)\n");
    kprint_colored!(Colors::BLUE, "  Blue (info)\n");
    kprint_colored!(Colors::YELLOW, "  Yellow (warnings)\n");
    kprint_colored!(Colors::PURPLE, "  Purple (branding)\n");
    kprint_colored!(Colors::CYAN, "  Cyan (highlights)\n");
    kprint_colored!(Colors::DIM, "  Dim (secondary)\n");
}

fn cmd_hex(args: &[&str]) {
    let Some(address) = args.first().and_then(|value| parse_u64(value)) else {
        kprintln!("Usage: hex <address> [length]");
        kprintln!("  Example: hex 0x1000 64");
        return;
    };

    let length = args
        .get(1)
        .and_then(|value| parse_usize(value))
        .unwrap_or(64)
        .min(256);

    kprintln!("Memory at 0x{:016X} ({} bytes):", address, length);

    for row in (0..length).step_by(16) {
        kprint!("  {:016X}  ", address + row as u64);

        for column in 0..16 {
            if row + column < length {
                let byte = read_memory_byte(address + (row + column) as u64);
                kprint!("{:02X} ", byte);
            } else {
                kprint!("   ");
            }

            if column == 7 {
                kprint!(" ");
            }
        }

        kprint!(" |");
        for column in 0..16 {
            if row + column < length {
                let byte = read_memory_byte(address + (row + column) as u64);
                let printable = if byte.is_ascii_graphic() || byte == b' ' {
                    byte as char
                } else {
                    '.'
                };
                kprint!("{}", printable);
            }
        }
        kprintln!("|");
    }
}

fn cmd_history() {
    let entries = history::snapshot();
    if entries.is_empty() {
        kprintln!("No commands in history yet.");
        return;
    }

    kprintln!("Recent commands:");
    for (index, entry) in entries.iter().enumerate() {
        kprintln!("  {:>2}: {}", index + 1, entry);
    }
}

fn cmd_banner() {
    console::clear_screen();
    branding::show_banner();
}

fn cmd_quantum() {
    kprint_colored!(Colors::PURPLE, "Quantum Subsystem Status\n");
    branding::show_separator();
    kprintln!("  Backend:        Classical Simulation (StateVector)");
    kprintln!("  Max qubits:     25 (limited by available RAM)");
    kprintln!("  QPU hardware:   Not detected");
    kprintln!("  QHAL drivers:   None loaded");
    kprintln!("  QEC engine:     Not initialized");
    kprintln!("  Quantum net:    Not available");
    kprintln!();
    kprint_colored!(Colors::YELLOW, "  Note: ");
    kprintln!("Running in classical simulation mode.");
    kprintln!("  Connect quantum hardware via QHAL for native QPU access.");
    kprintln!("  See: github.com/WarEnterprise/waros/blob/main/BLUEPRINT.md");
}

fn cmd_crypto() {
    kprint_colored!(Colors::CYAN, "Post-Quantum Cryptography Status\n");
    branding::show_separator();
    kprintln!("  Key Encapsulation:    ML-KEM-768 (FIPS 203)        [available]");
    kprintln!("  Digital Signatures:   ML-DSA-65 (FIPS 204)         [available]");
    kprintln!("  Hash-based Sigs:      SLH-DSA-SHA2-128s (FIPS 205) [available]");
    kprintln!("  Hash Functions:       SHA-3 / SHAKE                [available]");
    kprintln!("  QRNG:                 Simulated (CSPRNG fallback)  [active]");
    kprintln!("  QKD:                  Not available (no quantum network)");
    kprintln!();
    kprintln!("  All algorithms are quantum-resistant against quantum attacks.");
}

fn cmd_panic() {
    kprintln!("Triggering test kernel panic...");
    panic!("User-triggered test panic via 'panic' command");
}

fn cmd_reboot() {
    kprintln!("Rebooting system.");
    serial_println!("Rebooting system.");
    port::outb(0x64, 0xFE);
    crate::arch::x86_64::hlt_loop();
}

fn cmd_halt() {
    kprintln!("Halting CPU.");
    serial_println!("Halting CPU.");
    crate::arch::x86_64::hlt_loop();
}

fn cmd_waros() {
    kprintln!("WarOS - The first OS for the post-quantum era.");
    kprintln!("Quantum-native. Classically complete. Open source.");
    kprintln!("github.com/WarEnterprise/waros");
}

fn cmd_unknown(command: &str) {
    kprint_colored!(Colors::RED, "[ERR]");
    kprintln!(" Unknown command: '{}'. Type 'help' for commands.", command);
}

fn vendor_string_bytes(ebx: u32, edx: u32, ecx: u32) -> [u8; 12] {
    let ebx = ebx.to_le_bytes();
    let edx = edx.to_le_bytes();
    let ecx = ecx.to_le_bytes();

    [
        ebx[0], ebx[1], ebx[2], ebx[3], edx[0], edx[1], edx[2], edx[3], ecx[0], ecx[1], ecx[2],
        ecx[3],
    ]
}

fn emit_feature(enabled: bool, name: &str, any: &mut bool) {
    if enabled {
        *any = true;
        kprint_colored!(Colors::CYAN, "{} ", name);
    }
}

fn parse_u64(value: &str) -> Option<u64> {
    if let Some(hex) = value.strip_prefix("0x").or_else(|| value.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16).ok()
    } else {
        value
            .parse::<u64>()
            .ok()
            .or_else(|| u64::from_str_radix(value, 16).ok())
    }
}

fn parse_usize(value: &str) -> Option<usize> {
    if let Some(hex) = value.strip_prefix("0x").or_else(|| value.strip_prefix("0X")) {
        usize::from_str_radix(hex, 16).ok()
    } else {
        value
            .parse::<usize>()
            .ok()
            .or_else(|| usize::from_str_radix(value, 16).ok())
    }
}

fn read_memory_byte(address: u64) -> u8 {
    unsafe {
        // SAFETY: This debugging helper intentionally reads raw virtual memory. Callers opt into
        // the risk that the address might fault; if it does, the kernel panic path is exercised.
        *(address as *const u8)
    }
}
