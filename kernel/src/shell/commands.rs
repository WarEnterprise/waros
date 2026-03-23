use alloc::vec::Vec;

use core::arch::x86_64::__cpuid;
use core::str;

use crate::auth::{self, UserRole, USER_DB};
use crate::arch::x86_64::interrupts;
use crate::arch::x86_64::pit::PIT_FREQUENCY_HZ;
use crate::arch::x86_64::port;
use crate::display::branding;
use crate::display::console::{self, Colors};
use crate::disk;
use crate::drivers::keyboard;
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
        "startx" | "gui" => cmd_startx(),
        "ls" => cmd_ls(&parts[1..]),
        "cat" => cmd_cat(&parts[1..]),
        "write" => cmd_write(command_line),
        "rm" => cmd_rm(&parts[1..]),
        "touch" => cmd_touch(&parts[1..]),
        "stat" => cmd_stat(&parts[1..]),
        "df" => cmd_df(),
        "disk" => cmd_disk(),
        "sync" => cmd_sync(),
        "mount" => cmd_mount(),
        "format" => cmd_format_disk(),
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
        "keyboard" => cmd_keyboard(&parts[1..]),
        "quantum" => cmd_quantum(),
        "crypto" => cmd_crypto(),
        "ifconfig" => cmd_ifconfig(),
        "net" => cmd_net(command_line),
        "ping" => cmd_ping(&parts[1..]),
        "dns" => cmd_dns(&parts[1..]),
        "wget" => cmd_wget(&parts[1..]),
        "curl" => cmd_curl(&parts[1..]),
        "useradd" => cmd_useradd(&parts[1..]),
        "userdel" => cmd_userdel(&parts[1..]),
        "passwd" => cmd_passwd(&parts[1..]),
        "users" => cmd_users(),
        "su" => cmd_su(&parts[1..]),
        "logout" => cmd_logout(),
        "chmod" => cmd_chmod(&parts[1..]),
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
        kprintln!("  disk           Show persistent disk status");
        kprintln!("  sync           Force sync RAM files to disk");
        kprintln!("  mount          Show mounted filesystem mode");
        kprintln!("  format         Format the mounted disk");
        return;
    }

    kprint_colored!(
        Colors::CYAN,
        "WarOS v{} - Quantum-Classical Hybrid Operating System\n",
        KERNEL_VERSION
    );
    kprintln!("War Enterprise | warenterprise.com/waros | Apache 2.0");
    kprint_colored!(
        Colors::DIM,
        "----------------------------------------------------------------\n"
    );
    kprintln!();

    kprint_colored!(Colors::PURPLE, "System\n");
    kprintln!("  info            cpu             mem             time");
    kprintln!("  date            uptime          version         neofetch");
    kprintln!("  uname           whoami          users           lspci");
    kprintln!();

    kprint_colored!(Colors::PURPLE, "Quantum\n");
    kprintln!("  quantum         qalloc <n>      qrun <gate>     qstate");
    kprintln!("  qprobs          qmeasure        qcircuit        qinfo");
    kprintln!("  qsave           qexport         qresult         qreset");
    kprintln!("  qfree           crypto");
    kprintln!();

    kprint_colored!(Colors::PURPLE, "Network\n");
    kprintln!("  ifconfig        ping <host>     dns <domain>    wget <url>");
    kprintln!("  curl <url>      net status      net diag        net poll");
    kprintln!("  net txprobe     net send <txt>  net qsend <f>   net listen");
    kprintln!();

    kprint_colored!(Colors::PURPLE, "Filesystem\n");
    kprintln!("  ls              cat <file>      write <f> <t>   rm <file>");
    kprintln!("  touch <file>    stat <file>     chmod <mode> <file>  df");
    kprintln!("  disk            sync            mount               format");
    kprintln!();

    kprint_colored!(Colors::PURPLE, "Tools\n");
    kprintln!("  echo            hex <addr> [n]  color           history");
    kprintln!("  tasks           spawn <cmd>     kill <id>       banner");
    kprintln!("  keyboard <us|br>  useradd <name>  userdel <name>  passwd [name]");
    kprintln!("  su <name>       logout          startx|gui");
    kprintln!();

    kprint_colored!(Colors::PURPLE, "Control\n");
    kprintln!("  clear           halt            reboot          panic");
    kprintln!("  waros           help <topic>");
    kprintln!();
    kprint_colored!(
        Colors::DIM,
        "Type 'help quantum' or 'help fs' for focused command details.\n"
    );
}

