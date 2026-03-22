use alloc::vec::Vec;

use core::arch::x86_64::__cpuid;
use core::str;

use crate::arch::x86_64::interrupts;
use crate::arch::x86_64::pit::PIT_FREQUENCY_HZ;
use crate::arch::x86_64::port;
use crate::display::branding;
use crate::display::console::{self, Colors};
use crate::fs;
use crate::memory;
use crate::memory::heap;
use crate::net;
use crate::quantum;
use crate::shell::history;
use crate::task;
use crate::{
    boot_complete_ms, kprint, kprint_colored, kprintln, serial_println, BUILD_DATE,
    KERNEL_VERSION,
};

/// Execute a built-in shell command.
pub fn execute_command(command_line: &str) {
    let command_line = command_line.trim();
    let parts: Vec<&str> = command_line.split_whitespace().collect();
    let Some(command) = parts.first().copied() else {
        return;
    };

    match command {
        "help" => cmd_help(parts.get(1).copied()),
        "clear" => {
            console::clear_screen();
        }
        "ls" => cmd_ls(),
        "cat" => cmd_cat(&parts[1..]),
        "write" => cmd_write(command_line),
        "rm" => cmd_rm(&parts[1..]),
        "touch" => cmd_touch(&parts[1..]),
        "stat" => cmd_stat(&parts[1..]),
        "df" => cmd_df(),
        "info" => cmd_info(),
        "version" => cmd_version(&parts[1..]),
        "cpu" => cmd_cpu(),
        "mem" => cmd_mem(),
        "time" => cmd_time(),
        "uptime" => cmd_uptime(),
        "date" => cmd_date(),
        "whoami" => cmd_whoami(),
        "uname" => cmd_uname(),
        "neofetch" => cmd_neofetch(),
        "lspci" => cmd_lspci(),
        "echo" => cmd_echo(command_line),
        "color" => cmd_color(),
        "hex" => cmd_hex(&parts[1..]),
        "history" => cmd_history(),
        "tasks" => cmd_tasks(),
        "spawn" => cmd_spawn(command_line),
        "kill" => cmd_kill(&parts[1..]),
        "banner" => cmd_banner(),
        "quantum" => cmd_quantum(),
        "crypto" => cmd_crypto(),
        "net" => cmd_net(command_line),
        "ping" => cmd_ping(),
        "qalloc" | "qfree" | "qreset" | "qrun" | "qstate" | "qprobs" | "qmeasure" | "qcircuit"
        | "qinfo" | "qsave" | "qexport" | "qresult" => {
            if let Err(error) = quantum::handle_quantum_command(command, &parts[1..]) {
                kprint_colored!(Colors::RED, "Quantum error: ");
                kprintln!("{}", error);
            }
        }
        "panic" => cmd_panic(),
        "reboot" => cmd_reboot(),
        "halt" => cmd_halt(),
        "waros" => cmd_waros(),
        unknown => cmd_unknown(unknown),
    }
}

fn cmd_help(topic: Option<&str>) {
    if matches!(topic, Some("quantum")) {
        quantum::show_help();
        return;
    }
    if matches!(topic, Some("fs")) {
        kprint_colored!(Colors::CYAN, "WarFS Commands\n");
        kprintln!("  ls             List files");
        kprintln!("  cat <file>     Show file contents");
        kprintln!("  write <f> <t>  Create or overwrite a text file");
        kprintln!("  rm <file>      Delete a file");
        kprintln!("  touch <file>   Create an empty file");
        kprintln!("  stat <file>    Show file metadata");
        kprintln!("  df             Filesystem usage");
        return;
    }

    kprint_colored!(Colors::CYAN, "WarOS Shell");
    kprintln!(" v{} - Available commands:\n", KERNEL_VERSION);

    kprint_colored!(Colors::PURPLE, "  Filesystem\n");
    kprintln!("    ls          List files in WarFS");
    kprintln!("    cat         Show a text file");
    kprintln!("    write       Write text to a file");
    kprintln!("    rm          Delete a file");
    kprintln!("    touch       Create an empty file");
    kprintln!("    stat        File metadata");
    kprintln!("    df          Filesystem usage");

    kprint_colored!(Colors::PURPLE, "  System\n");
    kprintln!("    info        System information");
    kprintln!("    version     Detailed version information");
    kprintln!("    version --all  Full build, boot, and subsystem report");
    kprintln!("    cpu         CPU vendor and feature flags");
    kprintln!("    mem         Physical memory statistics");
    kprintln!("    time        Uptime as HH:MM:SS");
    kprintln!("    uptime      Uptime in ticks and seconds");
    kprintln!("    date        Current kernel date/time view");
    kprintln!("    whoami      Current shell identity");
    kprintln!("    uname       Kernel and architecture string");
    kprintln!("    neofetch    System summary display");
    kprintln!("    lspci       Enumerate basic PCI devices");

    kprintln!();
    kprint_colored!(Colors::PURPLE, "  Quantum & Crypto\n");
    kprintln!("    quantum     Quantum subsystem status");
    kprintln!("    crypto      Post-quantum crypto status");
    kprintln!("    qalloc      Allocate qubit register: qalloc <1-18>");
    kprintln!("    qrun        Apply gate: qrun <gate> <qubit(s)>");
    kprintln!("    qstate      Show current state vector");
    kprintln!("    qprobs      Show probability distribution");
    kprintln!("    qmeasure    Measure current register");
    kprintln!("    qcircuit    Run built-in quantum demo");
    kprintln!("    qinfo       Kernel quantum simulator info");
    kprintln!("    qsave       Save current circuit as QASM");
    kprintln!("    qexport     Export current circuit as QASM");
    kprintln!("    qresult     Save last measurement results");
    kprintln!("    qreset      Reset register to |0...0>");
    kprintln!("    qfree       Free current quantum register");

    kprintln!();
    kprint_colored!(Colors::PURPLE, "  Tools\n");
    kprintln!("    echo        Echo text");
    kprintln!("    hex         Hex dump memory: hex <addr> [len]");
    kprintln!("    color       Display color palette");
    kprintln!("    history     Show last 10 commands");
    kprintln!("    tasks       List background tasks");
    kprintln!("    spawn       Run a command as a background task");
    kprintln!("    kill        Stop a background task");

    kprintln!();
    kprint_colored!(Colors::PURPLE, "  Display\n");
    kprintln!("    clear       Clear screen");
    kprintln!("    banner      Show boot banner");

    kprintln!();
    kprint_colored!(Colors::PURPLE, "  Networking\n");
    kprintln!("    net         COM2 link commands (status, send, qsend, listen)");
    kprintln!("    ping        Ping another WarOS node over COM2");

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

fn cmd_version(args: &[&str]) {
    if matches!(args.first(), Some(&"--all")) {
        let stats = memory::stats();
        let vendor_leaf = __cpuid(0);
        let feature_leaf = __cpuid(1);
        let vendor_bytes = vendor_string_bytes(vendor_leaf.ebx, vendor_leaf.edx, vendor_leaf.ecx);
        let vendor = str::from_utf8(&vendor_bytes).unwrap_or("Unknown");
        let uptime = interrupts::tick_count() / u64::from(PIT_FREQUENCY_HZ);
        let hours = uptime / 3600;
        let minutes = (uptime % 3600) / 60;
        let seconds = uptime % 60;
        let file_count = fs::FILESYSTEM.lock().list().len();

        kprintln!("WarOS v{}", KERNEL_VERSION);
        kprintln!("  Kernel:       waros-kernel {}", KERNEL_VERSION);
        kprintln!("  Quantum:      in-kernel StateVector (18 qubits max)");
        kprintln!("  Crypto:       ML-KEM, ML-DSA, SLH-DSA, SHA-3");
        kprintln!("  Architecture: x86_64");
        kprintln!(
            "  CPU:          {} Family {} Model {}",
            vendor,
            cpu_family(feature_leaf.eax),
            cpu_model(feature_leaf.eax)
        );
        kprintln!(
            "  RAM:          {} MiB ({} frames)",
            (stats.total_frames * 4) / 1024,
            stats.total_frames
        );
        kprintln!("  Heap:         {} MiB", heap::HEAP_SIZE / (1024 * 1024));
        kprintln!("  Uptime:       {:02}:{:02}:{:02}", hours, minutes, seconds);
        kprintln!("  Boot time:    {} ms", boot_complete_ms());
        kprintln!("  Files:        {}", file_count);
        kprintln!("  Built:        {} (rustc nightly)", BUILD_DATE);
        kprintln!("  License:      Apache 2.0");
        kprintln!("  Repository:   github.com/WarEnterprise/waros");
        return;
    }

    kprintln!("WarOS kernel version information:");
    kprintln!("  Kernel:    v{}", KERNEL_VERSION);
    kprintln!("  Platform:  x86_64");
    kprintln!("  Toolchain: Rust nightly");
    kprintln!("  Boot:      bootloader BIOS/UEFI images");
    kprintln!("  Quantum:   Kernel simulator + Rust/Python SDK");
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

    let Some(end_address) = address.checked_add(length.saturating_sub(1) as u64) else {
        kprint_colored!(Colors::RED, "[ERR]");
        kprintln!(" address range overflow.");
        return;
    };
    if !memory::is_debug_readable(address) || !memory::is_debug_readable(end_address) {
        kprint_colored!(Colors::RED, "[ERR]");
        kprintln!(
            " address 0x{:016X} is outside the safe debug mapping range.",
            address
        );
        return;
    }

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
    kprintln!("  Backend:        Kernel StateVector Simulator");
    kprintln!("  Max qubits:     18 (kernel heap limited)");
    kprintln!("  QPU hardware:   Not detected");
    kprintln!("  QHAL drivers:   None loaded");
    kprintln!("  QEC engine:     Not initialized");
    kprintln!("  Quantum net:    Not available");
    kprintln!("  Shell commands: qalloc, qrun, qstate, qmeasure, qcircuit, qsave, qresult, qinfo");
    if let Some((qubits, bytes)) = quantum::active_register() {
        kprintln!("  Active reg:     {} qubits ({} bytes)", qubits, bytes);
    } else {
        kprintln!("  Active reg:     None");
    }
    kprintln!();
    kprint_colored!(Colors::YELLOW, "  Note: ");
    kprintln!("Running in kernel simulation mode.");
    kprintln!("  Type 'help quantum' for the quantum command reference.");
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

fn cmd_ls() {
    let filesystem = fs::FILESYSTEM.lock();
    if filesystem.list().is_empty() {
        kprintln!("WarFS is empty.");
        return;
    }

    for entry in filesystem.list() {
        kprintln!(
            "  {:<20} {:>6} B    {}{}",
            entry.name.as_str(),
            entry.data.len(),
            fs::format_timestamp(entry.modified_at),
            if entry.readonly { "  [ro]" } else { "" }
        );
    }
}

fn cmd_cat(args: &[&str]) {
    let Some(name) = args.first().copied() else {
        kprintln!("Usage: cat <file>");
        return;
    };

    let filesystem = fs::FILESYSTEM.lock();
    let Ok(data) = filesystem.read(name) else {
        kprint_colored!(Colors::RED, "[ERR]");
        kprintln!(" file not found.");
        return;
    };

    match str::from_utf8(data) {
        Ok(text) => {
            kprint!("{}", text);
            if !text.ends_with('\n') {
                kprintln!();
            }
        }
        Err(_) => {
            kprint_colored!(Colors::RED, "[ERR]");
            kprintln!(" file is not valid UTF-8 text.");
        }
    }
}

fn cmd_write(command_line: &str) {
    let mut parts = command_line.splitn(3, char::is_whitespace);
    let _ = parts.next();
    let Some(name) = parts.next() else {
        kprintln!("Usage: write <file> <text>");
        return;
    };
    let Some(text) = parts.next() else {
        kprintln!("Usage: write <file> <text>");
        return;
    };

    match fs::FILESYSTEM.lock().write(name, text.as_bytes()) {
        Ok(()) => {
            kprint_colored!(Colors::GREEN, "Wrote ");
            kprintln!("{} bytes to '{}'.", text.len(), name);
        }
        Err(error) => {
            kprint_colored!(Colors::RED, "[ERR]");
            kprintln!(" {}", error);
        }
    }
}

fn cmd_rm(args: &[&str]) {
    let Some(name) = args.first().copied() else {
        kprintln!("Usage: rm <file>");
        return;
    };

    match fs::FILESYSTEM.lock().delete(name) {
        Ok(()) => {
            kprint_colored!(Colors::GREEN, "Deleted ");
            kprintln!("'{}'.", name);
        }
        Err(error) => {
            kprint_colored!(Colors::RED, "[ERR]");
            kprintln!(" {}", error);
        }
    }
}

fn cmd_touch(args: &[&str]) {
    let Some(name) = args.first().copied() else {
        kprintln!("Usage: touch <file>");
        return;
    };

    let existed = fs::FILESYSTEM.lock().exists(name);
    match fs::FILESYSTEM.lock().touch(name) {
        Ok(()) => {
            kprint_colored!(Colors::GREEN, "{}", if existed { "Updated " } else { "Created " });
            kprintln!("'{}'.", name);
        }
        Err(error) => {
            kprint_colored!(Colors::RED, "[ERR]");
            kprintln!(" {}", error);
        }
    }
}

fn cmd_stat(args: &[&str]) {
    let Some(name) = args.first().copied() else {
        kprintln!("Usage: stat <file>");
        return;
    };

    let filesystem = fs::FILESYSTEM.lock();
    let Ok(entry) = filesystem.stat(name) else {
        kprint_colored!(Colors::RED, "[ERR]");
        kprintln!(" file not found.");
        return;
    };

    kprintln!("File: {}", entry.name.as_str());
    kprintln!("  Size:      {} bytes", entry.data.len());
    kprintln!("  Created:   {}", fs::format_timestamp(entry.created_at));
    kprintln!("  Modified:  {}", fs::format_timestamp(entry.modified_at));
    kprintln!("  Read-only: {}", if entry.readonly { "yes" } else { "no" });
}

fn cmd_df() {
    let filesystem = fs::FILESYSTEM.lock();
    kprintln!("WarFS usage:");
    kprintln!("  Files: {} / {}", filesystem.list().len(), fs::MAX_FILES);
    kprintln!(
        "  Used:  {} KiB",
        filesystem.used_space() / 1024
    );
    kprintln!(
        "  Free:  {} KiB",
        filesystem.free_space() / 1024
    );
    kprintln!("  Limit: {} KiB", fs::TOTAL_CAPACITY / 1024);
}

fn cmd_tasks() {
    let tasks = task::snapshot();
    kprintln!("  ID  NAME                           STATE");
    kprintln!("   0  shell                          running");
    if tasks.is_empty() {
        return;
    }

    for task in tasks {
        kprintln!(
            "  {:>2}  {:<30} {}",
            task.id,
            task.name,
            task_state_name(task.state)
        );
    }
}

fn cmd_spawn(command_line: &str) {
    let Some((_, remainder)) = command_line.split_once(char::is_whitespace) else {
        kprintln!("Usage: spawn <command>");
        return;
    };

    match task::spawn_command(remainder.trim()) {
        Ok(id) => {
            kprint_colored!(Colors::GREEN, "[Task {}]", id);
            kprintln!(" Started: {}", remainder.trim());
        }
        Err(error) => {
            kprint_colored!(Colors::RED, "[ERR]");
            kprintln!(" {}", error);
        }
    }
}

fn cmd_kill(args: &[&str]) {
    let Some(id) = args.first().and_then(|value| value.parse::<u64>().ok()) else {
        kprintln!("Usage: kill <id>");
        return;
    };

    match task::kill(id) {
        Ok(()) => {
            kprint_colored!(Colors::GREEN, "Killed ");
            kprintln!("task {}.", id);
        }
        Err(error) => {
            kprint_colored!(Colors::RED, "[ERR]");
            kprintln!(" {}", error);
        }
    }
}

fn cmd_date() {
    let ticks = interrupts::tick_count();
    kprintln!(
        "Kernel clock: {} since boot (RTC not synchronized)",
        fs::format_timestamp(ticks)
    );
}

fn cmd_whoami() {
    kprintln!("root@waros");
}

fn cmd_uname() {
    kprintln!("WarOS {} x86_64", KERNEL_VERSION);
}

fn cmd_neofetch() {
    let stats = memory::stats();
    let uptime = interrupts::tick_count() / u64::from(PIT_FREQUENCY_HZ);
    let hours = uptime / 3600;
    let minutes = (uptime % 3600) / 60;
    let seconds = uptime % 60;
    let file_count = fs::FILESYSTEM.lock().list().len();
    let vendor_leaf = __cpuid(0);
    let feature_leaf = __cpuid(1);
    let vendor_bytes = vendor_string_bytes(vendor_leaf.ebx, vendor_leaf.edx, vendor_leaf.ecx);
    let vendor = str::from_utf8(&vendor_bytes).unwrap_or("Unknown");

    kprintln!();
    kprint_colored!(Colors::PURPLE, "  W A R O S");
    kprintln!("                   waros@waros");
    kprint_colored!(Colors::CYAN, "  Quantum-Classical");
    kprintln!("       -----------");
    kprint_colored!(Colors::CYAN, "  Hybrid OS");
    kprintln!("               OS:       WarOS v{}", KERNEL_VERSION);
    kprintln!("                         Kernel:   waros-kernel {}", KERNEL_VERSION);
    kprintln!("                         Arch:     x86_64");
    kprintln!(
        "                         CPU:      {} Fam {} Mod {}",
        vendor,
        cpu_family(feature_leaf.eax),
        cpu_model(feature_leaf.eax)
    );
    kprintln!(
        "                         RAM:      {} MiB ({} frames)",
        (stats.total_frames * 4) / 1024,
        stats.total_frames
    );
    kprintln!("                         Heap:     {} MiB", heap::HEAP_SIZE / (1024 * 1024));
    kprintln!("                         Uptime:   {:02}:{:02}:{:02}", hours, minutes, seconds);
    kprintln!("                         Boot:     {} ms", boot_complete_ms());
    kprintln!("                         Quantum:  StateVector (18 qubits max)");
    kprintln!("                         Crypto:   ML-KEM, ML-DSA, SLH-DSA");
    kprintln!("                         Files:    {}", file_count);
    kprintln!("                         License:  Apache 2.0");
}

fn cmd_lspci() {
    kprintln!("PCI devices (bus 0 scan):");
    let mut found = false;
    for device in 0u8..32 {
        for function in 0u8..8 {
            let vendor_id = pci_config_read(bus_device_function(0, device, function), 0) & 0xFFFF;
            if vendor_id == 0xFFFF {
                if function == 0 {
                    break;
                }
                continue;
            }

            let device_info = pci_config_read(bus_device_function(0, device, function), 0);
            let class_info = pci_config_read(bus_device_function(0, device, function), 8);
            let device_id = (device_info >> 16) & 0xFFFF;
            let class_code = (class_info >> 24) & 0xFF;
            let subclass = (class_info >> 16) & 0xFF;
            kprintln!(
                "  00:{:02X}.{}  vendor={:04X} device={:04X} class={:02X}:{:02X}",
                device,
                function,
                vendor_id,
                device_id,
                class_code,
                subclass
            );
            found = true;
        }
    }

    if !found {
        kprintln!("  No PCI devices detected on bus 0.");
    }
}

fn cmd_net(command_line: &str) {
    let mut parts = command_line.splitn(3, char::is_whitespace);
    let _ = parts.next();
    let Some(subcommand) = parts.next() else {
        kprintln!("Usage: net <status|send|qsend|listen>");
        return;
    };

    match subcommand {
        "status" => {
            kprintln!("Network interface: {}", net::status());
        }
        "send" => {
            let Some(text) = parts.next() else {
                kprintln!("Usage: net send <text>");
                return;
            };
            match net::send_text(text) {
                Ok(()) => {
                    kprint_colored!(Colors::GREEN, "Sent ");
                    kprintln!("text message over COM2.");
                }
                Err(_error) => {
                    kprint_colored!(Colors::RED, "[ERR]");
                    kprintln!(" failed to send text frame.");
                }
            }
        }
        "qsend" => {
            let Some(name) = parts.next() else {
                kprintln!("Usage: net qsend <file>");
                return;
            };
            let filesystem = fs::FILESYSTEM.lock();
            let Ok(data) = filesystem.read(name) else {
                kprint_colored!(Colors::RED, "[ERR]");
                kprintln!(" file not found.");
                return;
            };
            match str::from_utf8(data) {
                Ok(qasm) => match net::send_circuit(qasm) {
                    Ok(()) => {
                        kprint_colored!(Colors::GREEN, "Sent ");
                        kprintln!("'{}' over COM2.", name);
                    }
                    Err(_error) => {
                        kprint_colored!(Colors::RED, "[ERR]");
                        kprintln!(" failed to send circuit frame.");
                    }
                },
                Err(_) => {
                    kprint_colored!(Colors::RED, "[ERR]");
                    kprintln!(" file is not UTF-8 text.");
                }
            }
        }
        "listen" => {
            let mut received = 0usize;
            while let Some(message) = net::receive() {
                received += 1;
                match message.msg_type {
                    net::MessageType::Ping => kprintln!("[NET] Received ping."),
                    net::MessageType::Pong => kprintln!("[NET] Received pong."),
                    net::MessageType::CircuitData => {
                        kprintln!("[NET] Circuit payload:");
                        if let Ok(text) = str::from_utf8(&message.payload) {
                            kprintln!("{}", text);
                        }
                    }
                    net::MessageType::MeasurementResult => {
                        kprintln!("[NET] Measurement result payload:");
                        if let Ok(text) = str::from_utf8(&message.payload) {
                            kprintln!("{}", text);
                        }
                    }
                    net::MessageType::Text => {
                        if let Ok(text) = str::from_utf8(&message.payload) {
                            kprintln!("[NET] {}", text);
                        }
                    }
                }
            }
            if received == 0 {
                kprintln!("No pending COM2 messages.");
            }
        }
        _ => {
            kprintln!("Unknown net command '{}'.", subcommand);
        }
    }
}

fn cmd_ping() {
    if net::NET
        .lock()
        .send(net::MessageType::Ping, b"ping")
        .is_err()
    {
        kprint_colored!(Colors::RED, "[ERR]");
        kprintln!(" failed to send ping.");
        return;
    }

    for _ in 0..100_000 {
        if let Some(message) = net::receive() {
            if message.msg_type == net::MessageType::Pong {
                kprint_colored!(Colors::GREEN, "Ping");
                kprintln!(" response received.");
                return;
            }
        }
    }

    kprint_colored!(Colors::YELLOW, "[WARN]");
    kprintln!(" no pong received.");
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

fn task_state_name(state: task::TaskState) -> &'static str {
    match state {
        task::TaskState::Ready => "ready",
        task::TaskState::Running => "running",
        task::TaskState::Waiting => "waiting",
        task::TaskState::Completed => "completed",
    }
}

fn bus_device_function(bus: u8, device: u8, function: u8) -> u32 {
    (u32::from(bus) << 16) | (u32::from(device) << 11) | (u32::from(function) << 8)
}

fn pci_config_read(address: u32, register: u8) -> u32 {
    let config_address = 0x8000_0000u32 | address | (u32::from(register) & 0xFC);
    port::outl(0xCF8, config_address);
    port::inl(0xCFC)
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

fn cpu_family(eax: u32) -> u32 {
    let base_family = (eax >> 8) & 0x0F;
    let ext_family = (eax >> 20) & 0xFF;
    if base_family == 0x0F {
        base_family + ext_family
    } else {
        base_family
    }
}

fn cpu_model(eax: u32) -> u32 {
    let base_family = (eax >> 8) & 0x0F;
    let base_model = (eax >> 4) & 0x0F;
    let ext_model = (eax >> 16) & 0x0F;
    if base_family == 0x06 || base_family == 0x0F {
        base_model | (ext_model << 4)
    } else {
        base_model
    }
}

fn emit_feature(enabled: bool, name: &str, any: &mut bool) {
    if enabled {
        *any = true;
        kprint_colored!(Colors::CYAN, "{} ", name);
    }
}

fn parse_u64(value: &str) -> Option<u64> {
    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        u64::from_str_radix(hex, 16).ok()
    } else {
        value
            .parse::<u64>()
            .ok()
            .or_else(|| u64::from_str_radix(value, 16).ok())
    }
}

fn parse_usize(value: &str) -> Option<usize> {
    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
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
        // SAFETY: `cmd_hex` only calls this helper after validating that the full range lies in
        // the kernel's direct-physical-memory or heap debug mappings.
        *(address as *const u8)
    }
}