fn cmd_info() {
    kprintln!("WarOS v{} - Quantum-Classical Hybrid Operating System", KERNEL_VERSION);
    kprintln!("Architecture: x86_64");
    kprintln!("Kernel: waros-kernel {}", KERNEL_VERSION);
    kprintln!("Boot mode: BIOS via bootloader");
    kprintln!("Timer: {} Hz PIT", PIT_FREQUENCY_HZ);
    kprintln!("War Enterprise - Building the future of computing");
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
        kprintln!("  Identity:     War Enterprise | Florianopolis, Brazil");
        kprintln!("  Tagline:      Building the future of computing");
        kprintln!("  License:      Apache 2.0");
        kprintln!("  Repository:   github.com/WarEnterprise/waros");
        return;
    }

    kprintln!("WarOS v{} (waros-kernel {})", KERNEL_VERSION, KERNEL_VERSION);
    kprintln!("Built: {} | Rust nightly", BUILD_DATE);
    kprintln!("War Enterprise - Building the future of computing");
    kprintln!("warenterprise.com/waros | github.com/WarEnterprise/waros");
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

fn cmd_keyboard(args: &[&str]) {
    let Some(layout) = args.first().copied() else {
        let current = keyboard::current_layout();
        kprintln!("Keyboard layout: {}", keyboard::layout_name(current));
        kprintln!("Usage: keyboard <us|br>");
        return;
    };

    match layout {
        "us" => {
            keyboard::set_layout(keyboard::KeyboardLayout::UsQwerty);
            kprintln!("[WarOS] INPUT: keyboard layout set to US QWERTY.");
        }
        "br" => {
            keyboard::set_layout(keyboard::KeyboardLayout::BrazilAbnt2);
            kprintln!("[WarOS] INPUT: keyboard layout set to Brazilian ABNT2 mode.");
            kprintln!("               Start QEMU with '-k pt-br' for correct host translation.");
        }
        _ => {
            kprint_colored!(Colors::RED, "[WarOS] INPUT:");
            kprintln!(" unknown layout '{}'. Use 'us' or 'br'.", layout);
        }
    }
}

fn cmd_useradd(args: &[&str]) {
    if !auth::session::is_admin() {
        kprint_colored!(Colors::RED, "[WarOS] ");
        kprintln!("Permission denied. Only admin can create users.");
        return;
    }

    let Some(username) = args.first().copied() else {
        kprintln!("Usage: useradd <username> [--admin]");
        return;
    };

    let role = if args.get(1) == Some(&"--admin") {
        UserRole::Admin
    } else {
        UserRole::User
    };

    let mut db = USER_DB.lock();
    match db.create_user(username, "changeme", role) {
        Ok(uid) => {
            db.save_to_fs();
            kprint_colored!(Colors::GREEN, "[WarOS] ");
            kprintln!(
                "User '{}' created (uid={}, role={}).",
                username,
                uid,
                role.as_str()
            );
            kprintln!("  Temporary password: changeme");
        }
        Err(error) => {
            kprint_colored!(Colors::RED, "[WarOS] ");
            kprintln!("Failed to create user: {}.", error);
        }
    }
}

fn cmd_userdel(args: &[&str]) {
    if !auth::session::is_admin() {
        kprint_colored!(Colors::RED, "[WarOS] ");
        kprintln!("Permission denied. Only admin can delete users.");
        return;
    }

    let Some(username) = args.first().copied() else {
        kprintln!("Usage: userdel <username>");
        return;
    };

    let mut db = USER_DB.lock();
    let Some(user) = db.find_by_name(username).cloned() else {
        kprintln!("User '{}' not found.", username);
        return;
    };

    match db.delete_user(user.uid) {
        Ok(()) => {
            db.save_to_fs();
            kprint_colored!(Colors::GREEN, "[WarOS] ");
            kprintln!("User '{}' deleted.", username);
        }
        Err(error) => {
            kprint_colored!(Colors::RED, "[WarOS] ");
            kprintln!("Failed to delete user: {}.", error);
        }
    }
}

fn cmd_passwd(args: &[&str]) {
    let Some(current) = auth::session::current_user() else {
        kprint_colored!(Colors::RED, "[WarOS] ");
        kprintln!("No active session.");
        return;
    };

    let target_name = args.first().copied().unwrap_or(&current.username);
    if target_name != current.username && current.role != UserRole::Admin {
        kprint_colored!(Colors::RED, "[WarOS] ");
        kprintln!("Permission denied. Use 'passwd' without arguments for your own account.");
        return;
    }

    kprint!("New password: ");
    let new_password = auth::login::read_line_hidden();
    kprintln!();
    kprint!("Confirm: ");
    let confirm = auth::login::read_line_hidden();
    kprintln!();

    if new_password != confirm {
        kprint_colored!(Colors::RED, "[WarOS] ");
        kprintln!("Passwords do not match.");
        return;
    }

    let mut db = USER_DB.lock();
    let Some(target) = db.find_by_name(target_name).cloned() else {
        kprintln!("User '{}' not found.", target_name);
        return;
    };

    match db.change_password(target.uid, &new_password) {
        Ok(()) => {
            db.save_to_fs();
            kprint_colored!(Colors::GREEN, "[WarOS] ");
            kprintln!("Password changed for '{}'.", target_name);
        }
        Err(error) => {
            kprint_colored!(Colors::RED, "[WarOS] ");
            kprintln!("Failed to change password: {}.", error);
        }
    }
}

fn cmd_users() {
    let Some(current) = auth::session::current_user() else {
        kprint_colored!(Colors::RED, "[WarOS] ");
        kprintln!("No active session.");
        return;
    };

    let db = USER_DB.lock();
    kprintln!("  UID  USERNAME        ROLE     HOME               FILES");
    for user in db.list_users() {
        if current.role == UserRole::Admin || user.uid == current.uid {
            kprintln!(
                "  {:>3}  {:<15} {:<8} {:<18} {}",
                user.uid,
                user.username,
                user.role.as_str(),
                user.home_dir,
                fs::file_count_for_user(user.uid)
            );
        }
    }
}

fn cmd_su(args: &[&str]) {
    let Some(username) = args.first().copied() else {
        kprintln!("Usage: su <username>");
        return;
    };

    let is_admin = auth::session::is_admin();
    let mut db = USER_DB.lock();
    let target = if is_admin {
        db.find_by_name(username).cloned()
    } else {
        kprint!("Password: ");
        let password = auth::login::read_line_hidden();
        kprintln!();
        db.authenticate(username, &password).ok()
    };

    match target {
        Some(user) => {
            let _ = db.record_login(user.uid);
            db.save_to_fs();
            drop(db);
            auth::session::start(user.clone());
            kprint_colored!(Colors::GREEN, "[WarOS] ");
            kprintln!("Switched to user '{}'.", user.username);
        }
        None => {
            drop(db);
            kprint_colored!(Colors::RED, "[WarOS] ");
            kprintln!("Authentication failed.");
        }
    }
}

fn cmd_logout() {
    kprintln!("Logging out...");
    auth::session::logout();
}

fn cmd_startx() {
    if crate::gui::is_active() {
        kprintln!("[WarOS] GUI is already active.");
        return;
    }

    crate::gui::start_gui();
}

fn cmd_chmod(args: &[&str]) {
    let Some(mode) = args.first().copied() else {
        kprintln!("Usage: chmod <mode> <file>");
        kprintln!("  Example: chmod rw-- notes.txt");
        return;
    };
    let Some(path) = args.get(1).copied() else {
        kprintln!("Usage: chmod <mode> <file>");
        return;
    };

    match fs::chmod_current(path, mode) {
        Ok(resolved) => {
            kprint_colored!(Colors::GREEN, "[WarOS] ");
            kprintln!("Updated permissions on '{}' to {}.", resolved, mode);
        }
        Err(error) => report_fs_error(path, error),
    }
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

fn cmd_ls(args: &[&str]) {
    let target = args.first().copied();
    match fs::list_current(target) {
        Ok((directory, entries)) => {
            if entries.is_empty() {
                kprintln!("No files in {}.", fs::display_path(&directory));
                return;
            }

            kprintln!("  OWNER      MODE  SIZE     MODIFIED   NAME");
            for entry in entries {
                kprintln!(
                    "  {:<10} {:<4} {:>6} B  {:<8} {}{}",
                    auth::username_for_uid(entry.owner_uid),
                    entry.permissions.mode_string(),
                    entry.data.len(),
                    fs::format_timestamp(entry.modified_at),
                    fs::basename(&entry.name),
                    if entry.readonly { "  [ro]" } else { "" }
                );
            }
        }
        Err(error) => report_fs_error(target.unwrap_or("~"), error),
    }
}

fn cmd_cat(args: &[&str]) {
    let Some(name) = args.first().copied() else {
        kprintln!("Usage: cat <file>");
        return;
    };

    let (path, data) = match fs::read_current(name) {
        Ok(result) => result,
        Err(error) => {
            report_fs_error(name, error);
            return;
        }
    };

    match str::from_utf8(&data) {
        Ok(text) => {
            kprint!("{}", text);
            if !text.ends_with('\n') {
                kprintln!();
            }
        }
        Err(_) => {
            kprint_colored!(Colors::RED, "[ERR]");
            kprintln!(" '{}' is not valid UTF-8 text.", fs::display_path(&path));
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

    match fs::write_current(name, text.as_bytes()) {
        Ok(path) => {
            kprint_colored!(Colors::GREEN, "Wrote ");
            kprintln!("{} bytes to '{}'.", text.len(), path);
        }
        Err(error) => report_fs_error(name, error),
    }
}

fn cmd_rm(args: &[&str]) {
    let Some(name) = args.first().copied() else {
        kprintln!("Usage: rm <file>");
        return;
    };

    match fs::delete_current(name) {
        Ok(path) => {
            kprint_colored!(Colors::GREEN, "Deleted ");
            kprintln!("'{}'.", path);
        }
        Err(error) => report_fs_error(name, error),
    }
}

fn cmd_touch(args: &[&str]) {
    let Some(name) = args.first().copied() else {
        kprintln!("Usage: touch <file>");
        return;
    };

    let existed = fs::stat_current(name).is_ok();
    match fs::touch_current(name) {
        Ok(path) => {
            kprint_colored!(Colors::GREEN, "{}", if existed { "Updated " } else { "Created " });
            kprintln!("'{}'.", path);
        }
        Err(error) => report_fs_error(name, error),
    }
}

fn cmd_stat(args: &[&str]) {
    let Some(name) = args.first().copied() else {
        kprintln!("Usage: stat <file>");
        return;
    };

    let entry = match fs::stat_current(name) {
        Ok(entry) => entry,
        Err(error) => {
            report_fs_error(name, error);
            return;
        }
    };

    kprintln!("File: {}", fs::display_path(&entry.name));
    kprintln!("  Size:      {} bytes", entry.data.len());
    kprintln!("  Created:   {}", fs::format_timestamp(entry.created_at));
    kprintln!("  Modified:  {}", fs::format_timestamp(entry.modified_at));
    kprintln!("  Owner:     {}", auth::username_for_uid(entry.owner_uid));
    kprintln!("  Perms:     {}", entry.permissions.mode_string());
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

fn cmd_disk() {
    match disk::disk_status() {
        Ok(Some(status)) => {
            let used_bytes = status.used_blocks as u64 * disk::format::BLOCK_SIZE as u64;
            let free_blocks = status.total_blocks.saturating_sub(status.used_blocks);
            let free_bytes = free_blocks as u64 * disk::format::BLOCK_SIZE as u64;
            kprintln!("WarFS Disk:");
            kprintln!(
                "  Device:     virtio-blk (PCI {:02X}:{:02X}.{})",
                status.bus,
                status.device,
                status.function
            );
            kprintln!("  I/O Base:   0x{:04X}", status.io_base);
            kprintln!(
                "  Capacity:   {} MB ({} sectors)",
                status.disk_size / (1024 * 1024),
                status.capacity_sectors
            );
            kprintln!("  Format:     WarFS v{}", status.version);
            kprintln!(
                "  Used:       {} blocks ({} KiB)",
                status.used_blocks,
                used_bytes / 1024
            );
            kprintln!(
                "  Free:       {} blocks ({} KiB)",
                free_blocks,
                free_bytes / 1024
            );
            kprintln!("  Files:      {}", status.file_count);
            kprintln!("  State:      {}", disk::state_label(status.state));
            kprintln!("  Mounted at: /");
        }
        Ok(None) => {
            kprint_colored!(Colors::YELLOW, "[INFO]");
            kprintln!(" no virtio-blk disk is mounted. WarFS is running in RAM-only mode.");
        }
        Err(error) => {
            kprint_colored!(Colors::RED, "[ERR]");
            kprintln!(" unable to read disk status: {}.", error);
        }
    }
}

fn cmd_sync() {
    if !disk::is_available() {
        kprint_colored!(Colors::YELLOW, "[INFO]");
        kprintln!(" no disk is mounted. WarFS is already running in RAM-only mode.");
        return;
    }

    match disk::sync_all() {
        Ok(count) => {
            kprint_colored!(Colors::GREEN, "Synced ");
            kprintln!("{} files to disk.", count);
        }
        Err(error) => {
            kprint_colored!(Colors::RED, "[ERR]");
            kprintln!(" sync failed: {}.", error);
        }
    }
}

fn cmd_mount() {
    match disk::mount_mode() {
        disk::MountMode::RamOnly => {
            kprintln!("Mounted filesystems:");
            kprintln!("  /  warfs  (ram-only)");
        }
        disk::MountMode::DiskBacked { version, disk_size } => {
            kprintln!("Mounted filesystems:");
            kprintln!(
                "  /  warfs  (disk-backed, WarFS v{}, {} MB)",
                version,
                disk_size / (1024 * 1024)
            );
        }
    }
}

fn cmd_format_disk() {
    if !auth::session::is_admin() {
        kprint_colored!(Colors::RED, "[WarOS] ");
        kprintln!("Permission denied. Only admin can format the disk.");
        return;
    }

    match disk::format_active() {
        Ok(true) => {
            kprint_colored!(Colors::YELLOW, "[WARN] ");
            kprintln!("Disk formatted. Current RAM files remain loaded until reboot or resync.");
        }
        Ok(false) => {
            kprint_colored!(Colors::YELLOW, "[INFO] ");
            kprintln!("No virtio-blk disk is mounted.");
        }
        Err(error) => {
            kprint_colored!(Colors::RED, "[ERR]");
            kprintln!(" disk format failed: {}.", error);
        }
    }
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
    if let Some(user) = auth::session::current_user() {
        kprintln!(
            "{} (uid={}, role={})",
            user.username,
            user.uid,
            user.role.as_str()
        );
    } else {
        kprintln!("not logged in");
    }
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
    let net_summary = net::network_config()
        .map(|config| config.cidr_string())
        .unwrap_or_else(|| "offline".into());

    kprintln!();
    kprint_colored!(Colors::GREEN, "     _       __           ____  _____");
    kprintln!("      waros@warenterprise");
    kprint_colored!(Colors::GREEN, "    | |     / /___ ______/ __ \\/ ___/");
    kprintln!("      ----------------------------");
    kprint_colored!(Colors::GREEN, "    | | /| / / __ `/ ___/ / / /\\__ \\");
    kprintln!("      OS:      WarOS v{}", KERNEL_VERSION);
    kprint_colored!(Colors::GREEN, "    | |/ |/ / /_/ / /  / /_/ /___/ /");
    kprintln!("      Kernel:  waros-kernel {}", KERNEL_VERSION);
    kprint_colored!(Colors::GREEN, "    |__/|__/\\__,_/_/   \\____//____/");
    kprintln!("      Arch:    x86_64");
    kprintln!(
        "      CPU:     {} Fam {} Mod {}",
        vendor,
        cpu_family(feature_leaf.eax),
        cpu_model(feature_leaf.eax)
    );
    kprintln!(
        "      RAM:     {} MiB ({} frames)",
        (stats.total_frames * 4) / 1024,
        stats.total_frames
    );
    kprintln!("      Heap:    {} MiB", heap::HEAP_SIZE / (1024 * 1024));
    kprintln!("      Uptime:  {:02}:{:02}:{:02}", hours, minutes, seconds);
    kprintln!("      Boot:    {} ms", boot_complete_ms());
    kprintln!("      Net:     {}", net_summary);
    kprintln!("      Quantum: 18 qubits (StateVector)");
    kprintln!("      Crypto:  ML-KEM + ML-DSA + SHA-3");
    kprintln!("      FS:      WarFS ({} files)", file_count);
    kprintln!("      Shell:   WarShell v{}", KERNEL_VERSION);
    kprintln!("      Origin:  Florianopolis, SC, Brazil");
    kprintln!("      Motto:   Building the future of computing");
}

fn cmd_lspci() {
    let devices = net::pci_devices();
    kprintln!("PCI devices ({} detected):", devices.len());
    if devices.is_empty() {
        kprintln!("  No PCI devices detected.");
        return;
    }

    for device in devices {
        let (bar_kind, bar_value) = match device.bar(0) {
            net::pci::PciBar::Io(base) => ("io", u64::from(base)),
            net::pci::PciBar::Memory32(base) => ("mmio32", u64::from(base)),
            net::pci::PciBar::Memory64(base) => ("mmio64", base),
            net::pci::PciBar::Unused => ("none", 0),
        };
        kprintln!(
            "  {:02X}:{:02X}.{}  vendor={:04X} device={:04X} class={:02X}:{:02X} {}={} ({})",
            device.bus,
            device.device,
            device.function,
            device.vendor_id,
            device.device_id,
            device.class_code,
            device.subclass,
            bar_kind,
            bar_value,
            net::pci::class_name(device.class_code, device.subclass)
        );
    }
}

fn cmd_net(command_line: &str) {
    let mut parts = command_line.splitn(3, char::is_whitespace);
    let _ = parts.next();
    let Some(subcommand) = parts.next() else {
        kprintln!("Usage: net <status|diag|poll|txprobe|send|qsend|listen>");
        return;
    };

    match subcommand {
        "status" => {
            kprintln!("Network stack: {}", net::status());
            kprintln!("PCI inventory: {} device(s)", net::pci_devices().len());
            if let Some(hardware) = net::hardware_status() {
                kprintln!("  MAC:       {}", net::format_mac(&hardware.mac));
                kprintln!("  RX queue:  {}", hardware.rx_queue_size);
                kprintln!("  TX queue:  {}", hardware.tx_queue_size);
                kprintln!("  IRQ line:  {}", hardware.interrupt_line);
                kprintln!("  RX/TX:     {}/{}", hardware.rx_frames, hardware.tx_frames);
            }
            if let Some(config) = net::network_config() {
                kprintln!("  IPv4:      {}", config.cidr_string());
                if let Some(gateway) = config.gateway {
                    kprintln!("  Gateway:   {}", gateway);
                }
                if let Some(dns_server) = config.dns_server {
                    kprintln!("  DNS:       {}", dns_server);
                }
            }
            kprintln!("  ARP cache: {} entrie(s)", net::arp_entries().len());
            kprintln!("  DNS cache: {} entrie(s)", net::dns_cache().len());
        }
        "diag" => {
            let Some(diag) = net::hardware_diagnostics() else {
                kprint_colored!(Colors::RED, "[WarOS] NET:");
                kprintln!(" virtio-net is not initialized.");
                return;
            };

            let target = net::network_config()
                .and_then(|config| config.gateway)
                .unwrap_or(net::ipv4::Ipv4Addr::new(10, 0, 2, 2));
            let rx_before = diag.rx_frames;
            let tx_before = diag.tx_frames;

            kprintln!("WarOS Network Diagnostics");
            kprintln!("  Device status: 0x{:02X}", diag.device_status);
            kprintln!("  PCI command:   0x{:04X}", diag.pci_command);
            kprintln!(
                "  RX queue:      size={} avail={} used={} processed={} buffers={}",
                diag.rx_queue.size,
                diag.rx_queue.avail_idx,
                diag.rx_queue.used_idx,
                diag.rx_queue.last_used_idx,
                diag.rx_buffers
            );
            kprintln!(
                "  TX queue:      size={} avail={} used={} processed={} free={}/{}",
                diag.tx_queue.size,
                diag.tx_queue.avail_idx,
                diag.tx_queue.used_idx,
                diag.tx_queue.last_used_idx,
                diag.tx_free,
                diag.tx_buffers
            );
            kprintln!(
                "  Frames:        tx={} rx={} pending={}",
                diag.tx_frames,
                diag.rx_frames,
                diag.pending_frames
            );
            kprintln!("  ARP probe:     who-has {}", target);

            match net::send_arp_probe(target) {
                Ok(()) => kprintln!("  Probe status:  transmitted"),
                Err(error) => {
                    kprint_colored!(Colors::RED, "[WarOS] NET:");
                    kprintln!(" ARP probe failed: {}.", error);
                    return;
                }
            }

            let deadline = interrupts::tick_count().saturating_add(u64::from(PIT_FREQUENCY_HZ));
            let mut events = 0usize;
            while interrupts::tick_count() < deadline {
                events = events.saturating_add(net::poll());
            }

            if let Some(after) = net::hardware_diagnostics() {
                kprintln!(
                    "  After probe:   tx={} rx={} events={}",
                    after.tx_frames,
                    after.rx_frames,
                    events
                );
                kprintln!(
                    "  RX queue now:  avail={} used={} processed={}",
                    after.rx_queue.avail_idx,
                    after.rx_queue.used_idx,
                    after.rx_queue.last_used_idx
                );
                kprintln!(
                    "  TX queue now:  avail={} used={} processed={} free={}/{}",
                    after.tx_queue.avail_idx,
                    after.tx_queue.used_idx,
                    after.tx_queue.last_used_idx,
                    after.tx_free,
                    after.tx_buffers
                );
                if after.rx_frames > rx_before || after.tx_frames > tx_before {
                    kprint_colored!(Colors::GREEN, "  Traffic:       ");
                    kprintln!("frame counters moved after the ARP probe.");
                } else {
                    kprint_colored!(Colors::YELLOW, "  Traffic:       ");
                    kprintln!("no frame counters moved after the ARP probe.");
                }
            }

            if let Some(mac) = net::arp_lookup(target) {
                kprint_colored!(Colors::GREEN, "  ARP cache:     ");
                kprintln!("{} -> {}", target, net::format_mac(&mac));
            } else {
                kprint_colored!(Colors::YELLOW, "  ARP cache:     ");
                kprintln!("no entry for {} yet.", target);
            }
        }
        "poll" => {
            let harvested = net::poll();
            kprintln!("Polled network stack: {} event(s) processed.", harvested);
            while let Some(frame) = net::receive_raw_frame() {
                match net::ethernet::EthernetFrame::parse(&frame) {
                    Ok(ethernet) => {
                        kprintln!(
                            "  {} bytes type=0x{:04X} src={} dst={}",
                            frame.len(),
                            ethernet.ethertype(),
                            net::format_mac(&ethernet.src_mac()),
                            net::format_mac(&ethernet.dst_mac())
                        );
                    }
                    Err(_) => {
                        kprintln!("  {} bytes (unparsed)", frame.len());
                    }
                }
            }
        }
        "txprobe" => {
            let Some(hardware) = net::hardware_status() else {
                kprint_colored!(Colors::RED, "[ERR]");
                kprintln!(" virtio-net is not initialized.");
                return;
            };
            let frame = net::ethernet::EthernetFrame::new(
                [0xFF; 6],
                hardware.mac,
                0x88B5,
                Vec::from(&b"waros-phase-a-probe"[..]),
            )
            .serialize();
            match net::send_raw_frame(&frame) {
                Ok(()) => {
                    kprint_colored!(Colors::GREEN, "Sent ");
                    kprintln!("raw Ethernet probe frame ({} bytes).", frame.len());
                }
                Err(error) => {
                    kprint_colored!(Colors::RED, "[ERR]");
                    kprintln!(" failed to send raw frame: {}.", error);
                }
            }
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
            let (path, data) = match fs::read_current(name) {
                Ok(result) => result,
                Err(error) => {
                    report_fs_error(name, error);
                    return;
                }
            };
            match str::from_utf8(&data) {
                Ok(qasm) => match net::send_circuit(qasm) {
                    Ok(()) => {
                        kprint_colored!(Colors::GREEN, "Sent ");
                        kprintln!("'{}' over COM2.", path);
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
            kprintln!("[WarOS] NET: unknown subcommand '{}'.", subcommand);
        }
    }
}

fn cmd_ifconfig() {
    let Some(hardware) = net::hardware_status() else {
        kprint_colored!(Colors::YELLOW, "[WARN]");
        kprintln!(" no virtio-net device is active.");
        return;
    };

    kprintln!("Interface: virtio-net");
    kprintln!("  MAC:     {}", net::format_mac(&hardware.mac));
    if let Some(config) = net::network_config() {
        kprintln!("  IPv4:    {}", config.cidr_string());
        kprintln!("  Mask:    {}", config.subnet_mask);
        kprintln!(
            "  Gateway: {}",
            config.gateway.unwrap_or(net::ipv4::Ipv4Addr::ZERO)
        );
        kprintln!(
            "  DNS:     {}",
            config.dns_server.unwrap_or(net::ipv4::Ipv4Addr::ZERO)
        );
    } else {
        kprintln!("  IPv4:    unconfigured");
    }
}

fn cmd_ping(args: &[&str]) {
    let Some(target) = args.first().copied() else {
        kprintln!("Usage: ping <host>");
        return;
    };

    match net::ping_host(target) {
        Ok(reply) => {
            kprint_colored!(Colors::GREEN, "Reply ");
            kprintln!(
                "from {}: seq={} bytes={}",
                reply.source,
                reply.seq_no,
                reply.payload_len
            );
        }
        Err(error) => {
            kprint_colored!(Colors::RED, "[ERR]");
            kprintln!(" ping failed: {}.", error);
        }
    }
}

fn cmd_dns(args: &[&str]) {
    let Some(domain) = args.first().copied() else {
        kprintln!("Usage: dns <domain>");
        return;
    };

    match net::resolve_host(domain) {
        Ok(address) => kprintln!("{} -> {}", domain, address),
        Err(error) => {
            kprint_colored!(Colors::RED, "[ERR]");
            kprintln!(" DNS lookup failed: {}.", error);
        }
    }
}

fn cmd_wget(args: &[&str]) {
    let Some(url) = args.first().copied() else {
        kprintln!("Usage: wget <url>");
        return;
    };

    match net::http_get(url) {
        Ok(response) => print_http_body(&response.body),
        Err(error) => {
            kprint_colored!(Colors::RED, "[ERR]");
            kprintln!(" request failed: {}.", error);
        }
    }
}

fn cmd_curl(args: &[&str]) {
    let Some(url) = args.first().copied() else {
        kprintln!("Usage: curl <url>");
        return;
    };

    match net::http_get(url) {
        Ok(response) => {
            kprintln!("HTTP {}", response.status_code);
            for (name, value) in response.headers {
                kprintln!("{}: {}", name, value);
            }
            kprintln!();
            print_http_body(&response.body);
        }
        Err(error) => {
            kprint_colored!(Colors::RED, "[ERR]");
            kprintln!(" request failed: {}.", error);
        }
    }
}

fn print_http_body(body: &[u8]) {
    if let Ok(text) = str::from_utf8(body) {
        kprint!("{}", text);
        if !text.ends_with('\n') {
            kprintln!();
        }
    } else {
        kprint_colored!(Colors::RED, "[ERR]");
        kprintln!(" response body is not UTF-8 text.");
    }
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
    kprintln!("WarOS v{} - Quantum-Classical Hybrid Operating System", KERNEL_VERSION);
    kprintln!("War Enterprise - Building the future of computing");
    kprintln!("warenterprise.com/waros");
    kprintln!("github.com/WarEnterprise/waros");
}

fn cmd_unknown(command: &str) {
    kprint_colored!(Colors::RED, "[WarOS] ERROR:");
    kprintln!(
        " command '{}' not found. Type 'help' for available commands.",
        command
    );
}

fn report_fs_error(path: &str, error: fs::FsError) {
    match error {
        fs::FsError::PermissionDenied => {
            let resolved = auth::session::resolve_path(path);
            if let Some((uid, owner)) = fs::owner_label(&resolved) {
                kprint_colored!(Colors::RED, "[WarOS] PERMISSION DENIED:");
                kprintln!(
                    " {} is owned by {} (uid={})",
                    fs::display_path(&resolved),
                    owner,
                    uid
                );
            } else {
                kprint_colored!(Colors::RED, "[WarOS] PERMISSION DENIED:");
                kprintln!(" {}", fs::display_path(&resolved));
            }
        }
        _ => {
            kprint_colored!(Colors::RED, "[ERR]");
            kprintln!(" {}", error);
        }
    }
}

fn task_state_name(state: task::TaskState) -> &'static str {
    match state {
        task::TaskState::Ready => "ready",
        task::TaskState::Running => "running",
        task::TaskState::Waiting => "waiting",
        task::TaskState::Completed => "completed",
    }
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
