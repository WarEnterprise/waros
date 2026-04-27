use alloc::borrow::Cow;
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use core::arch::x86_64::{__cpuid, __cpuid_count};
use core::str;

use spin::Lazy;

use crate::arch::x86_64::interrupts;
use crate::arch::x86_64::pit::PIT_FREQUENCY_HZ;
use crate::auth::{self, UserRole, USER_DB};
use crate::disk;
use crate::drivers;
use crate::display::branding;
use crate::display::console::{self, Colors};
use crate::exec;
use crate::fs;
use crate::hal;
use crate::memory;
use crate::memory::heap;
use crate::net;
use crate::pkg;
use crate::quantum;
use crate::security;
use crate::shell::history;
use crate::task;
use crate::{
    boot_complete_ms, kprint, kprint_colored, kprintln, serial_println, BUILD_DATE, KERNEL_VERSION,
};

static ENV: Lazy<spin::Mutex<BTreeMap<String, String>>> = Lazy::new(|| {
    let mut map = BTreeMap::new();
    map.insert(String::from("PATH"), String::from("/bin"));
    map.insert(String::from("HOME"), String::from("/root"));
    map.insert(String::from("USER"), String::from("root"));
    map.insert(String::from("SHELL"), String::from("/bin/warsh"));
    map.insert(String::from("WAROS_VERSION"), KERNEL_VERSION.to_string());
    spin::Mutex::new(map)
});

static ALIASES: Lazy<spin::Mutex<BTreeMap<String, String>>> =
    Lazy::new(|| spin::Mutex::new(BTreeMap::new()));

fn expand_vars(input: &str) -> String {
    if !input.contains('$') {
        return input.to_string();
    }
    let mut result = String::new();
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '$' {
            let mut var_name = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_alphanumeric() || c == '_' {
                    var_name.push(c);
                    chars.next();
                } else {
                    break;
                }
            }
            if var_name.is_empty() {
                result.push('$');
            } else {
                let value = ENV.lock().get(&var_name).cloned().unwrap_or_default();
                result.push_str(&value);
            }
        } else {
            result.push(ch);
        }
    }
    result
}

fn capture_command_output(command_line: &str) -> String {
    console::begin_capture();
    execute_command(command_line);
    console::end_capture()
}

fn execute_piped(left: &str, right: &str) {
    let output = capture_command_output(left);
    let tmp_path = "/tmp/.pipe_buf";
    let _ = fs::write_current(tmp_path, output.as_bytes());

    // For 'grep pattern' without a file, append the temp file path.
    let right_parts: Vec<&str> = right.split_whitespace().collect();
    let right_expanded = if right.starts_with("grep ") && right_parts.len() == 2 {
        alloc::format!("{} {}", right, tmp_path)
    } else {
        right.to_string()
    };
    execute_command(&right_expanded);
    let _ = fs::delete_current(tmp_path);
}

fn find_redirect(command_line: &str, op: &str) -> Option<usize> {
    command_line.find(op)
}

fn execute_redirected(command_line: &str) {
    if let Some(pos) = find_redirect(command_line, " >> ") {
        let cmd = command_line[..pos].trim();
        let file = command_line[pos + 4..].trim();
        let output = capture_command_output(cmd);
        match fs::read_current(file) {
            Ok((_, mut existing)) => {
                existing.extend_from_slice(output.as_bytes());
                let _ = fs::write_current(file, &existing);
            }
            Err(_) => {
                let _ = fs::write_current(file, output.as_bytes());
            }
        }
    } else if let Some(pos) = find_redirect(command_line, " > ") {
        let cmd = command_line[..pos].trim();
        let file = command_line[pos + 3..].trim();
        let output = capture_command_output(cmd);
        match fs::write_current(file, output.as_bytes()) {
            Ok(_) => {}
            Err(e) => report_fs_error(file, e),
        }
    } else if let Some(pos) = find_redirect(command_line, " < ") {
        let file = command_line[pos + 3..].trim();
        match fs::read_current(file) {
            Ok((_, data)) => {
                if let Ok(text) = core::str::from_utf8(&data) {
                    kprint!("{text}");
                }
            }
            Err(e) => report_fs_error(file, e),
        }
    }
}

fn print_help_header() {
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
    kprintln!("Use 'help fs', 'help security', or 'help recovery' for focused topics.");
    kprintln!();
}

fn print_help_section(title: &str, entries: &[(&str, &str)]) {
    kprint_colored!(Colors::PURPLE, "{}\n", title);
    for (command, summary) in entries {
        kprintln!("  {:<32} {}", command, summary);
    }
    kprintln!();
}

fn usb_net_protocol_name(protocol: hal::usb::UsbNetProtocol) -> &'static str {
    match protocol {
        hal::usb::UsbNetProtocol::CdcEcm => "CDC-ECM",
        hal::usb::UsbNetProtocol::Rndis => "RNDIS",
        hal::usb::UsbNetProtocol::CdcNcm => "CDC-NCM",
    }
}

fn usb_visibility_name(visibility: hal::usb::UsbPortVisibility) -> &'static str {
    match visibility {
        hal::usb::UsbPortVisibility::Generic => "generic-usb",
        hal::usb::UsbPortVisibility::Input => "input",
        hal::usb::UsbPortVisibility::Storage => "storage",
        hal::usb::UsbPortVisibility::PhoneMedia => "phone-media",
        hal::usb::UsbPortVisibility::PhoneAdb => "phone-adb",
        hal::usb::UsbPortVisibility::VendorSpecific => "vendor-specific",
        hal::usb::UsbPortVisibility::NetworkCandidate => "usb-net-candidate",
        hal::usb::UsbPortVisibility::NetworkReady => "usb-net-ready",
    }
}

fn usb_attach_state_name(state: hal::usb::UsbAttachState) -> &'static str {
    match state {
        hal::usb::UsbAttachState::NotApplicable => "n/a",
        hal::usb::UsbAttachState::Unsupported => "unsupported",
        hal::usb::UsbAttachState::Candidate => "candidate",
        hal::usb::UsbAttachState::Ready => "ready",
    }
}

fn usb_category_name(category: hal::DeviceCategory) -> &'static str {
    match category {
        hal::DeviceCategory::Processor => "processor",
        hal::DeviceCategory::Memory => "memory",
        hal::DeviceCategory::Storage => "storage",
        hal::DeviceCategory::Input => "input",
        hal::DeviceCategory::Display => "display",
        hal::DeviceCategory::Network => "network",
        hal::DeviceCategory::UsbController => "usb-controller",
        hal::DeviceCategory::UsbDevice => "usb-device",
        hal::DeviceCategory::Audio => "audio",
        hal::DeviceCategory::QuantumProcessor => "quantum",
        hal::DeviceCategory::PowerManagement => "power",
        hal::DeviceCategory::Other => "other",
    }
}

fn usb_interface_summary(interface: &hal::usb::UsbInterfaceStatus) -> String {
    let mut signals = String::new();
    if interface.has_bulk_in {
        signals.push_str("bulk-in ");
    }
    if interface.has_bulk_out {
        signals.push_str("bulk-out ");
    }
    if interface.has_interrupt_in {
        signals.push_str("intr-in ");
    }
    if interface.has_interrupt_out {
        signals.push_str("intr-out ");
    }
    let signals = signals.trim();
    if signals.is_empty() {
        alloc::format!(
            "if{} alt{} {:02X}/{:02X}/{:02X} eps={}",
            interface.number,
            interface.alternate_setting,
            interface.class,
            interface.subclass,
            interface.protocol,
            interface.endpoint_count
        )
    } else {
        alloc::format!(
            "if{} alt{} {:02X}/{:02X}/{:02X} eps={} {}",
            interface.number,
            interface.alternate_setting,
            interface.class,
            interface.subclass,
            interface.protocol,
            interface.endpoint_count,
            signals
        )
    }
}

fn print_usb_snapshot_detail(indent: &str, snapshot: &hal::usb::UsbDeviceSnapshot) {
    kprintln!(
        "{}ctrl={} {:02X}:{:02X}.{} port={} slot={} {:04X}:{:04X} {}",
        indent,
        snapshot.controller_index,
        snapshot.controller_bus,
        snapshot.controller_device,
        snapshot.controller_function,
        snapshot.port,
        snapshot.slot_id.unwrap_or(0),
        snapshot.vendor_id.unwrap_or(0),
        snapshot.product_id.unwrap_or(0),
        snapshot.name
    );
    kprintln!(
        "{}  state: connected={} enabled={} addressed={} configured={} speed={} cat={} driver={} note={}",
        indent,
        snapshot.connected,
        snapshot.enabled,
        snapshot.addressed,
        snapshot.configured,
        usb_speed_name(snapshot.speed),
        usb_category_name(snapshot.category),
        snapshot.driver,
        snapshot.enumeration_note
    );
    kprintln!(
        "{}  class: {:02X}/{:02X}/{:02X} cfgs={} active_cfg={} ifaces={} vis={} attach={} reason={}",
        indent,
        snapshot.class_code,
        snapshot.subclass,
        snapshot.protocol,
        snapshot.configuration_count,
        snapshot
            .active_configuration
            .map(|value| alloc::format!("{}", value))
            .unwrap_or_else(|| String::from("-")),
        snapshot.interface_count,
        usb_visibility_name(snapshot.visibility),
        usb_attach_state_name(snapshot.attach_state),
        snapshot.attach_reason
    );
    if let Some(network) = snapshot.network {
        kprintln!(
            "{}  net: protocol={} mac={} frame={}B",
            indent,
            usb_net_protocol_name(network.protocol),
            net::format_mac(&network.mac),
            network.max_frame_size
        );
    }
    for interface in &snapshot.interfaces {
        kprintln!("{}  {}", indent, usb_interface_summary(interface));
    }
}

fn print_usb_runtime_summary(indent: &str, detailed: bool) {
    let runtime = hal::usb::runtime_snapshot();
    let hide_retired_noise = hal::usb::best_usb_network_ready().is_some();
    let stale_failure_count: usize = runtime
        .controllers_detail
        .iter()
        .map(|controller| controller.stale_failed_ports.iter().flatten().count())
        .sum();
    let latest_failure = runtime
        .controllers_detail
        .iter()
        .find(|controller| controller.last_enumeration_failure_reason != "none");
    kprintln!(
        "{}runtime: controllers={} pending_topology={} cached_devices={}",
        indent,
        runtime.controllers,
        runtime.pending_topology_controllers,
        runtime.cached_devices
    );
    if !hide_retired_noise {
        if let Some(failure) = latest_failure {
            let port = failure
                .progress
                .port
                .or(failure.last_enumeration_failure_port)
                .map(|value| alloc::format!("{}", value))
                .unwrap_or_else(|| String::from("-"));
            kprintln!(
                "{}  last enumeration issue: ctrl={} port={} stage={} reason={} (details: usb diag full)",
                indent,
                failure.controller_index,
                port,
                failure.progress.failure_step,
                failure.last_enumeration_failure_reason
            );
        }
        if stale_failure_count != 0 {
            kprintln!(
                "{}  stale failed port attempt(s): {} retired (details: usb diag full)",
                indent,
                stale_failure_count
            );
        }
    }
    if !detailed {
        return;
    }
    if hide_retired_noise && stale_failure_count != 0 {
        let port = latest_failure
            .map(|entry| {
                entry.progress
                    .port
                    .or(entry.last_enumeration_failure_port)
                    .map(|value| alloc::format!("{}", value))
                    .unwrap_or_else(|| String::from("-"))
            })
            .unwrap_or_else(|| String::from("-"));
        kprintln!(
            "{}  retired USB port failures are hidden while an active USB NIC is healthy (latest ctrl/port={}{}; details: usb diag full)",
            indent,
            port,
            if let Some(failure) = latest_failure {
                alloc::format!(" stage={}", failure.progress.failure_step)
            } else {
                String::new()
            }
        );
    }
    for controller in &runtime.controllers_detail {
        let port = controller
            .last_runtime_event_port
            .map(|value| alloc::format!("{}", value))
            .unwrap_or_else(|| String::from("-"));
        let active_attempt_port = controller
            .active_attempt_port
            .map(|value| alloc::format!("{}", value))
            .unwrap_or_else(|| String::from("-"));
        let enum_failure_port = controller
            .last_enumeration_failure_port
            .map(|value| alloc::format!("{}", value))
            .unwrap_or_else(|| String::from("-"));
        let progress_port = controller
            .progress
            .port
            .map(|value| alloc::format!("{}", value))
            .unwrap_or_else(|| String::from("-"));
        let mut stale_failed_ports = String::from("-");
        for stale in controller.stale_failed_ports.iter().flatten() {
            if stale_failed_ports == "-" {
                stale_failed_ports.clear();
            } else {
                stale_failed_ports.push_str(", ");
            }
            stale_failed_ports.push_str(&alloc::format!(
                "p{}:{}:{}",
                stale.port, stale.stage, stale.reason
            ));
        }
        kprintln!(
            "{}  ctrl={} {:02X}:{:02X}.{} event={} port={} pending={} topo={} cached={} inv_sync={}ms active_port={} progress_port={} stage={} connect={} reset={} reset_done={} reset_src={} warm_reset={} enabled={} ped_seen={} ped_first={}ms ped_last={}ms ped_lost={} slot={} address={} desc={} cfg_try={} configured={} budget_after_reset={} portsc=0x{:08X} link={} chg=0x{:08X} ack=0x{:08X} seen=0x{:08X} acked=0x{:08X} fail_step={} fail_port={} fail_reason={} stale={}",
            indent,
            controller.controller_index,
            controller.controller_bus,
            controller.controller_device,
            controller.controller_function,
            controller.last_runtime_event,
            port,
            controller.pending_topology,
            controller.last_topology_service_result,
            controller.cached_connected_devices,
            controller.last_inventory_sync_ms,
            active_attempt_port,
            progress_port,
            controller.progress.last_stage,
            controller.progress.connect_seen,
            controller.progress.reset_attempted,
            controller.progress.reset_completed,
            controller.progress.reset_completion_source,
            controller.progress.warm_reset_attempted,
            controller.progress.port_enabled,
            controller.progress.ped_observed,
            controller.progress.ped_first_seen_ms,
            controller.progress.ped_last_seen_ms,
            controller.progress.ped_lost_after_observed,
            controller.progress.slot_allocated,
            controller.progress.address_assigned,
            controller.progress.descriptors_fetched,
            controller.progress.configuration_attempted,
            controller.progress.configured,
            controller.progress.budget_exhausted_after_reset,
            controller.progress.last_portsc,
            controller.progress.last_link_state,
            controller.progress.last_change_bits,
            controller.progress.last_ack_bits,
            controller.progress.observed_change_bits,
            controller.progress.observed_ack_bits,
            controller.progress.failure_step,
            enum_failure_port,
            controller.last_enumeration_failure_reason,
            stale_failed_ports
        );
    }
}

fn print_usb_tethering_status(indent: &str) {
    let snapshot = net::usb_tethering_snapshot();
    let runtime = hal::usb::runtime_snapshot();
    let usb_attached_devices = hal::devices()
        .into_iter()
        .filter(|device| matches!(device.info.bus, hal::BusLocation::Usb { .. }))
        .count();
    match snapshot.detected {
        Some(info) => {
            let attach_hint = if snapshot.active_matches_detected {
                "active"
            } else {
                "ready to attach"
            };
            kprintln!(
                "{}USB tethering: {} detected mac={} ctrl={} slot={} frame={}B ({})",
                indent,
                usb_net_protocol_name(info.protocol),
                net::format_mac(&info.mac),
                info.controller_index,
                info.slot_id,
                info.max_frame_size,
                attach_hint
            );
            if let Some(diag) = hal::usb::usb_net_diagnostics(info.controller_index, info.slot_id) {
                kprintln!(
                    "{}  rx_queue={} armed={} tx={} rx={} tx_err={} rx_err={}",
                    indent,
                    diag.rx_queue_depth,
                    diag.rx_armed,
                    diag.tx_frames,
                    diag.rx_frames,
                    diag.tx_errors,
                    diag.rx_errors
                );
            }
            if !snapshot.active_matches_detected {
                kprintln!(
                    "{}  attach: manual runtime promotion only ('net usb attach'); no forced topology rescan",
                    indent
                );
            }
        }
        None => {
            match snapshot.active {
                Some(active) => kprintln!(
                    "{}USB tethering: active {} path went stale mac={} ctrl={} slot={} (last_event={})",
                    indent,
                    usb_net_protocol_name(active.protocol),
                    net::format_mac(&active.mac),
                    active.controller_index,
                    active.slot_id,
                    snapshot.last_event
                ),
                None if snapshot.candidate.is_some() => {
                    let candidate = snapshot.candidate.as_ref().unwrap();
                    kprintln!(
                        "{}USB tethering: {} visible ctrl={} port={} slot={} {:04X}:{:04X} ({})",
                        indent,
                        usb_visibility_name(candidate.visibility),
                        candidate.controller_index,
                        candidate.port,
                        candidate.slot_id.unwrap_or(0),
                        candidate.vendor_id.unwrap_or(0),
                        candidate.product_id.unwrap_or(0),
                        usb_attach_state_name(candidate.attach_state)
                    );
                    kprintln!(
                        "{}  reason: {} (last_event={})",
                        indent,
                        candidate.attach_reason,
                        snapshot.last_event
                    );
                    if !candidate.interfaces.is_empty() {
                        for interface in &candidate.interfaces {
                            kprintln!("{}  {}", indent, usb_interface_summary(interface));
                        }
                    }
                }
                None if runtime.pending_topology_controllers != 0 => kprintln!(
                    "{}USB tethering: port-change seen and deferred enumeration is still pending (last_event={})",
                    indent,
                    snapshot.last_event
                ),
                None if runtime
                    .controllers_detail
                    .iter()
                    .any(|controller| controller.last_enumeration_failure_reason != "none") =>
                {
                    let failure = runtime
                        .controllers_detail
                        .iter()
                        .find(|controller| controller.last_enumeration_failure_reason != "none")
                        .unwrap();
                    kprintln!(
                        "{}USB tethering: enumeration issue on ctrl={} port={} stage={} reason={} (last_event={}; details: usb diag full)",
                        indent,
                        failure.controller_index,
                        failure
                            .progress
                            .port
                            .unwrap_or(failure.last_enumeration_failure_port.unwrap_or(0)),
                        failure.progress.failure_step,
                        failure.last_enumeration_failure_reason,
                        snapshot.last_event
                    );
                }
                None if usb_attached_devices != 0 => kprintln!(
                    "{}USB tethering: USB devices are present, but no CDC-ECM / CDC-NCM / RNDIS network device is visible (last_event={})",
                    indent,
                    snapshot.last_event
                ),
                None => kprintln!(
                    "{}USB tethering: no CDC-ECM / CDC-NCM / RNDIS device visible (last_event={})",
                    indent,
                    snapshot.last_event
                ),
            }
        }
    }
}

/// Execute a built-in shell command.
pub fn execute_command(command_line: &str) {
    let command_line = command_line.trim();

    // $VAR expansion
    let expanded = expand_vars(command_line);
    let command_line = expanded.as_str();

    // Handle pipe: cmd1 | cmd2
    if let Some(pipe_pos) = command_line.find(" | ") {
        let left = &command_line[..pipe_pos];
        let right = &command_line[pipe_pos + 3..];
        execute_piped(left.trim(), right.trim());
        return;
    }

    // Handle redirect: cmd > file, cmd >> file, cmd < file
    if command_line.contains(" > ") || command_line.contains(" >> ") || command_line.contains(" < ")
    {
        execute_redirected(command_line);
        return;
    }

    let parts: Vec<&str> = command_line.split_whitespace().collect();
    let Some(command) = parts.first().copied() else {
        return;
    };

    // Alias lookup
    let alias_expansion = ALIASES.lock().get(command).cloned();
    if let Some(alias_cmd) = alias_expansion {
        let expanded_cmd = if parts.len() > 1 {
            alloc::format!("{} {}", alias_cmd, parts[1..].join(" "))
        } else {
            alias_cmd
        };
        return execute_command(&expanded_cmd);
    }

    match command {
        "help" => cmd_help(parts.get(1).copied()),
        "clear" => {
            let _ = console::clear_screen_for(console::ScreenOwner::Shell, "shell-clear-command");
        }
        "startx" | "gui" => cmd_startx(),
        "cd" => cmd_cd(&parts[1..]),
        "pwd" => cmd_pwd(),
        "mkdir" => cmd_mkdir(&parts[1..]),
        "rmdir" => cmd_rmdir(&parts[1..]),
        "ls" => cmd_ls(&parts[1..]),
        "cat" => cmd_cat(&parts[1..]),
        "write" => cmd_write(command_line),
        "rm" => cmd_rm(&parts[1..]),
        "touch" => cmd_touch(&parts[1..]),
        "stat" => cmd_stat(&parts[1..]),
        "cp" => cmd_cp(&parts[1..]),
        "mv" => cmd_mv(&parts[1..]),
        "find" => cmd_find(&parts[1..]),
        "grep" => cmd_grep(command_line),
        "head" => cmd_head(&parts[1..]),
        "tail" => cmd_tail(&parts[1..]),
        "wc" => cmd_wc(&parts[1..]),
        "diff" => cmd_diff(&parts[1..]),
        "sort" => cmd_sort(&parts[1..]),
        "source" => cmd_source(&parts[1..]),
        "df" => cmd_df(),
        "disk" => cmd_disk(),
        "sync" => cmd_sync(),
        "mount" => cmd_mount(),
        "format" => cmd_format_disk(),
        "info" => cmd_info(),
        "version" => cmd_version(&parts[1..]),
        "cpu" | "cpuinfo" => cmd_cpu(),
        "mem" => cmd_mem(),
        "hwinfo" => cmd_hwinfo(),
        "lsdev" => cmd_lsdev(),
        "lsusb" => cmd_lsusb(),
        "power" | "battery" => cmd_power(command),
        "thermal" => cmd_thermal(),
        "usb" => cmd_usb(&parts[1..]),
        "display" => cmd_display(),
        "time" => cmd_time(),
        "uptime" => cmd_uptime(),
        "date" => cmd_date(),
        "timezone" => cmd_timezone(&parts[1..]),
        "ntp" => cmd_ntp(),
        "rtc" => cmd_rtc(),
        "whoami" => cmd_whoami(),
        "uname" => cmd_uname(),
        "neofetch" => cmd_neofetch(),
        "lspci" | "pciinfo" => cmd_lspci(),
        "echo" => cmd_echo(command_line),
        "color" => cmd_color(),
        "hex" => cmd_hex(&parts[1..]),
        "history" => cmd_history(),
        "tasks" => cmd_tasks(),
        "spawn" => cmd_spawn(command_line),
        "exec" => cmd_exec(&parts[1..]),
        "ps" => cmd_ps(),
        "top" => cmd_top(),
        "jobs" => cmd_jobs(),
        "wait" => cmd_wait(&parts[1..]),
        "nice" => cmd_nice(command_line),
        "kill" => cmd_kill(&parts[1..]),
        "warpkg" => pkg::commands::handle(&parts[1..]),
        "banner" => cmd_banner(),
        "keyboard" => cmd_keyboard(&parts[1..]),
        "language" => cmd_language(&parts[1..]),
        "quantum" => cmd_quantum(),
        "crypto" => cmd_crypto(),
        "ifconfig" | "nicinfo" => cmd_ifconfig(),
        "net" => cmd_net(command_line),
        "wifi" => cmd_wifi(&parts[1..]),
        "ping" => cmd_ping(&parts[1..]),
        "dns" => cmd_dns(&parts[1..]),
        "wget" => cmd_wget(&parts[1..]),
        "curl" => cmd_curl(&parts[1..]),
        "ibm" => cmd_ibm(&parts[1..]),
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
        "env" => cmd_env(),
        "export" => cmd_export(command_line),
        "unset" => cmd_unset(&parts[1..]),
        "alias" => cmd_alias(command_line),
        "unalias" => cmd_unalias(&parts[1..]),
        "panic" => cmd_panic(),
        "reboot" => cmd_reboot(&parts[1..]),
        "halt" => cmd_halt(),
        "waros" => cmd_waros(),
        // WarShield security commands
        "security" => cmd_security(&parts[1..]),
        "capabilities" => cmd_capabilities(&parts[1..]),
        "audit" => cmd_audit(&parts[1..]),
        "firewall" => cmd_firewall(&parts[1..]),
        "integrity" => cmd_integrity(&parts[1..]),
        "encrypt" => cmd_encrypt(&parts[1..]),
        "decrypt" => cmd_decrypt(&parts[1..]),
        "qkd" => cmd_qkd(&parts[1..]),
        "recovery" => cmd_recovery(&parts[1..]),
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
        kprintln!("  cd <dir>       Change directory");
        kprintln!("  pwd            Show current directory");
        kprintln!("  mkdir <dir>    Create directory marker");
        kprintln!("  rmdir <dir>    Remove empty directory");
        kprintln!("  ls             List files");
        kprintln!("  cat <file>     Show file contents");
        kprintln!("  write <f> <t>  Create or overwrite a text file");
        kprintln!("  rm <file>      Delete a file");
        kprintln!("  touch <file>   Create an empty file");
        kprintln!("  stat <file>    Show file metadata");
        kprintln!("  cp <a> <b>     Copy file");
        kprintln!("  mv <a> <b>     Move file");
        kprintln!("  find <pat>     Search filesystem paths");
        kprintln!("  grep <p> <f>   Search text in file");
        kprintln!("  head <f> [n]   First N lines");
        kprintln!("  tail <f> [n]   Last N lines");
        kprintln!("  wc <file>      Count lines/words/bytes");
        kprintln!("  diff <a> <b>   Compare files");
        kprintln!("  sort <file>    Sort file lines");
        kprintln!("  df             Filesystem usage");
        kprintln!("  disk           Show persistent disk status");
        kprintln!("  sync           Force sync RAM files to disk");
        kprintln!("  mount          Show mounted filesystem mode");
        kprintln!("  format         Format the mounted disk");
        return;
    }
    if matches!(topic, Some("security")) {
        kprint_colored!(Colors::CYAN, "WarShield / WarPkg Commands\n");
        kprintln!(
            "  security status                    Show the current WarShield scope and limits"
        );
        kprintln!("  security profile [name]            Show or apply the current profile preset");
        kprintln!("  capabilities                       Show the current process capability set");
        kprintln!("  capabilities drop <CAP> [CAP...]   One-way drop on the current process");
        kprintln!("  audit [log|stats]                  Show current audit-hook output");
        kprintln!("  firewall [status|rules|log]        Show current WarGuard coverage, counters, and rules");
        kprintln!("  warpkg status                      Show offline update / recovery state");
        kprintln!(
            "  warpkg stage|apply <bundle>        Explicit offline signed-bundle update path"
        );
        kprintln!("  warpkg confirm|reject|rollback     Complete or recover a pending update");
        kprintln!(
            "  warpkg proof                       Run the controlled offline update proof path"
        );
        kprintln!("  recovery [status|enter|resume|confirm|reject|rollback]");
        kprintln!("  qkd bb84 [n]                       Run the simulated BB84 demo");
        kprintln!();
        kprintln!("Current limits:");
        kprintln!("  - kernel TLS validates only the current embedded-host set; unsupported HTTPS hosts are rejected");
        kprintln!("  - kernel TLS verifies hostnames and trust anchors, but does not enforce RTC-backed certificate expiry");
        kprintln!(
            "  - WarPkg uses one embedded bootstrap ML-DSA root; no rotation or revocation yet"
        );
        kprintln!("  - offline updates are local bundle stage/apply only; there is no remote control plane or auto-update service");
        kprintln!("  - rollback is currently single-slot preparation backed by explicit file snapshots, not full A/B switching");
        kprintln!("  - WarGuard coverage is still narrow: TCP connect + inbound response, UDP/DNS egress, and ICMP ping/reply");
        kprintln!("  - Server currently shares Standard enforcement; Paranoid also builds the WarVault database");
        kprintln!("  - capability drops are one-way for the current process under the current spawn/exec model");
        return;
    }
    if matches!(topic, Some("recovery")) {
        kprint_colored!(Colors::CYAN, "Recovery Commands\n");
        kprintln!("  recovery status              Show the persisted update / boot health state");
        kprintln!("  recovery enter               Request recovery mode for the next boot");
        kprintln!("  recovery resume              Clear a manual recovery request when safe");
        kprintln!("  recovery confirm             Confirm a shell-ready post-update boot");
        kprintln!("  recovery reject [reason]     Mark the pending update failed and keep recovery active");
        kprintln!("  recovery rollback            Restore the pre-apply filesystem snapshot");
        kprintln!("  reboot recovery              Reboot directly into the recovery shell");
        kprintln!();
        kprintln!("Current model:");
        kprintln!("  - recovery is a narrow administrative shell, not a separate rescue OS");
        kprintln!("  - pending updates require shell-ready plus explicit confirmation");
        kprintln!("  - failed or unconfirmed post-update boots request recovery automatically");
        return;
    }

    print_help_header();
    print_help_section(
        "Platform & Session",
        &[
            ("info / version / uname / neofetch", "platform identity, build, and kernel summary"),
            ("cpu / cpuinfo / mem / hwinfo / lsdev", "hardware inventory and live system state"),
            ("lspci / lsusb / display / power / thermal", "bus, USB, display, and power diagnostics"),
            ("time / date / uptime / timezone / ntp / rtc", "clock, timezone, uptime, and RTC state"),
            ("whoami / users / passwd / su / logout", "session identity and operator control"),
            ("keyboard <layout> / language <code>", "layout and locale selection"),
        ],
    );
    print_help_section(
        "Filesystem & Disk",
        &[
            ("cd / pwd / ls / cat", "navigate and inspect the current filesystem"),
            ("mkdir / rmdir / touch / rm", "create and remove filesystem entries"),
            ("write / stat / cp / mv / chmod", "modify files and metadata"),
            ("find / grep / head / tail / wc / diff / sort", "text search and file inspection"),
            ("source <file>", "execute shell commands from a text file"),
            ("df / disk / sync / mount / format", "capacity, persistence, and disk administration"),
        ],
    );
    print_help_section(
        "Execution & Tools",
        &[
            ("exec / spawn / ps / top / jobs / wait / kill", "task control and experimental process execution"),
            ("nice <pri> <cmd> / tasks", "runtime scheduling and background task insight"),
            ("echo / hex / history / alias / env / export", "shell utilities, history, and environment state"),
            ("unset / unalias / banner / clear", "shell cleanup and operator utilities"),
            ("warpkg <subcmd> / startx | gui", "offline package flow and GUI handoff"),
        ],
    );
    print_help_section(
        "Network & Ports",
        &[
            ("ifconfig / net status / net diag", "active NIC status, DHCP, and low-level diagnostics"),
            ("net dhcp / net retry / net route", "lease acquisition and wired link bring-up"),
            ("net usb status|attach", "USB tethering detection and safe manual attach"),
            ("usb status / usb poll / usb diag full", "USB runtime status and detailed diagnostics"),
            ("ping / dns / wget / curl", "basic network validation and HTTP fetch"),
            ("wifi status / wifi probe", "honest Wi-Fi detection and staged probe-only path"),
        ],
    );
    print_help_section(
        "Security & Recovery",
        &[
            ("security / capabilities / audit / firewall", "WarShield posture, capabilities, and firewall state"),
            ("integrity / encrypt / decrypt / qkd", "integrity checks and crypto workflows"),
            ("recovery <subcmd> / reboot recovery", "update recovery state and rollback controls"),
        ],
    );
    print_help_section(
        "Quantum",
        &[
            ("quantum / crypto / ibm <subcmd>", "quantum runtime overview and backend integration"),
            ("qalloc / qrun / qstate / qmeasure", "allocate, execute, and inspect qubits"),
            ("qprobs / qcircuit / qinfo / qsave / qexport / qresult / qreset / qfree", "circuit introspection, export, and lifecycle"),
        ],
    );
    kprint_colored!(Colors::DIM, "Notes\n");
    kprintln!("  - WarExec remains experimental: static ELF entry, bounded syscall ABI, and no fork/dynamic linking claims.");
    kprintln!("  - USB input targets xHCI plus HID boot-protocol devices; legacy UHCI/OHCI/EHCI remain probe-only.");
    kprintln!("  - USB tethering is detection-first and manual-attach only; Wi-Fi remains staged/probe-only.");
    kprintln!("  - Focused topic help: help quantum | help fs | help security | help recovery.");
}

fn cmd_info() {
    let ticks = runtime_ticks();
    let irq_ticks = interrupts::irq_tick_count();
    let timezone = crate::ui::timezone();
    let cpu = inspect_cpu();
    let memory_stats = memory::stats();
    let memory_summary = memory::boot_memory_summary();
    let used_frames = memory_stats
        .total_frames
        .saturating_sub(memory_stats.free_frames);
    let acpi = hal::acpi::status();
    kprintln!(
        "WarOS v{} - Quantum-Classical Hybrid Operating System",
        KERNEL_VERSION
    );
    kprintln!("Architecture: x86_64");
    kprintln!("Kernel: waros-kernel {}", KERNEL_VERSION);
    kprintln!(
        "CPU: {} | {} | APIC {}{}",
        cpu.brand.as_deref().unwrap_or(cpu.vendor.as_str()),
        cpu_topology_summary(&cpu),
        cpu.topology.apic_id,
        if cpu.hypervisor_present {
            " | hypervisor-present"
        } else {
            ""
        }
    );
    kprintln!(
        "Memory: {} total | {} free | {} used | boot map {} region(s)",
        format_memory_mib(mib_from_frames(memory_stats.total_frames)),
        format_memory_mib(mib_from_frames(memory_stats.free_frames)),
        format_memory_mib(mib_from_frames(used_frames)),
        memory_summary.total_regions
    );
    kprintln!("Boot path: {}", boot_path_summary());
    kprintln!(
        "Timer: {} Hz PIT ({} observed tick(s), {} raw IRQ tick(s))",
        PIT_FREQUENCY_HZ,
        ticks,
        irq_ticks
    );
    kprintln!(
        "Locale: {} | {} | timezone preference {}",
        crate::ui::language().label(),
        crate::ui::layout_label(crate::ui::keyboard_layout()),
        timezone.label()
    );
    kprintln!(
        "Time: manual fixed offset {} ({:+} min); wall clock unavailable (no RTC/NTP/DST)",
        timezone.label(),
        timezone.offset_minutes()
    );
    if let Some(hardware) = net::hardware_status() {
        let net_state = net::network_config()
            .map(|c| c.cidr_string())
            .unwrap_or_else(|| "no DHCP lease".into());
        kprintln!(
            "Network: {} ({}) | {} | Wi-Fi staged",
            hardware.driver,
            net_state,
            network_link_summary(&hardware)
        );
    } else {
        kprintln!(
            "Network: no active NIC | supported: rtl8169-family / Intel e1000 / virtio-net wired | Wi-Fi staged"
        );
    }
    if acpi.available {
        kprintln!(
            "ACPI: FADT={} PM1a=0x{:04X} DSDT=0x{:08X} | thermal/battery AML paths not implemented",
            if acpi.fadt_present {
                "present"
            } else {
                "missing"
            },
            acpi.pm1a_cnt_blk,
            acpi.dsdt_address
        );
    } else {
        kprintln!("ACPI: unavailable");
    }
    if irq_ticks < ticks {
        kprintln!("Timer note: PIT-observed fallback is active beyond raw IRQ accounting.");
    }
    kprintln!("War Enterprise - Building the future of computing");
}

fn cmd_version(args: &[&str]) {
    if matches!(args.first(), Some(&"--all")) {
        let stats = memory::stats();
        let vendor_leaf = __cpuid(0);
        let feature_leaf = __cpuid(1);
        let vendor_bytes = vendor_string_bytes(vendor_leaf.ebx, vendor_leaf.edx, vendor_leaf.ecx);
        let vendor = str::from_utf8(&vendor_bytes).unwrap_or("Unknown");
        let ticks = runtime_ticks();
        let uptime = ticks / u64::from(PIT_FREQUENCY_HZ);
        let hours = uptime / 3600;
        let minutes = (uptime % 3600) / 60;
        let seconds = uptime % 60;
        let irq_ticks = interrupts::irq_tick_count();
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
        if irq_ticks < ticks {
            kprintln!(
                "  Timer:        PIT fallback active (raw IRQ ticks {})",
                irq_ticks
            );
        }
        kprintln!("  Boot time:    {} ms", boot_complete_ms());
        kprintln!("  Files:        {}", file_count);
        kprintln!(
            "  Time:         monotonic uptime only; timezone {} fixed offset; no RTC/NTP/DST",
            crate::ui::timezone().label()
        );
        kprintln!("  Built:        {} (rustc nightly)", BUILD_DATE);
        kprintln!("  Identity:     War Enterprise | Florianopolis, Brazil");
        kprintln!("  Tagline:      Building the future of computing");
        kprintln!("  License:      Apache 2.0");
        kprintln!("  Repository:   github.com/WarEnterprise/waros");
        return;
    }

    kprintln!(
        "WarOS v{} (waros-kernel {})",
        KERNEL_VERSION,
        KERNEL_VERSION
    );
    kprintln!("Built: {} | Rust nightly", BUILD_DATE);
    kprintln!("War Enterprise - Building the future of computing");
    kprintln!("warenterprise.com/waros | github.com/WarEnterprise/waros");
}

fn cmd_cpu() {
    let cpu = inspect_cpu();
    kprintln!("CPU Information:");
    kprintln!("  Vendor:           {}", cpu.vendor);
    if let Some(brand) = cpu.brand.as_deref() {
        kprintln!("  Brand:            {}", brand);
    }
    kprintln!("  Family/Model:     {} / {}", cpu.family, cpu.model);
    kprintln!("  Stepping:         {}", cpu.stepping);
    kprintln!(
        "  CPUID max:        basic 0x{:08X} | extended 0x{:08X}",
        cpu.max_basic_leaf,
        cpu.max_extended_leaf
    );
    kprintln!(
        "  Topology:         {} logical thread(s), {} core(s)/package, {} thread(s)/core",
        cpu.topology.logical_processors,
        cpu.topology.cores_per_package,
        cpu.topology.threads_per_core
    );
    kprintln!(
        "  APIC ID:          {}{}",
        cpu.topology.apic_id,
        if cpu.hypervisor_present {
            " | hypervisor-present"
        } else {
            ""
        }
    );
    if let Some((base_mhz, max_mhz, bus_mhz)) = cpu.frequency_mhz {
        kprintln!(
            "  Frequency:        base {} MHz | max {} MHz | bus {} MHz",
            base_mhz,
            max_mhz,
            bus_mhz
        );
    } else {
        kprintln!("  Frequency:        not reported by CPUID");
    }
    if let Some(cache_summary) = cpu.cache_summary.as_deref() {
        kprintln!("  Cache:            {}", cache_summary);
    } else {
        kprintln!("  Cache:            not enumerated");
    }
    if let Some((physical_bits, virtual_bits)) = cpu.address_bits {
        kprintln!(
            "  Address sizes:    phys {} bit | virt {} bit",
            physical_bits,
            virtual_bits
        );
    }
    kprintln!(
        "  Features:         {}",
        cpu.feature_summary
            .as_deref()
            .unwrap_or("none beyond the minimum inspected set")
    );
}

fn cmd_mem() {
    let stats = memory::stats();
    let summary = memory::boot_memory_summary();
    let used_frames = stats.total_frames.saturating_sub(stats.free_frames);
    let used_percent = percentage(used_frames, stats.total_frames);
    kprintln!("Physical memory:");
    kprintln!(
        "  Total: {} ({} frames)",
        format_memory_mib(mib_from_frames(stats.total_frames)),
        stats.total_frames
    );
    kprintln!(
        "  Used:  {} ({} frames, {}%)",
        format_memory_mib(mib_from_frames(used_frames)),
        used_frames,
        used_percent
    );
    kprintln!(
        "  Free:  {} ({} frames)",
        format_memory_mib(mib_from_frames(stats.free_frames)),
        stats.free_frames
    );
    kprintln!("  Heap:  {} MiB reserved", heap::HEAP_SIZE / (1024 * 1024));
    kprintln!(
        "  Boot map: {} region(s) | usable {} | bootloader {} | other {}",
        summary.total_regions,
        summary.usable_regions,
        summary.bootloader_regions,
        summary.other_regions
    );
    kprintln!(
        "  Usable RAM map: {}",
        format_memory_mib((summary.usable_bytes / (1024 * 1024)) as usize)
    );
    if let Some(offset) = summary.direct_map_offset {
        kprintln!("  Phys map: direct map base 0x{:016X}", offset);
    } else {
        kprintln!("  Phys map: direct map base unavailable");
    }
    kprintln!("  Max phys: 0x{:016X}", summary.max_physical_address);
}

fn cmd_hwinfo() {
    let stats = memory::stats();
    let memory_summary = memory::boot_memory_summary();
    let cpu = inspect_cpu();
    let devices = hal::devices();
    let acpi = hal::acpi::status();
    let display = devices
        .iter()
        .find(|device| device.info.category == hal::DeviceCategory::Display);
    let storage = devices
        .iter()
        .filter(|device| {
            device.info.category == hal::DeviceCategory::Storage
                && device.status == hal::DeviceStatus::Active
        })
        .count();
    let usb = devices
        .iter()
        .filter(|device| device.info.category == hal::DeviceCategory::UsbController)
        .count();
    let usb_active = devices
        .iter()
        .filter(|device| {
            device.info.category == hal::DeviceCategory::UsbController
                && device.status == hal::DeviceStatus::Active
        })
        .count();
    let usb_probe_only = devices
        .iter()
        .filter(|device| {
            device.info.category == hal::DeviceCategory::UsbController
                && device.status != hal::DeviceStatus::Active
        })
        .count();
    let usb_devices = devices
        .iter()
        .filter(|device| matches!(device.info.bus, hal::BusLocation::Usb { .. }))
        .count();
    let input = devices
        .iter()
        .filter(|device| {
            device.info.category == hal::DeviceCategory::Input
                && device.status == hal::DeviceStatus::Active
        })
        .count();
    let usb_input = devices
        .iter()
        .filter(|device| {
            device.info.category == hal::DeviceCategory::Input
                && device.status == hal::DeviceStatus::Active
                && matches!(device.info.bus, hal::BusLocation::Usb { .. })
        })
        .count();
    let ticks = runtime_ticks();
    let irq_ticks = interrupts::irq_tick_count();
    let supported_detected = devices
        .iter()
        .filter(|device| {
            matches!(
                classify_network_inventory(device),
                Some(NetworkInventoryState::SupportedDetected)
            )
        })
        .count();
    let unsupported_wired = devices
        .iter()
        .filter(|device| {
            matches!(
                classify_network_inventory(device),
                Some(NetworkInventoryState::UnsupportedDetected)
            )
        })
        .count();
    let wifi_probe_only = devices
        .iter()
        .filter(|device| {
            matches!(
                classify_network_inventory(device),
                Some(NetworkInventoryState::WifiProbeOnly)
            )
        })
        .count();

    kprintln!("WarOS Hardware Summary:");
    if let Some(brand) = cpu.brand.as_deref() {
        kprintln!("  CPU:      {}", brand);
    } else {
        kprintln!("  CPU:      {}", cpu.vendor);
    }
    kprintln!("  Topology: {}", cpu_topology_summary(&cpu));
    kprintln!(
        "  CPU ID:   family {} model {} stepping {} | APIC {}{}",
        cpu.family,
        cpu.model,
        cpu.stepping,
        cpu.topology.apic_id,
        if cpu.hypervisor_present {
            " | hypervisor-present"
        } else {
            ""
        }
    );
    if let Some((base_mhz, max_mhz, bus_mhz)) = cpu.frequency_mhz {
        kprintln!(
            "  CPU MHz:  base {} | max {} | bus {}",
            base_mhz,
            max_mhz,
            bus_mhz
        );
    } else {
        kprintln!("  CPU MHz:  not reported by CPUID");
    }
    kprintln!(
        "  Cache:    {}",
        cpu.cache_summary.as_deref().unwrap_or("not enumerated")
    );
    kprintln!("  Boot:     {}", boot_path_summary());
    kprintln!(
        "  Memory:   {} total / {} free ({} frames)",
        format_memory_mib(mib_from_frames(stats.total_frames)),
        format_memory_mib(mib_from_frames(stats.free_frames)),
        stats.total_frames
    );
    kprintln!(
        "  Mem map:  {} region(s), usable {} / bootloader {} / other {}",
        memory_summary.total_regions,
        memory_summary.usable_regions,
        memory_summary.bootloader_regions,
        memory_summary.other_regions
    );
    if let Some(device) = display {
        if let hal::DeviceCapabilities::Display(cap) = &device.info.capabilities {
            kprintln!("  Display:  {}x{} @ {}bpp", cap.width, cap.height, cap.bpp);
        } else {
            kprintln!("  Display:  {}", device.info.name);
        }
    } else {
        kprintln!("  Display:  unavailable");
    }
    let detected_wired: Vec<_> = devices
        .iter()
        .filter(|device| {
            device.info.category == hal::DeviceCategory::Network
                && device.status != hal::DeviceStatus::Active
                && matches!(
                    &device.info.capabilities,
                    hal::DeviceCapabilities::Network(cap) if cap.has_ethernet && !cap.has_wifi
                )
        })
        .cloned()
        .collect();
    let detected_wireless: Vec<_> = devices
        .iter()
        .filter(|device| {
            device.info.category == hal::DeviceCategory::Network
                && matches!(
                    &device.info.capabilities,
                    hal::DeviceCapabilities::Network(cap) if cap.has_wifi
                )
        })
        .cloned()
        .collect();
    if let Some(hardware) = net::hardware_status() {
        kprintln!(
            "  Network:  {} | link {}",
            hardware.name,
            network_link_summary(&hardware)
        );
    } else if !detected_wired.is_empty() || !detected_wireless.is_empty() {
        kprintln!(
            "  Network:  no supported active NIC ({} supported inactive, {} unsupported wired, {} Wi-Fi probe-only)",
            supported_detected,
            unsupported_wired,
            wifi_probe_only
        );
    } else {
        kprintln!("  Network:  unavailable");
    }
    kprintln!("  Storage:  {} active device(s)", storage);
    match disk::disk_status() {
        Ok(Some(status)) => kprintln!(
            "  Storage:  active controller at PCI {:02X}:{:02X}.{} | {} MiB | {} block(s) used",
            status.bus,
            status.device,
            status.function,
            status.disk_size / (1024 * 1024),
            status.used_blocks
        ),
        Ok(None) => kprintln!("  Storage:  no active block-device driver (virtio-blk only today)"),
        Err(error) => kprintln!("  Storage:  status read failed: {}", error),
    }
    kprintln!(
        "  USB:      {} controller(s): {} active, {} probe-only; {} device(s)",
        usb,
        usb_active,
        usb_probe_only,
        usb_devices
    );
    let kbd_snap = drivers::keyboard::debug_snapshot();
    kprintln!(
        "  Input:    {} active device(s) ({} platform, {} USB)",
        input,
        input.saturating_sub(usb_input),
        usb_input
    );
    kprintln!(
        "  PS/2:     present={} init_ok={} irq={} polled={} forced_poll={}",
        kbd_snap.i8042_present,
        !kbd_snap.i8042_init_failed,
        kbd_snap.irq_scancodes,
        kbd_snap.polled_scancodes,
        kbd_snap.forced_polling
    );
    kprintln!(
        "  Timer:    {} observed tick(s), {} raw IRQ tick(s)",
        ticks,
        irq_ticks
    );
    kprintln!(
        "  Time:     manual fixed offset {} / wall clock unavailable",
        crate::ui::timezone().label()
    );
    if irq_ticks < ticks {
        kprintln!("  Timer:    PIT fallback is covering missing/late IRQ tick accounting.");
    }
    if usb_probe_only > 0 {
        kprintln!(
            "  USB:      Legacy UHCI/OHCI/EHCI controllers are discovered only; xHCI is the active USB path."
        );
    }
    if !detected_wired.is_empty() {
        print_network_inventory_section(
            "  Detected wired controllers (not active):",
            &detected_wired,
        );
    }
    if !detected_wireless.is_empty() {
        print_network_inventory_section(
            "  Detected wireless-class controllers:",
            &detected_wireless,
        );
    }
    if acpi.available {
        kprintln!(
            "  ACPI:     FADT={} PM1a=0x{:04X} DSDT=0x{:08X} reset=0x{:08X}",
            if acpi.fadt_present { "yes" } else { "no" },
            acpi.pm1a_cnt_blk,
            acpi.dsdt_address,
            acpi.reset_reg_address
        );
    } else {
        kprintln!("  ACPI:     not available");
    }
    kprintln!("  Thermal:  no live temperature path (AML thermal/EC read not implemented)");
    kprintln!("  Battery:  no live battery/AC path (ACPI battery/EC path not implemented)");
    kprintln!(
        "  Quantum:  {} qubits (simulator)",
        crate::quantum::state::MAX_KERNEL_QUBITS
    );
    kprintln!("  Supported wired NICs:");
    kprintln!("    virtio-net: vendor 1AF4, device 1000-103F or 1041");
    kprintln!("    Intel e1000: vendor 8086, device 100E 10D3 153A 15B8 15BD 15BE");
    kprintln!("    Realtek rtl8169-family: vendor 10EC, device 8161 8168 8169");
}

fn cmd_time() {
    let ticks = runtime_ticks();
    let total_seconds = ticks / u64::from(PIT_FREQUENCY_HZ);
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    let timezone = crate::ui::timezone();
    kprintln!(
        "Monotonic runtime: {:02}:{:02}:{:02} since boot ({} ticks)",
        hours,
        minutes,
        seconds,
        ticks
    );
    kprintln!("Wall clock:       unavailable (RTC/NTP not implemented)");
    kprintln!(
        "Timezone:         {} ({}, fixed offset {:+} min)",
        timezone.label(),
        timezone.description(),
        timezone.offset_minutes()
    );
    print_timer_note(ticks);
}

fn cmd_uptime() {
    let ticks = runtime_ticks();
    let seconds = ticks / u64::from(PIT_FREQUENCY_HZ);
    kprintln!("Monotonic uptime: {}s ({} ticks)", seconds, ticks);
    print_timer_note(ticks);
}

fn cmd_ntp() {
    kprint_colored!(Colors::YELLOW, "[STAGED]");
    kprintln!(" NTP client is not implemented.");
    kprintln!(
        "  Current time source: monotonic PIT counter ({} Hz) since boot",
        PIT_FREQUENCY_HZ
    );
    kprintln!("  Wall clock:          unavailable");
    kprintln!("  NTP sync:            not implemented");
    kprintln!("  Prerequisites:       active wired NIC with DHCP lease and UDP connectivity");
    if net::network_config().is_some() {
        kprintln!(
            "  Network:             DHCP lease active (NTP would be reachable once implemented)"
        );
    } else {
        kprintln!("  Network:             no DHCP lease (NTP would require connectivity first)");
    }
    kprintln!("  Note: a future WarOS release targets NTP as the first wall-clock source.");
}

fn cmd_rtc() {
    kprint_colored!(Colors::YELLOW, "[STAGED]");
    kprintln!(" RTC read is not implemented.");
    kprintln!("  x86 CMOS RTC:   not accessed (read path not implemented)");
    kprintln!("  ACPI RTC:       not accessed");
    kprintln!("  Wall clock:     unavailable — all timestamps are boot-relative");
    kprintln!("  File timestamps: stored as PIT tick counts since last boot");
    kprintln!("  Note: a future WarOS release targets CMOS RTC read as a minimal wall-clock seed.");
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
    let _ = console::clear_screen_for(console::ScreenOwner::Shell, "shell-banner-command");
    branding::show_banner();
}

fn cmd_keyboard(args: &[&str]) {
    let Some(layout) = args.first().copied() else {
        let current = crate::ui::keyboard_layout();
        kprintln!("Keyboard layout: {}", crate::ui::layout_label(current));
        kprintln!("Usage: keyboard <layout>");
        kprintln!("  Try: keyboard list");
        return;
    };

    match layout {
        "list" => {
            kprintln!("Available keyboard layouts:");
            for (code, description, available) in hal::input::supported_layouts() {
                let status = if *available { "ready" } else { "planned" };
                kprintln!("  {:<4} {:<20} {}", code, description, status);
            }
            kprintln!("  es   Spanish              planned");
            kprintln!("  ru   Russian              planned");
        }
        "us" => {
            match crate::ui::set_keyboard_layout(crate::hal::device::KeyboardLayout::UsQwerty) {
                Ok(()) => match persist_ui_preferences_if_allowed(
                    security::capabilities::Capabilities::FS_ADMIN,
                ) {
                    Ok(true) => kprintln!("[WarOS] INPUT: keyboard layout set to en-US."),
                    Ok(false) => kprintln!(
                        "[WarOS] INPUT: en-US active for this session only. Persisting the shared keyboard preference requires FS_ADMIN."
                    ),
                    Err(error) => kprintln!(
                        "[WarOS] INPUT: en-US active for this session, but preference save failed: {}.",
                        error
                    ),
                },
                Err(error) => kprintln!("[WarOS] INPUT: failed to select en-US: {}.", error),
            }
        }
        "br" => {
            match crate::ui::set_keyboard_layout(crate::hal::device::KeyboardLayout::BrazilAbnt2)
            {
                Ok(()) => match persist_ui_preferences_if_allowed(
                    security::capabilities::Capabilities::FS_ADMIN,
                ) {
                    Ok(true) => {
                        kprintln!("[WarOS] INPUT: keyboard layout set to pt-BR.");
                    }
                    Ok(false) => kprintln!(
                        "[WarOS] INPUT: pt-BR active for this session only. Persisting the shared keyboard preference requires FS_ADMIN."
                    ),
                    Err(error) => kprintln!(
                        "[WarOS] INPUT: pt-BR active for this session, but preference save failed: {}.",
                        error
                    ),
                },
                Err(error) => kprintln!("[WarOS] INPUT: failed to select pt-BR: {}.", error),
            }
        }
        _ => match hal::input::set_layout_by_name(layout) {
            Ok(selected) => match persist_ui_preferences_if_allowed(
                security::capabilities::Capabilities::FS_ADMIN,
            ) {
                Ok(true) => kprintln!(
                    "[WarOS] INPUT: keyboard layout set to {}.",
                    selected.short_name()
                ),
                Ok(false) => kprintln!(
                    "[WarOS] INPUT: {} active for this session only. Persisting the shared keyboard preference requires FS_ADMIN.",
                    selected.short_name()
                ),
                Err(error) => kprintln!(
                    "[WarOS] INPUT: {} active for this session, but preference save failed: {}.",
                    selected.short_name(),
                    error
                ),
            },
            Err("layout not implemented yet") => {
                kprint_colored!(Colors::YELLOW, "[WarOS] INPUT:");
                kprintln!(
                    " layout '{}' is staged for a future shared scan-code mapper. Use 'keyboard list'.",
                    layout
                );
            }
            Err(_) => {
                kprint_colored!(Colors::RED, "[WarOS] INPUT:");
                kprintln!(" unknown layout '{}'. Use 'keyboard list'.", layout);
            }
        },
    }
}

fn cmd_language(args: &[&str]) {
    match args.first().copied() {
        None => {
            let current = crate::ui::language();
            kprintln!("System language: {} ({})", current.label(), current.code());
            if let Some(note) = crate::ui::localization_status_note() {
                kprintln!("Note: {}", note);
            }
            kprintln!("Usage: language <code>|list");
        }
        Some("list") => {
            kprintln!("Available system languages:");
            for (language, label, ready) in crate::ui::supported_languages() {
                let status = if *ready { "ready" } else { "staged" };
                kprintln!("  {:<4} {:<12} {}", language.code(), label, status);
            }
        }
        Some(code) => match crate::ui::set_language_by_code(code) {
            Ok(language) => {
                match persist_ui_preferences_if_allowed(
                    security::capabilities::Capabilities::FS_ADMIN,
                ) {
                    Ok(true) => kprintln!(
                        "[WarOS] UI: system language set to {} ({}).",
                        language.label(),
                        language.code()
                    ),
                    Ok(false) => kprintln!(
                        "[WarOS] UI: {} ({}) active for this session only. Persisting the shared language preference requires FS_ADMIN.",
                        language.label(),
                        language.code()
                    ),
                    Err(error) => kprintln!(
                        "[WarOS] UI: {} active for this session, but preference save failed: {}.",
                        language.label(),
                        error
                    ),
                }
                if let Some(note) = crate::ui::localization_status_note() {
                    kprintln!("Note: {}", note);
                }
            }
            Err(_) => {
                kprint_colored!(Colors::RED, "[WarOS] UI:");
                kprintln!(" unknown language '{}'. Use 'language list'.", code);
            }
        },
    }
}

fn cmd_timezone(args: &[&str]) {
    match args.first().copied() {
        None => {
            let timezone = crate::ui::timezone();
            kprintln!(
                "Timezone preference: {} ({}, fixed offset {} min)",
                timezone.label(),
                timezone.description(),
                timezone.offset_minutes()
            );
            kprintln!("Note: {}", crate::ui::timezone_status_note());
            kprintln!("Usage: timezone <code>|list");
        }
        Some("list") => {
            kprintln!("Available fixed-offset timezones (manual preference only):");
            for timezone in crate::ui::supported_timezones() {
                kprintln!(
                    "  {:<8} {:<10} {:+4} min  {}",
                    timezone.code(),
                    timezone.label(),
                    timezone.offset_minutes(),
                    timezone.description()
                );
            }
        }
        Some(code) => {
            if security::capabilities::session_require(
                security::capabilities::Capabilities::SYS_TIME,
            )
            .is_err()
            {
                kprint_colored!(Colors::RED, "[WarOS] TIME:");
                kprintln!(" changing the timezone requires SYS_TIME capability.");
                return;
            }
            match crate::ui::set_timezone_by_code(code) {
                Ok(timezone) => {
                    match crate::ui::save_preferences() {
                        Ok(()) => kprintln!(
                            "[WarOS] TIME: timezone preference set to {} (persisted).",
                            timezone.label()
                        ),
                        Err(error) => kprintln!(
                            "[WarOS] TIME: {} applied, but preference save failed: {}.",
                            timezone.label(),
                            error
                        ),
                    }
                    kprintln!("Note: {}", crate::ui::timezone_status_note());
                }
                Err(_) => {
                    kprint_colored!(Colors::RED, "[WarOS] TIME:");
                    kprintln!(" unknown timezone '{}'. Use 'timezone list'.", code);
                }
            }
        }
    }
}

fn cmd_useradd(args: &[&str]) {
    if let Err(_) =
        security::capabilities::session_require(security::capabilities::Capabilities::USER_ADMIN)
    {
        kprint_colored!(Colors::RED, "[WarOS] ");
        kprintln!("Permission denied. Requires USER_ADMIN capability.");
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
        Ok(uid) => match db.try_save_to_fs() {
            Ok(()) => {
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
                let _ = db.delete_user(uid);
                kprint_colored!(Colors::RED, "[WarOS] ");
                kprintln!(
                    "Failed to persist new user '{}': {}. Creation was rolled back.",
                    username,
                    error
                );
            }
        },
        Err(error) => {
            kprint_colored!(Colors::RED, "[WarOS] ");
            kprintln!("Failed to create user: {}.", error);
        }
    }
}

fn cmd_userdel(args: &[&str]) {
    if let Err(_) =
        security::capabilities::session_require(security::capabilities::Capabilities::USER_ADMIN)
    {
        kprint_colored!(Colors::RED, "[WarOS] ");
        kprintln!("Permission denied. Requires USER_ADMIN capability.");
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
        Ok(()) => match db.try_save_to_fs() {
            Ok(()) => {
                kprint_colored!(Colors::GREEN, "[WarOS] ");
                kprintln!("User '{}' deleted.", username);
            }
            Err(error) => {
                db.restore_user_for_rollback(user);
                kprint_colored!(Colors::RED, "[WarOS] ");
                kprintln!(
                    "Failed to persist deletion of '{}': {}. User was restored.",
                    username,
                    error
                );
            }
        },
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
        Ok(()) => match db.try_save_to_fs() {
            Ok(()) => {
                kprint_colored!(Colors::GREEN, "[WarOS] ");
                kprintln!("Password changed for '{}'.", target_name);
            }
            Err(error) => {
                if let Some(rollback) = db.find_mut_by_uid(target.uid) {
                    rollback.password_hash = target.password_hash;
                    rollback.salt = target.salt;
                }
                kprint_colored!(Colors::RED, "[WarOS] ");
                kprintln!(
                    "Failed to persist password change for '{}': {}. Password was left unchanged.",
                    target_name,
                    error
                );
            }
        },
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
            if let Err(error) = db.try_save_to_fs() {
                serial_println!(
                    "[WARN] auth: failed to persist login metadata for {} after su: {}",
                    user.username,
                    error
                );
            }
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
    let uid = auth::session::current_uid();
    let username = auth::session::current_username();
    security::audit::log_event(security::audit::events::AuditEvent::Logout { username, uid });
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

fn cmd_cd(args: &[&str]) {
    let target = args.first().copied().unwrap_or("~");
    match fs::change_directory(target) {
        Ok(path) => kprintln!("{}", fs::display_path(&path)),
        Err(error) => report_fs_error(target, error),
    }
}

fn cmd_pwd() {
    kprintln!("{}", fs::display_path(&auth::session::current_cwd()));
}

fn cmd_mkdir(args: &[&str]) {
    let Some(path) = args.first().copied() else {
        kprintln!("Usage: mkdir <dir>");
        return;
    };
    match fs::mkdir_current(path) {
        Ok(path) => {
            kprint_colored!(Colors::GREEN, "Created ");
            kprintln!("directory '{}'.", fs::display_path(&path));
        }
        Err(error) => report_fs_error(path, error),
    }
}

fn cmd_rmdir(args: &[&str]) {
    let Some(path) = args.first().copied() else {
        kprintln!("Usage: rmdir <dir>");
        return;
    };
    match fs::rmdir_current(path) {
        Ok(path) => {
            kprint_colored!(Colors::GREEN, "Removed ");
            kprintln!("directory '{}'.", fs::display_path(&path));
        }
        Err(error) => report_fs_error(path, error),
    }
}

fn cmd_ls(args: &[&str]) {
    let target = args.first().copied();
    match fs::list_entries_current(target) {
        Ok((directory, entries)) => {
            if entries.is_empty() {
                kprintln!("No entries in {}.", fs::display_path(&directory));
                return;
            }

            kprintln!("  OWNER      MODE  SIZE     MODIFIED   NAME");
            for entry in entries {
                kprintln!(
                    "  {:<10} {:<4} {:>6} {}  {:<8} {}{}",
                    auth::username_for_uid(entry.owner_uid),
                    entry.permissions.mode_string(),
                    entry.size,
                    if entry.is_dir { "D" } else { "B" },
                    fs::format_timestamp(entry.modified_at),
                    entry.name,
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
            kprint_colored!(
                Colors::GREEN,
                "{}",
                if existed { "Updated " } else { "Created " }
            );
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

fn cmd_cp(args: &[&str]) {
    let Some(source) = args.first().copied() else {
        kprintln!("Usage: cp <source> <destination>");
        return;
    };
    let Some(destination) = args.get(1).copied() else {
        kprintln!("Usage: cp <source> <destination>");
        return;
    };
    match fs::copy_current(source, destination) {
        Ok(path) => {
            kprint_colored!(Colors::GREEN, "Copied ");
            kprintln!("to '{}'.", fs::display_path(&path));
        }
        Err(error) => report_fs_error(source, error),
    }
}

fn cmd_mv(args: &[&str]) {
    let Some(source) = args.first().copied() else {
        kprintln!("Usage: mv <source> <destination>");
        return;
    };
    let Some(destination) = args.get(1).copied() else {
        kprintln!("Usage: mv <source> <destination>");
        return;
    };
    match fs::move_current(source, destination) {
        Ok(path) => {
            kprint_colored!(Colors::GREEN, "Moved ");
            kprintln!("to '{}'.", fs::display_path(&path));
        }
        Err(error) => report_fs_error(source, error),
    }
}

fn cmd_find(args: &[&str]) {
    let Some(pattern) = args.first().copied() else {
        kprintln!("Usage: find <pattern>");
        return;
    };
    match fs::find_current(pattern) {
        Ok(matches) if matches.is_empty() => kprintln!("No matches for '{}'.", pattern),
        Ok(matches) => {
            for path in matches {
                kprintln!("{}", fs::display_path(&path));
            }
        }
        Err(error) => report_fs_error(pattern, error),
    }
}

fn cmd_grep(command_line: &str) {
    let mut parts = command_line.splitn(3, char::is_whitespace);
    let _ = parts.next();
    let Some(pattern) = parts.next() else {
        kprintln!("Usage: grep <pattern> <file>");
        return;
    };
    let Some(path) = parts.next() else {
        kprintln!("Usage: grep <pattern> <file>");
        return;
    };

    match fs::grep_current(pattern, path) {
        Ok(matches) if matches.is_empty() => kprintln!("No matches."),
        Ok(matches) => {
            for hit in matches {
                kprintln!("{:>4}: {}", hit.line_number, hit.line);
            }
        }
        Err(error) => report_fs_error(path, error),
    }
}

fn cmd_head(args: &[&str]) {
    let Some(path) = args.first().copied() else {
        kprintln!("Usage: head <file> [n]");
        return;
    };
    let lines = args
        .get(1)
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(10);
    match fs::head_current(path, lines) {
        Ok(lines) => {
            for line in lines {
                kprintln!("{}", line);
            }
        }
        Err(error) => report_fs_error(path, error),
    }
}

fn cmd_tail(args: &[&str]) {
    let Some(path) = args.first().copied() else {
        kprintln!("Usage: tail <file> [n]");
        return;
    };
    let lines = args
        .get(1)
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(10);
    match fs::tail_current(path, lines) {
        Ok(lines) => {
            for line in lines {
                kprintln!("{}", line);
            }
        }
        Err(error) => report_fs_error(path, error),
    }
}

fn cmd_wc(args: &[&str]) {
    let Some(path) = args.first().copied() else {
        kprintln!("Usage: wc <file>");
        return;
    };
    match fs::wc_current(path) {
        Ok(counts) => {
            kprintln!(
                "{} {} {} {}",
                counts.lines,
                counts.words,
                counts.bytes,
                path
            );
        }
        Err(error) => report_fs_error(path, error),
    }
}

fn cmd_diff(args: &[&str]) {
    let Some(left) = args.first().copied() else {
        kprintln!("Usage: diff <file-a> <file-b>");
        return;
    };
    let Some(right) = args.get(1).copied() else {
        kprintln!("Usage: diff <file-a> <file-b>");
        return;
    };
    match fs::diff_current(left, right) {
        Ok(lines) if lines.is_empty() => kprintln!("Files are identical."),
        Ok(lines) => {
            for line in lines {
                kprintln!("{}", line);
            }
        }
        Err(error) => report_fs_error(left, error),
    }
}

fn cmd_sort(args: &[&str]) {
    let Some(path) = args.first().copied() else {
        kprintln!("Usage: sort <file>");
        return;
    };
    match fs::sort_current(path) {
        Ok(lines) => {
            for line in lines {
                kprintln!("{}", line);
            }
        }
        Err(error) => report_fs_error(path, error),
    }
}

fn cmd_source(args: &[&str]) {
    let Some(path) = args.first().copied() else {
        kprintln!("Usage: source <file>");
        return;
    };
    let (_, data) = match fs::read_current(path) {
        Ok(result) => result,
        Err(error) => {
            report_fs_error(path, error);
            return;
        }
    };
    let Ok(text) = str::from_utf8(&data) else {
        kprint_colored!(Colors::RED, "[ERR]");
        kprintln!(" '{}' is not valid UTF-8 text.", path);
        return;
    };
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        execute_command(trimmed);
    }
}

fn cmd_df() {
    let filesystem = fs::FILESYSTEM.lock();
    kprintln!("WarFS usage:");
    kprintln!("  Files: {} / {}", filesystem.list().len(), fs::MAX_FILES);
    kprintln!("  Used:  {} KiB", filesystem.used_space() / 1024);
    kprintln!("  Free:  {} KiB", filesystem.free_space() / 1024);
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
    if let Err(_) =
        security::capabilities::session_require(security::capabilities::Capabilities::FS_ADMIN)
    {
        kprint_colored!(Colors::RED, "[WarOS] ");
        kprintln!("Permission denied. Requires FS_ADMIN capability.");
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

    match exec::spawn_shell_command(remainder.trim(), exec::process::Priority::Batch) {
        Ok(pid) => {
            kprint_colored!(Colors::GREEN, "[PID {}]", pid);
            kprintln!(" Started: {}", remainder.trim());
        }
        Err(error) => {
            kprint_colored!(Colors::RED, "[ERR]");
            kprintln!(" {:?}", error);
        }
    }
}

fn cmd_kill(args: &[&str]) {
    let (signal, pid_arg) = if matches!(args.first(), Some(&"-9")) {
        (9, args.get(1).copied())
    } else {
        (15, args.first().copied())
    };
    let Some(id) = pid_arg.and_then(|value| value.parse::<u64>().ok()) else {
        kprintln!("Usage: kill [-9] <pid>");
        return;
    };

    if let Ok(()) = exec::kill_process(id as u32, -signal) {
        kprint_colored!(Colors::GREEN, "Killed ");
        kprintln!("process {}.", id);
        return;
    }

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

fn cmd_exec(args: &[&str]) {
    let Some(path) = args.first().copied() else {
        kprintln!("Usage: exec <path> [args]");
        kprintln!("  Supports the current minimal WarExec ABI only: static ELF entry, stack-based argc/argv,");
        kprintln!("  stdout/stderr write, absolute-path open/read/close, stat/fstat, readdir,");
        kprintln!("  create-new write with close-time commit, monotonic heap growth through brk(0)/brk(new_end),");
        kprintln!("  deterministic wait4 exited-child observation, and a minimal in-place exec replacement path.");
        kprintln!("  The current ABI also applies stack ASLR, loader W^X checks, and bounded user-pointer/string validation with deterministic");
        kprintln!("  negative error returns on bad addresses.");
        return;
    };

    let (resolved, data) = match fs::read_current(path) {
        Ok(result) => result,
        Err(error) => {
            report_fs_error(path, error);
            return;
        }
    };

    if exec::elf::parse_elf(&data).is_ok() {
        let mut process_args = Vec::with_capacity(args.len());
        process_args.push(resolved.as_str());
        process_args.extend_from_slice(&args[1..]);
        let env = Vec::<(String, String)>::new();
        match exec::loader::spawn_process(
            &resolved,
            &process_args,
            &env,
            auth::session::current_uid(),
            exec::ensure_shell_process(),
            exec::process::Priority::Normal,
        ) {
            Ok(pid) => match exec::run_user_process(pid) {
                Ok(exit_code) => {
                    kprint_colored!(Colors::GREEN, "[WarExec] ");
                    kprintln!(
                        "'{}' exited with code {}.",
                        fs::display_path(&resolved),
                        exit_code
                    );
                }
                Err(error) => {
                    kprint_colored!(Colors::RED, "[WarExec] ");
                    kprintln!(
                        "failed to run '{}' as PID {}: {:?}.",
                        fs::display_path(&resolved),
                        pid,
                        error
                    );
                }
            },
            Err(error) => {
                kprint_colored!(Colors::RED, "[WarExec] ");
                kprintln!("failed to load ELF: {:?}.", error);
            }
        }
        return;
    }

    let Ok(text) = str::from_utf8(&data) else {
        kprint_colored!(Colors::RED, "[WarExec] ");
        kprintln!(
            "'{}' is neither ELF nor UTF-8 script.",
            fs::display_path(&resolved)
        );
        return;
    };

    let command = if text.starts_with("#!warsh") {
        alloc::format!("source {}", fs::display_path(&resolved))
    } else {
        text.lines().next().unwrap_or("").trim().to_string()
    };
    if command.is_empty() {
        kprint_colored!(Colors::RED, "[WarExec] ");
        kprintln!("'{}' is empty.", fs::display_path(&resolved));
        return;
    }

    match exec::spawn_shell_command(&command, exec::process::Priority::Normal) {
        Ok(pid) => {
            kprint_colored!(Colors::GREEN, "[WarExec] ");
            kprintln!(
                "spawned PID {} from '{}'.",
                pid,
                fs::display_path(&resolved)
            );
        }
        Err(error) => {
            kprint_colored!(Colors::RED, "[WarExec] ");
            kprintln!(
                "failed to execute '{}': {:?}.",
                fs::display_path(&resolved),
                error
            );
        }
    }
}

fn cmd_ps() {
    let processes = exec::snapshot();
    if processes.is_empty() {
        kprintln!("No processes registered.");
        return;
    }
    kprintln!(" PID  PPID USER       PRI    STATE    MEM    QUBITS IMAGE    NAME");
    for process in processes {
        kprintln!(
            " {:>3}  {:>4} {:<10} {:<6} {:<8} {:>4}p {:>6} {:<8} {}",
            process.pid,
            process.parent_pid,
            auth::username_for_uid(process.uid),
            priority_name(process.priority),
            process_state_name(process.state),
            process.memory_pages,
            process.qubits,
            image_kind_name(process.image_kind),
            process.name
        );
    }
}

fn cmd_top() {
    let processes = exec::snapshot();
    let stats = memory::stats();
    kprintln!("WarOS Process Monitor - WarSched bootstrap");
    kprintln!(
        "Uptime: {} | Processes: {} | Memory: {}/{} MiB",
        fs::format_timestamp(interrupts::tick_count()),
        processes.len(),
        ((stats.total_frames - stats.free_frames) * 4) / 1024,
        (stats.total_frames * 4) / 1024
    );
    kprintln!(
        "Context switches: {} | Quantum ops: {}",
        exec::context_switch_count(),
        quantum::active_register()
            .map(|(qubits, _)| qubits)
            .unwrap_or(0)
    );
    kprintln!();
    cmd_ps();
}

fn cmd_jobs() {
    let shell_pid = exec::ensure_shell_process();
    let jobs: Vec<_> = exec::snapshot()
        .into_iter()
        .filter(|process| {
            process.pid != shell_pid && process.state != exec::process::ProcessState::Zombie
        })
        .collect();
    if jobs.is_empty() {
        kprintln!("No background jobs.");
        return;
    }
    kprintln!("Jobs:");
    for process in jobs {
        kprintln!(
            "  [{:>3}] {:<8} {}",
            process.pid,
            process_state_name(process.state),
            process.name
        );
    }
}

fn cmd_wait(args: &[&str]) {
    let Some(pid) = args.first().and_then(|value| value.parse::<u32>().ok()) else {
        kprintln!("Usage: wait <pid>");
        return;
    };
    loop {
        if let Some(process) = exec::snapshot()
            .into_iter()
            .find(|process| process.pid == pid)
        {
            if process.state == exec::process::ProcessState::Zombie {
                kprintln!("Process {} exited with {:?}.", pid, process.exit_code);
                return;
            }
        } else {
            kprintln!("Process {} not found.", pid);
            return;
        }
        let _ = net::poll();
        let _ = wait_for_runtime_tick();
    }
}

fn cmd_nice(command_line: &str) {
    let mut parts = command_line.splitn(3, char::is_whitespace);
    let _ = parts.next();
    let Some(priority_text) = parts.next() else {
        kprintln!("Usage: nice <priority> <command>");
        return;
    };
    let Some(command) = parts.next() else {
        kprintln!("Usage: nice <priority> <command>");
        return;
    };
    let Some(priority) = parse_priority(priority_text) else {
        kprintln!("Priority must be one of: rt sys quant int norm batch idle");
        return;
    };
    match exec::spawn_shell_command(command, priority) {
        Ok(pid) => {
            kprint_colored!(Colors::GREEN, "[PID {}]", pid);
            kprintln!(" Started with {} priority.", priority_name(priority));
        }
        Err(error) => {
            kprint_colored!(Colors::RED, "[ERR]");
            kprintln!(" {:?}", error);
        }
    }
}

fn cmd_date() {
    let ticks = runtime_ticks();
    let timezone = crate::ui::timezone();
    kprintln!("Civil date/time: unavailable (RTC/NTP not implemented).");
    kprintln!(
        "Timezone preference: {} ({}, fixed offset {} min)",
        timezone.label(),
        timezone.description(),
        timezone.offset_minutes()
    );
    kprintln!("Monotonic:  {} since boot", fs::format_timestamp(ticks));
    kprintln!(
        "Build date: {} (kernel compile time — not current date)",
        BUILD_DATE
    );
    kprintln!("RTC:        not read (use 'rtc' for status)");
    kprintln!("NTP:        not synced (use 'ntp' for status)");
    kprintln!("Note:       {}", crate::ui::timezone_status_note());
    print_timer_note(ticks);
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
    let uptime = runtime_ticks() / u64::from(PIT_FREQUENCY_HZ);
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
    let timezone = crate::ui::timezone();

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
    kprintln!("      Time:    monotonic only | TZ {}", timezone.label());
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
        let hal_match = hal_device_for_pci(&device);
        kprintln!(
            "  {:02X}:{:02X}.{}  {:04X}:{:04X}  class={:02X}:{:02X} rev={:02X} prog_if={:02X} irq={}/{}",
            device.bus,
            device.device,
            device.function,
            device.vendor_id,
            device.device_id,
            device.class_code,
            device.subclass,
            device.revision_id,
            device.prog_if,
            device.interrupt_pin,
            device.interrupt_line
        );
        kprintln!(
            "                 {} [{}] | WarOS driver={} status={}",
            net::pci::class_name(device.class_code, device.subclass),
            pci_vendor_name(device.vendor_id),
            hal_match
                .as_ref()
                .map(|entry| entry.driver.name())
                .unwrap_or("none"),
            hal_match
                .as_ref()
                .map(|entry| entry.status.short_name())
                .unwrap_or("disc")
        );
        kprintln!("                 BARs: {}", format_pci_bars(&device));
        if let Some(hal_device) = hal_match.as_ref() {
            kprintln!(
                "                 HAL: {} | {}",
                format_bus_and_ids(hal_device),
                summarize_device_capabilities(hal_device)
            );
        }
    }
}

fn cmd_lsdev() {
    let devices = hal::devices();
    kprintln!("WarOS Hardware Devices (WarHAL):");
    if devices.is_empty() {
        kprintln!("  No devices registered.");
        return;
    }
    kprintln!("  ID  CATEGORY    STATUS    DRIVER          BUS/IDS                    NAME");
    for device in devices {
        kprintln!(
            "  {:<3} {:<11} {:<9} {:<15} {:<26} {}",
            device.id.0,
            device.info.category.short_name(),
            device.status.short_name(),
            device.driver.name(),
            format_bus_and_ids(&device),
            device.info.name
        );
        kprintln!("      caps: {}", summarize_device_capabilities(&device));
    }
}

fn cmd_lsusb() {
    let devices = hal::devices();
    let snapshots = hal::usb::device_snapshots();
    let controllers: alloc::vec::Vec<_> = devices
        .iter()
        .filter(|device| device.info.category == hal::DeviceCategory::UsbController)
        .collect();

    kprintln!("USB Controllers:");
    if controllers.is_empty() {
        kprintln!("  No USB controllers registered.");
    } else {
        for device in &controllers {
            let speed = match &device.info.capabilities {
                hal::DeviceCapabilities::Usb(cap) => usb_speed_name(cap.speed),
                _ => "unknown",
            };
            kprintln!(
                "  {:<12} {:<8} {:<15} {:<6} {}",
                format_bus_location(&device.info.bus),
                device.status.short_name(),
                device.info.name,
                device.driver.name(),
                speed
            );
        }
    }

    kprintln!("USB Devices:");
    if snapshots.is_empty() {
        kprintln!("  No USB-attached devices registered.");
    } else {
        for snapshot in &snapshots {
            print_usb_snapshot_detail("  ", snapshot);
        }
    }

    if controllers
        .iter()
        .any(|device| device.status != hal::DeviceStatus::Active)
    {
        kprint_colored!(Colors::DIM, "  note: ");
        kprintln!("WarOS currently drives xHCI. Legacy UHCI/OHCI/EHCI controllers are probe-only.");
    }
    if !snapshots
        .iter()
        .any(|snapshot| snapshot.visibility == hal::usb::UsbPortVisibility::Input)
    {
        kprint_colored!(Colors::DIM, "  note: ");
        kprintln!(
            "USB input currently depends on an active xHCI path plus boot-protocol HID keyboard/mouse support."
        );
    }
    kprint_colored!(Colors::DIM, "  note: ");
    kprintln!(
        "USB Ethernet/tethering is detection-first. Use 'net usb status' to inspect and 'net usb attach' to promote it."
    );
    kprint_colored!(Colors::DIM, "  note: ");
    kprintln!("lsusb is snapshot-only in shell context; live topology refresh stays deferred to runtime service.");
    print_usb_runtime_summary("  ", false);
    print_usb_tethering_status("  ");
}

fn cmd_power(invoked_as: &str) {
    let acpi = hal::acpi::status();
    if invoked_as == "battery" {
        kprintln!("Battery status:");
    } else {
        kprintln!("Power status:");
    }
    kprintln!(
        "  ACPI:               {}",
        if acpi.available {
            "available"
        } else {
            "not available"
        }
    );
    if acpi.available {
        kprintln!(
            "  FADT / PM1a:        {} / 0x{:04X}",
            if acpi.fadt_present {
                "present"
            } else {
                "missing"
            },
            acpi.pm1a_cnt_blk
        );
        kprintln!(
            "  Reset register:     0x{:08X} value 0x{:02X}",
            acpi.reset_reg_address,
            acpi.reset_value
        );
        kprintln!(
            "  S5 sleep type:      {}",
            if acpi.slp_typ_s5 != 0 {
                alloc::format!("0x{:X}", acpi.slp_typ_s5)
            } else {
                String::from("not decoded")
            }
        );
    }
    kprintln!("  AC adapter state:   not implemented (no ACPI _PSR / EC path)");
    kprintln!("  Battery telemetry:  not implemented (no ACPI battery / EC path)");
    kprintln!("  Charge level:       unavailable");
    kprintln!("  Fan telemetry:      unavailable");
}

fn cmd_thermal() {
    let acpi = hal::acpi::status();
    kprintln!("Thermal status:");
    kprintln!(
        "  ACPI tables:        {}",
        if acpi.available {
            "available"
        } else {
            "not available"
        }
    );
    if acpi.available {
        kprintln!("  DSDT address:       0x{:08X}", acpi.dsdt_address);
    }
    kprintln!("  Live temperature:   not implemented");
    kprintln!("  Thermal zones:      not evaluated");
    kprintln!("  Fan state:          not implemented");
    kprintln!("  Blocker:            WarOS parses ACPI tables but does not yet execute AML methods such as _TMP/_TZ or EC-backed thermal paths.");
}

fn cmd_display() {
    let devices = hal::devices();
    let Some(device) = devices
        .into_iter()
        .find(|device| device.info.category == hal::DeviceCategory::Display)
    else {
        kprintln!("No display device registered.");
        return;
    };

    kprintln!("Display:");
    kprintln!("  Name:    {}", device.info.name);
    kprintln!("  Status:  {}", device.status.short_name());
    kprintln!("  Driver:  {}", device.driver.name());
    if let hal::DeviceCapabilities::Display(cap) = device.info.capabilities {
        kprintln!("  Size:    {}x{}", cap.width, cap.height);
        kprintln!("  Format:  {} bpp {:?}", cap.bpp, cap.pixel_format);
    }
}

fn cmd_usb(args: &[&str]) {
    match args.first().copied() {
        None | Some("status") => {
            let devices = hal::devices();
            let controllers = devices
                .iter()
                .filter(|device| device.info.category == hal::DeviceCategory::UsbController)
                .count();
            let active_controllers = devices
                .iter()
                .filter(|device| {
                    device.info.category == hal::DeviceCategory::UsbController
                        && device.status == hal::DeviceStatus::Active
                })
                .count();
            let usb_devices = devices
                .iter()
                .filter(|device| matches!(device.info.bus, hal::BusLocation::Usb { .. }))
                .count();
            let usb_input = devices
                .iter()
                .filter(|device| {
                    device.info.category == hal::DeviceCategory::Input
                        && device.status == hal::DeviceStatus::Active
                        && matches!(device.info.bus, hal::BusLocation::Usb { .. })
                })
                .count();
            let (hid_keyboards, hid_keyboards_armed) = hal::usb::hid_keyboard_count();
            kprintln!("USB runtime:");
            kprintln!(
                "  Controllers tracked: {} ({} active, {} probe-only)",
                controllers,
                active_controllers,
                controllers.saturating_sub(active_controllers)
            );
            kprintln!("  Enumerated USB devices: {}", usb_devices);
            kprintln!("  Active USB input devices: {}", usb_input);
            if controllers > active_controllers {
                kprintln!(
                    "  Note: legacy UHCI/OHCI/EHCI controllers are discovered only; xHCI is the active USB path."
                );
            }
            if usb_input == 0 {
                kprintln!(
                    "  Note: USB input currently depends on xHCI plus HID boot-protocol keyboard/mouse support."
                );
            }
            let kbd = drivers::keyboard::debug_snapshot();
            kprintln!("  Input pipeline:");
            kprintln!(
                "    PS/2: present={} init_ok={} irq_count={} polled_count={} forced_poll={}",
                kbd.i8042_present,
                !kbd.i8042_init_failed,
                kbd.irq_scancodes,
                kbd.polled_scancodes,
                kbd.forced_polling
            );
            kprintln!("    USB HID: {} active device(s)", usb_input);
            kprintln!(
                "    HID keyboards: {} tracked / {} armed",
                hid_keyboards,
                hid_keyboards_armed
            );
            kprintln!("  Snapshot mode: shell status does not force live topology rescans.");
            print_usb_runtime_summary("  ", false);
            print_usb_tethering_status("  ");
        }
        Some("reset") => {
            if !require_session_capability(
                security::capabilities::Capabilities::HW_ACCESS,
                "HW_ACCESS",
                "usb reset",
            ) {
                return;
            }
            kprintln!("USB controller reprobe requested (runtime path only; legacy controllers remain probe-only)...");
            let controllers = hal::usb::probe_controllers();
            kprintln!(
                "USB reprobe complete: {} controller(s) tracked.",
                controllers
            );
        }
        Some("poll") => {
            if !require_session_capability(
                security::capabilities::Capabilities::HW_ACCESS,
                "HW_ACCESS",
                "usb poll",
            ) {
                return;
            }
            let report = hal::usb::runtime_catch_up();
            kprintln!(
                "USB runtime catch-up: controllers={} pending={} -> {} rescanned={} cached_devices={}",
                report.controllers,
                report.pending_topology_before,
                report.pending_topology_after,
                report.rescanned_controllers,
                report.cached_devices
            );
            print_usb_runtime_summary("  ", false);
        }
        Some("diag") if matches!(args.get(1).copied(), Some("full")) => {
            if !require_session_capability(
                security::capabilities::Capabilities::HW_ACCESS,
                "HW_ACCESS",
                "usb diag full",
            ) {
                return;
            }
            for line in hal::usb::full_diagnostics() {
                kprintln!("{}", line);
            }
        }
        _ => {
            kprintln!("Usage: usb [status|reset|poll|diag full]");
        }
    }
}

fn cmd_net(command_line: &str) {
    let mut parts = command_line.splitn(3, char::is_whitespace);
    let _ = parts.next();
    let Some(subcommand) = parts.next() else {
        kprintln!("Usage: net <status|diag [ipv4]|poll|dhcp|retry|usb|route|arp|dns-cache|txprobe|send|qsend|listen>");
        kprintln!("  usb:   'net usb status' for detection, 'net usb attach' for manual attach.");
        kprintln!(
            "  retry [ms]: re-bring-up the active wired NIC (PHY wake + autoneg + forced 100M fallback)."
        );
        return;
    };

    match subcommand {
        "status" => {
            kprintln!("Network stack: {}", net::status());
            kprintln!("PCI inventory: {} device(s)", net::pci_devices().len());
            kprintln!("Serial transport: legacy COM2 link for send/qsend/listen");
            kprintln!(
                "  IP path:    supported on rtl8169-family, Intel e1000, virtio-net, USB NIC (RNDIS/ECM)"
            );
            kprintln!("  HTTPS:      {}", net::tls::trust_policy_summary());
            if let Some(hardware) = net::hardware_status() {
                kprintln!("  Interface: {}", hardware.name);
                kprintln!("  Driver:    {}", hardware.driver);
                kprintln!("  MAC:       {}", net::format_mac(&hardware.mac));
                match hardware.transport {
                    net::NetworkTransport::Io(io_base) => {
                        kprintln!("  Transport: I/O 0x{:04X}", io_base);
                    }
                    net::NetworkTransport::Mmio(mmio_base) => {
                        kprintln!("  Transport: MMIO 0x{:08X}", mmio_base);
                    }
                }
                kprintln!("  RX queue:  {}", hardware.rx_queue_size);
                kprintln!("  TX queue:  {}", hardware.tx_queue_size);
                kprintln!("  IRQ line:  {}", hardware.interrupt_line);
                kprintln!("  Link:      {}", network_link_summary(&hardware));
                kprintln!("  Pending:   {}", hardware.pending_frames);
                kprintln!("  RX/TX:     {}/{}", hardware.rx_frames, hardware.tx_frames);
            } else {
                kprintln!("  Interface: no supported active NIC");
            }
            let config = net::network_config();
            let maintenance = net::maintenance_report();
            if let Some(config) = config {
                kprintln!("  DHCP:      lease acquired");
                kprintln!("  IPv4:      {}", config.cidr_string());
                if let Some(gateway) = config.gateway {
                    kprintln!("  Gateway:   {}", gateway);
                }
                if let Some(dns_server) = config.dns_server {
                    kprintln!("  DNS:       {}", dns_server);
                }
            } else {
                kprintln!(
                    "  DHCP:      {}",
                    ipv4_lease_status_line(&maintenance)
                );
            }
            kprintln!(
                "  Lease mgmt: idle poll service active (last_poll={}ms at +{}ms, dhcp_event={}ms at +{}ms, socket={}, lease={})",
                maintenance.ms_since_poll,
                maintenance.last_poll_ms,
                maintenance.ms_since_dhcp_event,
                maintenance.last_dhcp_event_ms,
                if maintenance.dhcp_socket_ready { "ready" } else { "missing" },
                maintenance.canonical_lease_state
            );
            kprintln!(
                "  DHCP owner: owner={} gen={} trigger={} elapsed={}ms remaining={}ms",
                maintenance.dhcp_owner,
                maintenance.dhcp_generation,
                maintenance.dhcp_owner_trigger,
                maintenance.dhcp_owner_elapsed_ms,
                maintenance.dhcp_owner_remaining_ms
            );
            kprintln!(
                "  Auto DHCP:  active={} trigger={} state={} wait={} polls={} elapsed={}ms remaining={}ms",
                if maintenance.dhcp_auto_pump_active {
                    "yes"
                } else {
                    "no"
                },
                maintenance.dhcp_auto_pump_trigger,
                maintenance.dhcp_auto_pump_state,
                dhcp_wait_label(maintenance.dhcp_auto_pump_waiting_for),
                maintenance.dhcp_auto_pump_polls,
                maintenance.dhcp_auto_pump_elapsed_ms,
                maintenance.dhcp_auto_pump_remaining_ms
            );
            kprintln!(
                "  DHCP arb:   manual_active={} preempted_auto={} auto_timeout_before_manual={} stale_ignored={} count={} last_auto={}",
                if maintenance.dhcp_manual_active { "yes" } else { "no" },
                if maintenance.dhcp_manual_preempted_auto { "yes" } else { "no" },
                if maintenance.dhcp_auto_timed_out_before_manual { "yes" } else { "no" },
                if maintenance.dhcp_stale_worker_ignored { "yes" } else { "no" },
                maintenance.dhcp_stale_worker_ignored_count,
                maintenance.dhcp_last_auto_state
            );
            print_usb_tethering_status("  ");
            kprintln!("  ARP cache: {} entrie(s)", net::arp_entries().len());
            kprintln!("  DNS cache: {} entrie(s)", net::dns_cache().len());
            let supported_detected: Vec<_> = hal::devices()
                .into_iter()
                .filter(|device| {
                    matches!(
                        classify_network_inventory(device),
                        Some(NetworkInventoryState::SupportedDetected)
                    )
                })
                .collect();
            let unsupported_wired: Vec<_> = hal::devices()
                .into_iter()
                .filter(|device| {
                    matches!(
                        classify_network_inventory(device),
                        Some(NetworkInventoryState::UnsupportedDetected)
                    )
                })
                .collect();
            let discovered_wireless: Vec<_> = hal::devices()
                .into_iter()
                .filter(|device| {
                    matches!(
                        classify_network_inventory(device),
                        Some(NetworkInventoryState::WifiProbeOnly)
                    )
                })
                .collect();
            print_network_inventory_section(
                "  Supported wired controllers detected but not active:",
                &supported_detected,
            );
            print_network_inventory_section(
                "  Detected but unsupported wired NICs:",
                &unsupported_wired,
            );
            print_network_inventory_section(
                "  Probe-only wireless controllers:",
                &discovered_wireless,
            );
            kprintln!(
                "  Wi-Fi:     detection/status only; scan/connect/auth/data path not implemented"
            );
        }
        "diag" => {
            if !require_session_capability(
                security::capabilities::Capabilities::NET_ADMIN,
                "NET_ADMIN",
                "net diag",
            ) {
                return;
            }
            let Some(diag) = net::hardware_diagnostics() else {
                kprint_colored!(Colors::RED, "[WarOS] NET:");
                kprintln!(" no NIC is initialized.");
                return;
            };

            let explicit_target = match parts.next() {
                Some(value) => match net::ipv4::Ipv4Addr::parse(value.trim()) {
                    Some(target) => Some(target),
                    None => {
                        kprint_colored!(Colors::RED, "[WarOS] NET:");
                        kprintln!(" net diag expects an IPv4 target like 'net diag 192.168.1.1'.");
                        return;
                    }
                },
                None => None,
            };
            let target =
                explicit_target.or_else(|| net::network_config().and_then(|config| config.gateway));

            kprintln!("WarOS Network Diagnostics");
            let maintenance = net::maintenance_report();
            kprintln!(
                "  Active sel:    iface={} driver={} usb={} iface_ready={} dhcp_socket={} lease_state={} lease_view={}",
                maintenance.active_interface,
                maintenance.active_driver,
                if maintenance.active_is_usb { "yes" } else { "no" },
                if maintenance.iface_ready { "ready" } else { "missing" },
                if maintenance.dhcp_socket_ready { "ready" } else { "missing" },
                maintenance.canonical_lease_state,
                maintenance.lease_view
            );
            kprintln!(
                "  Poll service:  last_poll={}ms ago at +{}ms | dhcp_event={}ms ago at +{}ms | wire_last={}",
                maintenance.ms_since_poll,
                maintenance.last_poll_ms,
                maintenance.ms_since_dhcp_event,
                maintenance.last_dhcp_event_ms,
                maintenance.dhcp_last_wire_event
            );
            kprintln!(
                "  DHCP owner:   owner={} gen={} trigger={} start={}ms deadline={}ms elapsed={}ms remaining={}ms",
                maintenance.dhcp_owner,
                maintenance.dhcp_generation,
                maintenance.dhcp_owner_trigger,
                maintenance.dhcp_owner_started_ms,
                maintenance.dhcp_owner_deadline_ms,
                maintenance.dhcp_owner_elapsed_ms,
                maintenance.dhcp_owner_remaining_ms
            );
            kprintln!(
                "  Worker state: auto_active={} manual_active={} trigger={} state={} wait={} polls={} start={}ms deadline={}ms remaining={}ms last={}ms",
                if maintenance.dhcp_auto_pump_active {
                    "yes"
                } else {
                    "no"
                },
                if maintenance.dhcp_manual_active { "yes" } else { "no" },
                maintenance.dhcp_auto_pump_trigger,
                maintenance.dhcp_auto_pump_state,
                dhcp_wait_label(maintenance.dhcp_auto_pump_waiting_for),
                maintenance.dhcp_auto_pump_polls,
                maintenance.dhcp_auto_pump_started_ms,
                maintenance.dhcp_auto_pump_deadline_ms,
                maintenance.dhcp_auto_pump_remaining_ms,
                maintenance.dhcp_auto_pump_last_service_ms
            );
            kprintln!(
                "  Ownership:    manual_preempted_auto={} auto_timeout_before_manual={} stale_ignored={} count={} last_auto={}",
                if maintenance.dhcp_manual_preempted_auto { "yes" } else { "no" },
                if maintenance.dhcp_auto_timed_out_before_manual { "yes" } else { "no" },
                if maintenance.dhcp_stale_worker_ignored { "yes" } else { "no" },
                maintenance.dhcp_stale_worker_ignored_count,
                maintenance.dhcp_last_auto_state
            );
            kprintln!(
                "  Read-only poll: trigger={} state={} polls={} start={}ms end={}ms",
                maintenance.read_only_trigger,
                maintenance.read_only_state,
                maintenance.read_only_polls,
                maintenance.read_only_started_ms,
                maintenance.read_only_finished_ms
            );
            let dhcp_attempt = net::dhcp_attempt();
            kprintln!(
                "  DHCP last:     {} (iface={} timeout={}ms start={}ms end={}ms polls={})",
                dhcp_attempt.state,
                dhcp_attempt.interface,
                dhcp_attempt.timeout_ms,
                dhcp_attempt.started_ms,
                dhcp_attempt.finished_ms,
                dhcp_attempt.polls
            );
            kprintln!(
                "  DHCP wire:     discover={} offer={} request={} ack={} nak={} last={}",
                dhcp_attempt.discover_sent,
                dhcp_attempt.offer_received,
                dhcp_attempt.request_sent,
                dhcp_attempt.ack_received,
                dhcp_attempt.nak_received,
                dhcp_attempt.last_event
            );
            kprintln!(
                "  DHCP frames:   tx_delta={} rx_delta={}",
                dhcp_attempt.tx_frames_delta,
                dhcp_attempt.rx_frames_delta
            );
            kprintln!(
                "  DHCP RX path:  eth={} ipv4={} udp={} udp67-68={} dhcp={} parse_err={} drop={}",
                dhcp_attempt.rx_eth_frames,
                dhcp_attempt.rx_ipv4_frames,
                dhcp_attempt.rx_udp_frames,
                dhcp_attempt.rx_udp_67_68,
                dhcp_attempt.rx_dhcp_frames,
                dhcp_attempt.rx_dhcp_parse_errors,
                dhcp_attempt.dhcp_drop_reason
            );
            match diag {
                net::NetworkDiagnostics::Virtio(diag) => {
                    kprintln!("  Driver:        virtio-net");
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
                }
                net::NetworkDiagnostics::E1000(diag) => {
                    kprintln!("  Driver:        e1000");
                    kprintln!(
                        "  CTRL/STATUS:   0x{:08X} / 0x{:08X}",
                        diag.ctrl,
                        diag.status
                    );
                    kprintln!("  RX head/tail:  {} / {}", diag.rx_head, diag.rx_tail);
                    kprintln!("  TX head/tail:  {} / {}", diag.tx_head, diag.tx_tail);
                    kprintln!(
                        "  Frames:        tx={} rx={}",
                        diag.tx_frames,
                        diag.rx_frames
                    );
                }
                net::NetworkDiagnostics::Rtl8169(diag) => {
                    kprintln!(
                        "  Driver:        rtl8169 ({})",
                        if diag.is_rtl8168 { "RTL8168" } else { "RTL8169" }
                    );
                    kprintln!(
                        "  PCI:           dev {:04X} rev 0x{:02X} irq {} cmd 0x{:04X}",
                        diag.device_id,
                        diag.revision_id,
                        diag.interrupt_line,
                        diag.pci_command
                    );
                    kprintln!(
                        "  MAC version:   0x{:03X} (ASPM disabled, quirks={})",
                        diag.mac_version,
                        if diag.is_rtl8168 { "rtl8168" } else { "generic" }
                    );
                    kprintln!(
                        "  CMD/PHY:       0x{:02X} / 0x{:02X}",
                        diag.chip_cmd,
                        diag.phy_status
                    );
                    kprintln!(
                        "  ISR/IMR:       0x{:04X} / 0x{:04X}",
                        diag.intr_status,
                        diag.intr_mask
                    );
                    kprintln!("  Ring idx:      RX {} | TX {}", diag.rx_head, diag.tx_tail);
                    kprintln!(
                        "  PHY regs:      BMCR 0x{:04X} | BMSR 0x{:04X} | ANAR 0x{:04X} | GBCR 0x{:04X}",
                        diag.phy_control,
                        diag.phy_register_status,
                        diag.phy_auto_negotiation,
                        diag.phy_gigabit_control
                    );
                    kprintln!(
                        "  PHY partner:   ANLPAR 0x{:04X} | ANER 0x{:04X}",
                        diag.phy_link_partner,
                        diag.phy_expansion
                    );
                    kprintln!(
                        "  Link state:    {} | {} Mbps | {} duplex | reset={}",
                        if diag.link_up { "up" } else { "down" },
                        diag.link_speed_mbps,
                        if diag.full_duplex { "full" } else { "half" },
                        diag.reset_complete
                    );
                    kprintln!(
                        "  Frames:        tx={} rx={} attempts={}",
                        diag.tx_frames,
                        diag.rx_frames,
                        diag.tx_attempts
                    );
                    kprintln!(
                        "  Errors:        tx_err={} rx_err={}",
                        diag.tx_errors,
                        diag.rx_errors
                    );
                    kprintln!(
                        "  IRQ events:    rx={} tx={} link-change={}",
                        diag.rx_interrupts,
                        diag.tx_interrupts,
                        diag.link_changes
                    );
                }
                net::NetworkDiagnostics::UsbNet(diag) => {
                    kprintln!(
                        "  Driver:        usb-net ({:?})",
                        diag.protocol
                    );
                    kprintln!(
                        "  MAC:           {}",
                        net::format_mac(&diag.mac)
                    );
                    kprintln!(
                        "  Frames:        tx={} rx={}",
                        diag.tx_frames,
                        diag.rx_frames
                    );
                    kprintln!(
                        "  USB RX raw:    events={} extract_err={} short={} raw_len={} frame_len={} drop={}",
                        diag.rx_raw_events,
                        diag.rx_extract_errors,
                        diag.rx_short_frames,
                        diag.rx_last_raw_len,
                        diag.rx_last_frame_len,
                        diag.rx_last_drop
                    );
                    kprintln!(
                        "  Errors:        tx_err={} rx_err={}",
                        diag.tx_errors,
                        diag.rx_errors
                    );
                    kprintln!(
                        "  RX queue:      depth={} armed={}",
                        diag.rx_queue_depth,
                        diag.rx_armed
                    );
                }
            }
            let Some(target) = target else {
                kprintln!(
                    "  ARP probe:     skipped (no DHCP gateway; specify an on-link IPv4 target)"
                );
                return;
            };
            kprintln!("  ARP probe:     who-has {}", target);

            match net::send_arp_probe(target) {
                Ok(()) => kprintln!("  Probe status:  transmitted"),
                Err(error) => {
                    kprint_colored!(Colors::RED, "[WarOS] NET:");
                    kprintln!(" ARP probe failed: {}.", error);
                    return;
                }
            }

            let mut events = 0usize;
            let mut remaining = u64::from(PIT_FREQUENCY_HZ);
            while remaining > 0 {
                events = events.saturating_add(net::poll());
                remaining = remaining.saturating_sub(wait_for_runtime_tick());
            }

            if let Some(after) = net::hardware_diagnostics() {
                match after {
                    net::NetworkDiagnostics::Virtio(after) => {
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
                    }
                    net::NetworkDiagnostics::E1000(after) => {
                        kprintln!(
                            "  After probe:   tx={} rx={} events={}",
                            after.tx_frames,
                            after.rx_frames,
                            events
                        );
                        kprintln!(
                            "  Rings now:     RX {} / {} | TX {} / {}",
                            after.rx_head,
                            after.rx_tail,
                            after.tx_head,
                            after.tx_tail
                        );
                    }
                    net::NetworkDiagnostics::Rtl8169(after) => {
                        kprintln!(
                            "  After probe:   tx={} rx={} attempts={} events={}",
                            after.tx_frames,
                            after.rx_frames,
                            after.tx_attempts,
                            events
                        );
                        kprintln!(
                            "  State now:     CMD 0x{:02X} | PHY 0x{:02X} | ISR 0x{:04X} | BMSR 0x{:04X} | link={} {} Mbps {} duplex",
                            after.chip_cmd,
                            after.phy_status,
                            after.intr_status,
                            after.phy_register_status,
                            if after.link_up { "up" } else { "down" },
                            after.link_speed_mbps,
                            if after.full_duplex { "full" } else { "half" }
                        );
                    }
                    net::NetworkDiagnostics::UsbNet(after) => {
                        kprintln!(
                            "  After probe:   tx={} rx={} events={}",
                            after.tx_frames,
                            after.rx_frames,
                            events
                        );
                        kprintln!(
                            "  USB NIC:       rx_queue={} rx_armed={}",
                            after.rx_queue_depth,
                            after.rx_armed
                        );
                    }
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
            kprintln!(
                "Polled network stack: {} DHCP/stack state change(s) reported.",
                harvested
            );
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
        "dhcp" => {
            if !require_session_capability(
                security::capabilities::Capabilities::NET_ADMIN,
                "NET_ADMIN",
                "net dhcp",
            ) {
                return;
            }
            if let Err(reason) = ensure_supported_wired_nic() {
                kprint_colored!(Colors::RED, "[WarOS] NET:");
                kprintln!(" {}.", reason);
                return;
            }
            let timeout_ms = parts
                .next()
                .and_then(|value| value.trim().parse::<u64>().ok())
                .unwrap_or(net::DEFAULT_DHCP_TIMEOUT_MS);
            kprintln!("Starting DHCP on active interface (timeout={} ms)...", timeout_ms);
            match net::wait_for_dhcp(timeout_ms) {
                Ok(Some(config)) => {
                    kprint_colored!(Colors::GREEN, "[WarOS] NET: ");
                    kprintln!("DHCP lease acquired: {}", config.cidr_string());
                    kprintln!("  Gateway: {}", format_optional_ipv4(config.gateway));
                    kprintln!("  DNS:     {}", format_optional_ipv4(config.dns_server));
                }
                Ok(None) => {
                    kprint_colored!(Colors::YELLOW, "[WarOS] NET: ");
                    kprintln!(
                        "No DHCP lease became active before the {} ms timeout.",
                        timeout_ms
                    );
                }
                Err(error) => {
                    kprint_colored!(Colors::RED, "[WarOS] NET:");
                    kprintln!(" DHCP request failed: {}.", error);
                }
            }
        }
        "retry" => {
            if !require_session_capability(
                security::capabilities::Capabilities::NET_ADMIN,
                "NET_ADMIN",
                "net retry",
            ) {
                return;
            }
            let timeout_ms = parts
                .next()
                .and_then(|value| value.trim().parse::<u64>().ok())
                .unwrap_or(3_000);
            kprintln!("Re-running PHY/autoneg bring-up on the active wired NIC...");
            match net::retry_active_nic(timeout_ms) {
                Ok(true) => {
                    kprint_colored!(Colors::GREEN, "[WarOS] NET: ");
                    kprintln!("link came up after retry");
                    if let Some(hardware) = net::hardware_status() {
                        kprintln!("  Link:   {}", network_link_summary(&hardware));
                    }
                    if let Some(config) = net::network_config() {
                        kprintln!("  IPv4:   {}", config.cidr_string());
                        kprintln!("  Gateway: {}", format_optional_ipv4(config.gateway));
                        kprintln!("  DNS:    {}", format_optional_ipv4(config.dns_server));
                    } else {
                        kprintln!("  DHCP:   no lease yet — try 'net dhcp'");
                    }
                }
                Ok(false) => {
                    kprint_colored!(Colors::YELLOW, "[WarOS] NET: ");
                    kprintln!(
                        "link still down after {} ms (cable, switch port, or PHY did not respond)",
                        timeout_ms
                    );
                }
                Err(error) => {
                    kprint_colored!(Colors::RED, "[WarOS] NET:");
                    kprintln!(" net retry failed: {}.", error);
                }
            }
        }
        "route" => {
            kprintln!("Routing:");
            if let Some(config) = net::network_config() {
                if let Some(gateway) = config.gateway {
                    kprintln!("  default via {}", gateway);
                } else {
                    kprintln!("  default route unavailable");
                }
                kprintln!("  connected {}", config.cidr_string());
            } else {
                kprintln!("  no active IPv4 route (no DHCP lease)");
            }
        }
        "arp" => {
            let entries = net::arp_entries();
            if entries.is_empty() {
                kprintln!("ARP cache: empty");
            } else {
                kprintln!("ARP cache ({} entries):", entries.len());
                for entry in &entries {
                    kprintln!(
                        "  {} -> {} (observed at {}ms since boot)",
                        entry.ip,
                        net::format_mac(&entry.mac),
                        entry.timestamp_ms
                    );
                }
            }
            kprintln!(
                "  Note: ARP cache is observation-based; entries do not expire automatically."
            );
        }
        "dns-cache" => {
            let entries = net::dns_cache();
            if entries.is_empty() {
                kprintln!("DNS cache: empty");
            } else {
                kprintln!("DNS cache ({} entries):", entries.len());
                for entry in &entries {
                    kprintln!(
                        "  {} -> {} (TTL 60s from query time)",
                        entry.domain,
                        entry.ip
                    );
                }
            }
            kprintln!("  Note: DNS resolves A records only (IPv4). Cache entries expire after 60 seconds.");
        }
        "txprobe" => {
            if !require_session_capability(
                security::capabilities::Capabilities::NET_RAW,
                "NET_RAW",
                "net txprobe",
            ) {
                return;
            }
            let Some(hardware) = net::hardware_status() else {
                kprint_colored!(Colors::RED, "[ERR]");
                kprintln!(" no active NIC is initialized.");
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
        "usb" => {
            if !require_session_capability(
                security::capabilities::Capabilities::NET_ADMIN,
                "NET_ADMIN",
                "net usb",
            ) {
                return;
            }
            let usb_action = parts
                .next()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("attach");
            match usb_action {
                "status" => {
                    kprintln!("USB network path:");
                    kprintln!("  Snapshot mode: no live topology rescan is forced from this command.");
                    print_usb_runtime_summary("  ", false);
                    print_usb_tethering_status("  ");
                    if net::active_is_usb() {
                        kprintln!("  Active path: USB NIC currently owns the network stack.");
                    } else {
                        kprintln!("  Active path: non-USB or offline (manual attach required).");
                    }
                }
                "attach" => {
                    kprintln!("Attaching USB NIC...");
                    match net::attach_usb_nic() {
                        Ok(()) => {
                            if let Some(hw) = net::hardware_status() {
                                kprint_colored!(Colors::GREEN, "USB NIC active: ");
                                kprintln!(
                                    "{} ({}) mac={}",
                                    hw.name,
                                    hw.driver,
                                    net::format_mac(&hw.mac)
                                );
                                let config = net::network_config();
                                let maintenance = net::maintenance_report();
                                if let Some(config) = config {
                                    kprintln!(
                                        "  IPv4: {} gw {}",
                                        config.cidr_string(),
                                        config.gateway.unwrap_or(net::ipv4::Ipv4Addr::ZERO)
                                    );
                                } else {
                                    kprintln!("  DHCP: {}", ipv4_lease_status_line(&maintenance));
                                }
                                print_usb_tethering_status("  ");
                            }
                        }
                        Err(error) => {
                            kprint_colored!(Colors::RED, "[ERR]");
                            kprintln!(" USB NIC attach failed: {}.", error);
                            print_usb_runtime_summary("  ", false);
                            print_usb_tethering_status("  ");
                            kprintln!("  Ensure a USB Ethernet adapter or phone tethering is connected and actually exposes a USB network interface.");
                            kprintln!("  Status/attach are snapshot-based; if the device was just connected, wait for runtime enumeration and retry 'net usb status'.");
                            kprintln!("  WarOS currently supports detection-first CDC-ECM / CDC-NCM / RNDIS paths only.");
                        }
                    }
                }
                other => {
                    kprintln!("Usage: net usb [status|attach]");
                    kprintln!("  Unknown action: {}", other);
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
                    kprintln!("text message over the legacy COM2 serial transport.");
                }
                Err(_error) => {
                    kprint_colored!(Colors::RED, "[ERR]");
                    kprintln!(" failed to send serial text frame.");
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
                        kprintln!("'{}' over the legacy COM2 serial transport.", path);
                    }
                    Err(_error) => {
                        kprint_colored!(Colors::RED, "[ERR]");
                        kprintln!(" failed to send serial circuit frame.");
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
                    net::MessageType::Ping => kprintln!("[SERIAL] Received ping."),
                    net::MessageType::Pong => kprintln!("[SERIAL] Received pong."),
                    net::MessageType::CircuitData => {
                        kprintln!("[SERIAL] Circuit payload:");
                        if let Ok(text) = str::from_utf8(&message.payload) {
                            kprintln!("{}", text);
                        }
                    }
                    net::MessageType::MeasurementResult => {
                        kprintln!("[SERIAL] Measurement result payload:");
                        if let Ok(text) = str::from_utf8(&message.payload) {
                            kprintln!("{}", text);
                        }
                    }
                    net::MessageType::Text => {
                        if let Ok(text) = str::from_utf8(&message.payload) {
                            kprintln!("[SERIAL] {}", text);
                        }
                    }
                }
            }
            if received == 0 {
                kprintln!("No pending COM2 serial messages.");
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
        kprintln!(" no supported active NIC is present.");
        let discovered: Vec<_> = hal::devices()
            .into_iter()
            .filter(|device| device.info.category == hal::DeviceCategory::Network)
            .collect();
        if !discovered.is_empty() {
            let supported_detected: Vec<_> = discovered
                .iter()
                .filter(|device| {
                    matches!(
                        classify_network_inventory(device),
                        Some(NetworkInventoryState::SupportedDetected)
                    )
                })
                .cloned()
                .collect();
            let unsupported_wired: Vec<_> = discovered
                .iter()
                .filter(|device| {
                    matches!(
                        classify_network_inventory(device),
                        Some(NetworkInventoryState::UnsupportedDetected)
                    )
                })
                .cloned()
                .collect();
            let wifi_probe: Vec<_> = discovered
                .iter()
                .filter(|device| {
                    matches!(
                        classify_network_inventory(device),
                        Some(NetworkInventoryState::WifiProbeOnly)
                    )
                })
                .cloned()
                .collect();
            print_network_inventory_section(
                "Detected supported wired controllers:",
                &supported_detected,
            );
            print_network_inventory_section("Detected unsupported wired NICs:", &unsupported_wired);
            print_network_inventory_section(
                "Detected probe-only wireless controllers:",
                &wifi_probe,
            );
        }
        return;
    };

    kprintln!("Interface: {} ({})", hardware.name, hardware.driver);
    kprintln!("  Note:    read-only status; manual IP/route configuration is not implemented.");
    kprintln!("  Bus:     {}", if net::active_is_usb() { "USB (CDC-ECM/NCM/RNDIS class)" } else { "PCI" });
    kprintln!("  MAC:     {}", net::format_mac(&hardware.mac));
    kprintln!("  Link:    {}", network_link_summary(&hardware));
    kprintln!("  IRQ:     {}", hardware.interrupt_line);
    kprintln!(
        "  RX/TX:   {} / {} frames  (pending {})",
        hardware.rx_frames, hardware.tx_frames, hardware.pending_frames
    );
    let config = net::network_config();
    let maintenance = net::maintenance_report();
    if let Some(config) = config {
        kprintln!("  IPv4:    {}", config.cidr_string());
        kprintln!("  Mask:    {}", config.subnet_mask);
        kprintln!("  Gateway: {}", format_optional_ipv4(config.gateway));
        kprintln!("  DNS:     {}", format_optional_ipv4(config.dns_server));
    } else {
        kprintln!("  IPv4:    {}", ipv4_ifconfig_line(&maintenance));
    }

    let discovered: Vec<_> = hal::devices()
        .into_iter()
        .filter(|device| device.info.category == hal::DeviceCategory::Network)
        .collect();
    if !discovered.is_empty() {
        let supported_detected: Vec<_> = discovered
            .iter()
            .filter(|device| {
                matches!(
                    classify_network_inventory(device),
                    Some(NetworkInventoryState::SupportedDetected)
                )
            })
            .cloned()
            .collect();
        let unsupported_wired: Vec<_> = discovered
            .iter()
            .filter(|device| {
                matches!(
                    classify_network_inventory(device),
                    Some(NetworkInventoryState::UnsupportedDetected)
                )
            })
            .cloned()
            .collect();
        let wifi_probe: Vec<_> = discovered
            .iter()
            .filter(|device| {
                matches!(
                    classify_network_inventory(device),
                    Some(NetworkInventoryState::WifiProbeOnly)
                )
            })
            .cloned()
            .collect();
        print_network_inventory_section(
            "Other detected supported wired controllers:",
            &supported_detected,
        );
        print_network_inventory_section("Detected unsupported wired NICs:", &unsupported_wired);
        print_network_inventory_section("Detected wireless-class controllers:", &wifi_probe);
    }
}

fn cmd_wifi(args: &[&str]) {
    let subcommand = args.first().copied().unwrap_or("status");

    match subcommand {
        "status" => {}
        "probe" => {
            if !require_session_capability(
                security::capabilities::Capabilities::NET_ADMIN,
                "NET_ADMIN",
                "wifi probe",
            ) {
                return;
            }
            let probes = hal::net::probe_intel_wifi();
            if probes.is_empty() {
                kprint_colored!(Colors::YELLOW, "[wifi probe] ");
                kprintln!("no Intel iwlwifi-class controllers detected on PCI bus.");
                return;
            }
            kprintln!(
                "Intel iwlwifi probe ({} controller{} found, read-only):",
                probes.len(),
                if probes.len() == 1 { "" } else { "s" }
            );
            for probe in &probes {
                let (step, dash) = probe.step_dash();
                kprintln!(
                    "  PCI {:02X}:{:02X}.{}  {:04X}:{:04X} rev 0x{:02X}",
                    probe.bus,
                    probe.device,
                    probe.function,
                    probe.vendor_id,
                    probe.device_id,
                    probe.revision_id
                );
                kprintln!("    Family:        {}", probe.family);
                kprintln!("    BAR0 phys:     0x{:016X}", probe.bar0_phys);
                kprintln!(
                    "    CSR_HW_REV:    0x{:08X} (step={} dash={})",
                    probe.csr_hw_rev,
                    step,
                    dash
                );
                kprintln!(
                    "    CSR_HW_RF_ID:  0x{:08X} ({})",
                    probe.csr_hw_rf_id,
                    probe.rf_type_label()
                );
                kprintln!("    CSR_GP_CNTRL:  0x{:08X}", probe.csr_gp_cntrl);
                kprintln!("    CSR_HW_IF_CFG: 0x{:08X}", probe.csr_hw_if_config);
                if probe.mmio_alive {
                    kprint_colored!(Colors::GREEN, "    MMIO alive:    yes");
                    kprintln!(" — silicon responded to CSR reads");
                } else {
                    kprint_colored!(Colors::RED, "    MMIO alive:    no");
                    kprintln!(" — controller not powered or BAR0 routing broken");
                }
                kprintln!("    Firmware:      {}", probe.firmware_required);
                kprintln!(
                    "    Firmware state: {} via {} ({})",
                    probe.firmware.state.as_str(),
                    probe.firmware.provider,
                    probe.firmware.reason
                );
                kprintln!(
                    "    Blob present:   {}",
                    if probe.firmware.blob_found { "yes" } else { "no" }
                );
                kprintln!(
                    "    Blob path:      {}",
                    probe.firmware
                        .blob_path
                        .as_deref()
                        .unwrap_or("not-found")
                );
                kprintln!(
                    "    Load/upload:    attempted={} upload={}",
                    probe.firmware.load_attempted,
                    probe.firmware.upload_state
                );
                kprintln!("    Driver:        {}", probe.driver_state);
                kprintln!("    Transport:     {}", probe.transport_state);
                kprintln!("    Scan:          {}", probe.scan_state);
                kprintln!("    Connect/auth:  {}", probe.connect_state);
                kprintln!("    Data path:     {}", probe.data_state);
                kprintln!("    Next step:     {}", probe.next_step);
                kprintln!("    Blocker:       {}", probe.blocker);
            }
            kprintln!(
                "  Note: this is honest probe-only. WarOS can now locate firmware blobs in WarFS, but no upload path,"
            );
            kprintln!(
                "        FH command queue, NVM parser, or 802.11 MAC yet. wifi scan/connect remain staged."
            );
            return;
        }
        "scan" => {
            kprint_colored!(Colors::YELLOW, "[STAGED]");
            kprintln!(
                " wifi scan is not implemented. WarOS does not yet have an 802.11 scan path."
            );
            kprintln!(
                "  Wireless-capable controllers are detected via PCI but no driver is loaded."
            );
            kprintln!("  Run 'wifi probe' for honest CSR-level introspection of Intel hardware.");
            return;
        }
        "connect" => {
            kprint_colored!(Colors::YELLOW, "[STAGED]");
            kprintln!(" wifi connect is not implemented. WarOS does not yet support 802.11 association or authentication.");
            kprintln!("  No SSID selection, WPA/WPA2/WPA3, or DHCP-over-Wi-Fi exists.");
            kprintln!("  Use 'wifi status' to see current capability level.");
            return;
        }
        "disconnect" => {
            kprint_colored!(Colors::YELLOW, "[STAGED]");
            kprintln!(
                " wifi disconnect is not implemented. No active Wi-Fi connection support exists."
            );
            return;
        }
        _ => {
            kprintln!("Usage: wifi [status|probe|scan|connect|disconnect]");
            kprintln!("  status      Show Wi-Fi capability and detected hardware");
            kprintln!("  probe       Read iwlwifi CSR registers from real silicon (read-only)");
            kprintln!("  scan        [staged] Scan for nearby access points");
            kprintln!("  connect     [staged] Connect to an access point");
            kprintln!("  disconnect  [staged] Disconnect from current AP");
            return;
        }
    }

    let candidate_controllers: Vec<_> = hal::devices()
        .into_iter()
        .filter(|device| {
            device.info.category == hal::DeviceCategory::Network
                && matches!(
                    &device.info.capabilities,
                    hal::DeviceCapabilities::Network(cap) if cap.has_wifi
                )
        })
        .collect();

    kprintln!("Wi-Fi status:");
    kprintln!("  Hardware path:      PCI detection and Intel iwlwifi CSR probe foundation");
    kprintln!("  Driver path:        probe foundation present; upload/NVM/MAC still staged");
    kprintln!(
        "  Firmware loader:    WarFS lookup roots active ({})",
        firmware_search_roots_summary()
    );
    kprintln!("  Scan/list path:     unavailable until firmware upload + transport init exist");
    kprintln!("  Association/auth:   unavailable");
    kprintln!("  DHCP over Wi-Fi:    unavailable");
    kprintln!("  Traffic path:       unavailable");
    kprintln!(
        "  Active NIC paths:   USB RNDIS/ECM/NCM, wired RTL8168/RTL8169, Intel E1000, VirtIO-net"
    );
    if candidate_controllers.is_empty() {
        kprintln!("  Wireless-class controllers detected: none");
    } else {
        kprintln!("  Wireless-class controllers detected:");
        for device in &candidate_controllers {
            let vendor_label = pci_vendor_name(device.info.vendor_id);
            kprintln!(
                "    {:<12} {:04X}:{:04X} [{}]",
                format_bus_location(&device.info.bus),
                device.info.vendor_id,
                device.info.product_id,
                vendor_label
            );
            kprintln!(
                "               {}",
                device.info.name
            );
            // Show specific blocker for Intel WiFi
            if device.info.vendor_id == 0x8086 {
                let firmware = hal::net::firmware::require_blob(
                    hal::net::iwlwifi::firmware_name(device.info.product_id),
                );
                kprintln!(
                    "               Driver: probe foundation present; transport not initialized"
                );
                kprintln!(
                    "               Firmware: {} ({})",
                    firmware.name_pattern,
                    firmware.state.as_str()
                );
                kprintln!(
                    "               Loader: {} - {}",
                    firmware.provider,
                    firmware.reason
                );
                kprintln!(
                    "               Blob: {}",
                    firmware.blob_path.as_deref().unwrap_or("not-found")
                );
                kprintln!(
                    "               Upload: attempted={} result={}",
                    firmware.load_attempted,
                    firmware.upload_state
                );
                kprintln!(
                    "               Capabilities: scan=no connect=no auth=no data=no"
                );
                kprintln!(
                    "               Status: detected + classified, not drivable yet."
                );
            } else {
                kprintln!(
                    "               Status: detected, no driver available."
                );
            }
        }
    }
    kprintln!(
        "  USB Wi-Fi:  not driven as network interfaces yet."
    );
    kprintln!(
        "  Workaround: use USB tethering or USB Ethernet adapter for connectivity."
    );
}

fn cmd_ping(args: &[&str]) {
    let Some(target) = args.first().copied() else {
        kprintln!("Usage: ping <host>");
        return;
    };

    if !require_session_capability(
        security::capabilities::Capabilities::NET_RAW,
        "NET_RAW",
        "ping",
    ) {
        return;
    }
    if let Err(reason) = ensure_ip_connectivity_for_host(target) {
        kprint_colored!(Colors::RED, "[ERR]");
        kprintln!(" ping failed: {}.", reason);
        return;
    }

    let tick_before = runtime_ticks();
    match net::ping_host(target) {
        Ok(reply) => {
            let tick_after = runtime_ticks();
            let elapsed_ticks = tick_after.saturating_sub(tick_before);
            let elapsed_ms = elapsed_ticks * 1000 / u64::from(PIT_FREQUENCY_HZ);
            kprint_colored!(Colors::GREEN, "Reply ");
            kprintln!(
                "from {}: seq={} bytes={} time=~{}ms",
                reply.source,
                reply.seq_no,
                reply.payload_len,
                elapsed_ms
            );
        }
        Err(error) => {
            kprint_colored!(Colors::RED, "[ERR]");
            kprintln!(" ping failed: {}.", error);
            if matches!(&error, net::NetError::ProtocolError(message) if message.contains("no reply was received"))
            {
                kprintln!(
                    " note: the interface and route can still be healthy here; many gateways simply do not answer ICMP echo on-link."
                );
            }
        }
    }
}

fn cmd_dns(args: &[&str]) {
    let Some(domain) = args.first().copied() else {
        kprintln!("Usage: dns <domain>");
        return;
    };

    if let Err(reason) = ensure_ip_connectivity_for_host(domain) {
        kprint_colored!(Colors::RED, "[ERR]");
        kprintln!(" DNS lookup failed: {}.", reason);
        return;
    }

    match net::resolve_host(domain) {
        Ok(address) => kprintln!("{} -> {}", domain, address),
        Err(error) => {
            kprint_colored!(Colors::RED, "[ERR]");
            kprintln!(" DNS lookup failed: {}.", error);
        }
    }
}

fn cmd_wget(args: &[&str]) {
    let Ok((url, requested_output)) = parse_wget_args(args) else {
        kprintln!("Usage: wget <url>");
        kprintln!("       wget -o <file> <url>");
        kprintln!("       wget -O <file> <url>");
        kprintln!("  Without -o/-O, WarOS derives a safe filename from the final URL and response type.");
        return;
    };
    if let Err(reason) = ensure_http_request_ready(url) {
        kprint_colored!(Colors::RED, "[ERR]");
        kprintln!(" request failed: {}.", reason);
        return;
    }
    if url.starts_with("https://") {
        kprint_colored!(Colors::YELLOW, "[NOTE]");
        kprintln!(
            " HTTPS is limited to {}. Other hosts will fail.",
            net::tls::supported_hosts_summary()
        );
    }

    serial_println!(
        "[SHELL] wget {}{}",
        url,
        requested_output
            .map(|path| alloc::format!(" -> {}", path))
            .unwrap_or_default()
    );
    match net::http_get(url) {
        Ok(response) => {
            let status_code = response.status_code;
            let redirects_followed = response.redirects_followed;
            let effective_url = response.effective_url.clone();
            let body_len = response.body.len();
            serial_println!(
                "[SHELL] wget completed: HTTP {} ({} bytes)",
                status_code,
                body_len
            );
            if redirects_followed != 0 {
                kprintln!(
                    "Followed {} redirect(s) to {}.",
                    redirects_followed,
                    effective_url
                );
            }
            if !http_status_is_success(status_code) {
                kprint_colored!(Colors::RED, "[ERR]");
                kprintln!(
                    " download failed: server returned HTTP {} from {}. No file was written.",
                    status_code,
                    effective_url
                );
                return;
            }
            let (output, used_fallback_name) = match requested_output {
                Some(path) => (path.to_string(), false),
                None => match default_download_path(&response.effective_url, &response.headers) {
                    Ok(derived) => (derived.path, derived.used_fallback),
                    Err(error) => {
                        kprint_colored!(Colors::YELLOW, "[NOTE]");
                        kprintln!(
                            " {} Final URL: {}. Use 'wget -O <file> <url>' to choose the output path explicitly.",
                            error,
                            response.effective_url
                        );
                        return;
                    }
                },
            };
            match fs::write_current_owned(&output, response.body) {
                Ok(path) => {
                    kprint_colored!(Colors::GREEN, "[WarOS] NET: ");
                    if requested_output.is_none() && used_fallback_name {
                        kprintln!(
                            "saved {} byte(s) from {} to '{}' (HTTP {}, auto-derived filename).",
                            body_len,
                            effective_url,
                            fs::display_path(&path),
                            status_code
                        );
                    } else {
                        kprintln!(
                            "saved {} byte(s) from {} to '{}' (HTTP {}).",
                            body_len,
                            effective_url,
                            fs::display_path(&path),
                            status_code
                        );
                    }
                }
                Err(error) => report_fs_error(&output, error),
            }
        }
        Err(error) => report_http_request_error(url, &error),
    }
}

fn cmd_curl(args: &[&str]) {
    let Ok((url, output_mode)) = parse_curl_args(args) else {
        kprintln!("Usage: curl <url>");
        kprintln!("       curl -o <file> <url>");
        return;
    };
    if let Err(reason) = ensure_http_request_ready(url) {
        kprint_colored!(Colors::RED, "[ERR]");
        kprintln!(" request failed: {}.", reason);
        return;
    }
    if url.starts_with("https://") {
        kprint_colored!(Colors::YELLOW, "[NOTE]");
        kprintln!(
            " HTTPS is limited to {}. Other hosts will fail.",
            net::tls::supported_hosts_summary()
        );
    }

    serial_println!("[SHELL] curl {}", url);
    match net::http_get(url) {
        Ok(response) => {
            let status_code = response.status_code;
            let redirects_followed = response.redirects_followed;
            let effective_url = response.effective_url.clone();
            serial_println!(
                "[SHELL] curl completed: HTTP {} ({} bytes)",
                status_code,
                response.body.len()
            );
            match output_mode {
                HttpOutputMode::Print => {
                    kprintln!("HTTP {}", status_code);
                    if redirects_followed != 0 {
                        kprintln!(
                            "Redirects: {} -> {}",
                            redirects_followed,
                            effective_url
                        );
                    }
                    print_http_response_headers(&response.headers, response.body.len());
                    kprintln!();
                    print_http_body(&response.headers, &response.body);
                }
                HttpOutputMode::Save(path) => {
                    let body_len = response.body.len();
                    if !http_status_is_success(status_code) {
                        kprint_colored!(Colors::RED, "[ERR]");
                        kprintln!(
                            " download failed: server returned HTTP {} from {}. No file was written.",
                            status_code,
                            effective_url
                        );
                        return;
                    }
                    match fs::write_current_owned(path, response.body) {
                        Ok(saved_path) => {
                            kprint_colored!(Colors::GREEN, "[WarOS] NET: ");
                            kprintln!(
                                "saved {} byte(s) from {} to '{}' (HTTP {}).",
                                body_len,
                                effective_url,
                                fs::display_path(&saved_path),
                                status_code
                            );
                        }
                        Err(error) => report_fs_error(path, error),
                    }
                }
            }
        }
        Err(error) => report_http_request_error(url, &error),
    }
}

fn cmd_ibm(args: &[&str]) {
    let Some(subcommand) = args.first().copied() else {
        kprintln!("Usage: ibm <login|instance|backends|submit>");
        kprintln!("  login <api-key> [service-crn]");
        kprintln!("  instance <service-crn>");
        kprintln!("  backends");
        kprintln!("  submit [backend] [shots]");
        return;
    };

    match subcommand {
        "login" => cmd_ibm_login(&args[1..]),
        "instance" => cmd_ibm_instance(&args[1..]),
        "backends" => cmd_ibm_backends(),
        "submit" => cmd_ibm_submit(&args[1..]),
        _ => {
            kprintln!("Unknown IBM subcommand '{}'.", subcommand);
            kprintln!("Usage: ibm <login|instance|backends|submit>");
        }
    }
}

fn cmd_ibm_login(args: &[&str]) {
    let Some(api_key) = args.first().copied() else {
        kprintln!("Usage: ibm login <api-key> [service-crn]");
        return;
    };

    if let Err(error) = net::ibm::save_api_key(api_key) {
        kprint_colored!(Colors::RED, "[WarOS] ");
        kprintln!("Failed to save IBM API key: {}.", error);
        return;
    }

    if let Some(instance_crn) = args.get(1).copied() {
        if let Err(error) = net::ibm::save_instance_crn(instance_crn) {
            kprint_colored!(Colors::RED, "[WarOS] ");
            kprintln!("Failed to save IBM service CRN: {}.", error);
            return;
        }
        kprint_colored!(Colors::GREEN, "[WarOS] ");
        kprintln!("IBM Quantum credentials saved.");
        return;
    }

    kprint_colored!(Colors::GREEN, "[WarOS] ");
    kprintln!("IBM Quantum API key saved.");
    kprintln!("  Current IBM Runtime access also requires a service CRN.");
    kprintln!("  Set it with: ibm instance <service-crn>");
}

fn cmd_ibm_instance(args: &[&str]) {
    let Some(instance_crn) = args.first().copied() else {
        kprintln!("Usage: ibm instance <service-crn>");
        return;
    };

    match net::ibm::save_instance_crn(instance_crn) {
        Ok(()) => {
            kprint_colored!(Colors::GREEN, "[WarOS] ");
            kprintln!("IBM Quantum service CRN saved.");
        }
        Err(error) => {
            kprint_colored!(Colors::RED, "[WarOS] ");
            kprintln!("Failed to save IBM service CRN: {}.", error);
        }
    }
}

fn cmd_ibm_backends() {
    kprintln!("Querying IBM Quantum backends...");
    match net::ibm::list_backends() {
        Ok(backends) => {
            if backends.is_empty() {
                kprintln!("No IBM backends returned.");
                return;
            }

            kprintln!("Available IBM Quantum backends:");
            for backend in backends {
                let qubits = backend.qubits.unwrap_or_default();
                kprintln!(
                    "  {:<16} {:>3} qubits  {:<8} (queue {})",
                    backend.name,
                    qubits,
                    backend.status.message(),
                    backend.queue_length
                );
            }
        }
        Err(error) => {
            kprint_colored!(Colors::RED, "[WarOS] ");
            kprintln!("Failed to query IBM Runtime: {}.", error);
        }
    }
}

fn cmd_ibm_submit(args: &[&str]) {
    let backend = args.first().copied().unwrap_or("ibm_brisbane");
    let shots = args
        .get(1)
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(1000)
        .clamp(1, 100_000);

    match net::ibm::submit_current_job(backend, shots) {
        Ok(job) => {
            kprintln!(
                "Submitting circuit to {} ({} qubits, {} shots)...",
                job.backend,
                job.backend_qubits,
                shots
            );
            if job.queue_length > 0 {
                kprintln!(
                    "Job queued: job_id={} (backend queue depth {})",
                    job.job_id,
                    job.queue_length
                );
            } else {
                kprintln!("Job queued: job_id={}", job.job_id);
            }
            kprintln!("Waiting for results...");

            let mut last_status = String::new();
            let mut waited_ms = 0u64;
            loop {
                match net::ibm::job_status(&job.job_id) {
                    Ok(status) => {
                        if status.status != last_status {
                            if let Some(reason) = status.reason.as_deref() {
                                kprintln!("Job {}: {} ({})", job.job_id, status.status, reason);
                            } else {
                                kprintln!("Job {}: {}", job.job_id, status.status);
                            }
                            last_status = status.status.clone();
                        }

                        match status.status.as_str() {
                            "Completed" => break,
                            "Queued" | "Running" => {}
                            "Cancelled" | "Cancelled - Ran too long" | "Failed" => {
                                kprint_colored!(Colors::RED, "[WarOS] ");
                                kprintln!(
                                    "IBM job {} ended with status '{}'.",
                                    job.job_id,
                                    status.status
                                );
                                return;
                            }
                            _ => {}
                        }
                    }
                    Err(error) => {
                        kprint_colored!(Colors::RED, "[WarOS] ");
                        kprintln!("Failed to poll IBM job {}: {}.", job.job_id, error);
                        return;
                    }
                }

                if waited_ms >= 600_000 {
                    kprint_colored!(Colors::RED, "[WarOS] ");
                    kprintln!("Timed out waiting for IBM job {}.", job.job_id);
                    return;
                }

                wait_ms(2_000);
                waited_ms = waited_ms.saturating_add(2_000);
            }

            match net::ibm::job_result(&job.job_id, job.output_bits) {
                Ok(result) => {
                    kprintln!();
                    kprintln!("Results from IBM quantum hardware:");
                    print_ibm_histogram(&result);
                    kprintln!();
                    kprintln!("Note: imperfect results are real quantum noise from a superconducting processor.");
                }
                Err(error) => {
                    kprint_colored!(Colors::RED, "[WarOS] ");
                    kprintln!("Failed to fetch IBM job results: {}.", error);
                }
            }
        }
        Err(error) => {
            kprint_colored!(Colors::RED, "[WarOS] ");
            kprintln!("IBM submission failed: {}.", error);
        }
    }
}

struct HttpBodyPreview {
    text: String,
    truncated: bool,
    bytes_shown: usize,
    lines_shown: usize,
}

fn print_http_response_headers(headers: &[(String, String)], body_len: usize) {
    const SUMMARY_HEADERS: [&str; 6] = [
        "Content-Type",
        "Content-Length",
        "Content-Encoding",
        "Cache-Control",
        "ETag",
        "Last-Modified",
    ];

    let mut printed_content_length = false;
    for name in SUMMARY_HEADERS {
        if let Some(value) = header_value(headers, name) {
            kprintln!("{}: {}", name, value);
            if name.eq_ignore_ascii_case("Content-Length") {
                printed_content_length = true;
            }
        }
    }

    if !printed_content_length {
        kprintln!("Body-Bytes: {}", body_len);
    }
}

fn print_http_body(headers: &[(String, String)], body: &[u8]) {
    if body.is_empty() {
        kprintln!("Body: empty.");
        return;
    }

    match decode_http_body_for_display(headers, body) {
        Ok(text) => {
            let preview = preview_text_for_terminal(text.as_ref());
            if preview.truncated {
                kprintln!(
                    "Body preview (showing {} of {} byte(s), {} line(s)):",
                    preview.bytes_shown,
                    body.len(),
                    preview.lines_shown
                );
            } else {
                kprintln!("Body ({} byte(s)):", body.len());
            }

            kprint!("{}", preview.text);
            if !preview.text.ends_with('\n') {
                kprintln!();
            }

            if preview.truncated {
                kprintln!(
                    "[truncated] showing the first {} byte(s) / {} line(s) of terminal-safe text; use 'curl -o <file> <url>' or 'wget' for the full body.",
                    preview.bytes_shown,
                    preview.lines_shown
                );
            }
        }
        Err(reason) => {
            kprint_colored!(Colors::RED, "[ERR]");
            kprintln!(" {}.", reason);
        }
    }
}

fn preview_text_for_terminal(text: &str) -> HttpBodyPreview {
    let mut end = text.len();
    let mut newline_count = 0usize;

    for (index, ch) in text.char_indices() {
        let next = index + ch.len_utf8();
        let next_newline_count = newline_count + usize::from(ch == '\n');
        if next > CURL_PREVIEW_MAX_BYTES || next_newline_count > CURL_PREVIEW_MAX_LINES {
            end = index;
            break;
        }
        newline_count = next_newline_count;
    }

    let truncated = end < text.len();
    let preview_text = if truncated {
        text[..end].to_string()
    } else {
        text.to_string()
    };
    let lines_shown = if preview_text.is_empty() {
        0
    } else {
        preview_text.lines().count().max(1)
    };

    HttpBodyPreview {
        text: preview_text,
        truncated,
        bytes_shown: end.min(text.len()),
        lines_shown,
    }
}

fn decode_http_body_for_display<'a>(
    headers: &[(String, String)],
    body: &'a [u8],
) -> Result<Cow<'a, str>, String> {
    let content_type = response_content_type(headers);
    let declared_charset = response_declared_charset(headers);
    let textual = content_type
        .as_deref()
        .map(is_textual_content_type)
        .unwrap_or(false);
    let non_text_hint =
        "response body is not safe terminal text; use 'curl -o <file> <url>' or 'wget'";

    match declared_charset.as_deref() {
        Some("utf-8" | "utf8") => str::from_utf8(body)
            .map(Cow::Borrowed)
            .map_err(|_| {
                String::from(
                    "response body declares UTF-8 text but contains invalid UTF-8; use 'curl -o <file> <url>' or 'wget'",
                )
            }),
        Some("iso-8859-1" | "latin-1" | "latin1") => {
            if textual {
                Ok(Cow::Owned(decode_latin1_text(body)))
            } else {
                Err(String::from(non_text_hint))
            }
        }
        Some(other) => Err(alloc::format!(
            "response body declares unsupported charset '{}'; use 'curl -o <file> <url>' or 'wget'",
            other
        )),
        None => {
            if let Ok(text) = str::from_utf8(body) {
                if textual || is_probably_text_body(body) {
                    return Ok(Cow::Borrowed(text));
                }
            }
            if should_try_latin1_fallback(content_type.as_deref(), body) {
                return Ok(Cow::Owned(decode_latin1_text(body)));
            }
            Err(String::from(non_text_hint))
        }
    }
}

fn decode_latin1_text(body: &[u8]) -> String {
    let mut decoded = String::with_capacity(body.len());
    for &byte in body {
        decoded.push(char::from(byte));
    }
    decoded
}

fn should_try_latin1_fallback(content_type: Option<&str>, body: &[u8]) -> bool {
    let Some(content_type) = content_type else {
        return false;
    };
    if !is_probably_text_body(body) {
        return false;
    }
    content_type.starts_with("text/") || matches!(content_type, "application/xhtml+xml")
}

fn is_probably_text_body(body: &[u8]) -> bool {
    if body.is_empty() {
        return true;
    }
    let sample = &body[..body.len().min(512)];
    let mut control_bytes = 0usize;
    for &byte in sample {
        if byte == 0 {
            return false;
        }
        if byte < 0x20 && !matches!(byte, b'\n' | b'\r' | b'\t' | 0x0C) {
            control_bytes += 1;
        }
    }
    control_bytes.saturating_mul(20) <= sample.len()
}

fn response_content_type(headers: &[(String, String)]) -> Option<String> {
    header_value(headers, "Content-Type").map(|value| {
        value
            .split(';')
            .next()
            .unwrap_or(value)
            .trim()
            .to_ascii_lowercase()
    })
}

fn response_declared_charset(headers: &[(String, String)]) -> Option<String> {
    let value = header_value(headers, "Content-Type")?;
    for parameter in value.split(';').skip(1) {
        let Some((name, value)) = parameter.split_once('=') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("charset") {
            let charset = value.trim().trim_matches('"').trim_matches('\'');
            if !charset.is_empty() {
                return Some(charset.to_ascii_lowercase());
            }
        }
    }
    None
}

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(header, _)| header.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn is_textual_content_type(content_type: &str) -> bool {
    content_type.starts_with("text/")
        || content_type == "application/json"
        || content_type.ends_with("+json")
        || content_type == "application/javascript"
        || content_type == "application/x-javascript"
        || content_type == "text/javascript"
        || content_type == "application/xml"
        || content_type.ends_with("+xml")
        || content_type == "application/x-www-form-urlencoded"
        || content_type == "image/svg+xml"
}

fn print_ibm_histogram(result: &net::ibm::IBMJobResult) {
    if result.total_shots == 0 || result.counts.is_empty() {
        kprintln!("  No measurement counts returned.");
        return;
    }

    let mut counts = result.counts.clone();
    counts.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    let peak = counts.iter().map(|(_, count)| *count).max().unwrap_or(1);

    for (state, count) in counts {
        let percentage = (count as f64 / result.total_shots as f64) * 100.0;
        let bar_len = ((count as usize) * 24 / peak as usize).max(1);
        let mut bar = String::new();
        for _ in 0..bar_len {
            bar.push('#');
        }
        kprintln!(
            "  |{}> : {:>4} ({:>4.1}%) {}",
            state,
            count,
            percentage,
            bar
        );
    }
}

fn cmd_panic() {
    kprintln!("Triggering test kernel panic...");
    panic!("User-triggered test panic via 'panic' command");
}

fn cmd_reboot(args: &[&str]) {
    if let Err(_) =
        security::capabilities::session_require(security::capabilities::Capabilities::SYS_POWER)
    {
        kprint_colored!(Colors::RED, "[WarOS] ");
        kprintln!("Permission denied. Requires SYS_POWER capability.");
        return;
    }
    if matches!(args.first(), Some(&"recovery")) {
        if let Err(error) = pkg::update::request_recovery("operator requested reboot into recovery")
        {
            kprint_colored!(Colors::RED, "[WarOS] ");
            kprintln!("Failed to request recovery: {}", error);
            return;
        }
        kprintln!("Recovery requested for next boot.");
        serial_println!("Recovery requested for next boot.");
    }
    kprintln!("Rebooting system.");
    kprint_colored!(Colors::DIM, "  note: ");
    kprintln!("WarOS can restart the machine, but firmware boot order decides what boots next.");
    kprint_colored!(Colors::DIM, "  note: ");
    kprintln!("If you are running from removable media, reselect the WarOS USB in the firmware boot menu for the next boot.");
    serial_println!("Rebooting system.");
    hal::acpi::reboot();
}

fn cmd_halt() {
    if let Err(_) =
        security::capabilities::session_require(security::capabilities::Capabilities::SYS_POWER)
    {
        kprint_colored!(Colors::RED, "[WarOS] ");
        kprintln!("Permission denied. Requires SYS_POWER capability.");
        return;
    }
    kprintln!("Shutting down system.");
    serial_println!("Shutting down system.");
    hal::acpi::shutdown();
}

fn cmd_waros() {
    kprintln!(
        "WarOS v{} - Quantum-Classical Hybrid Operating System",
        KERNEL_VERSION
    );
    kprintln!("War Enterprise - Building the future of computing");
    kprintln!("warenterprise.com/waros");
    kprintln!("github.com/WarEnterprise/waros");
}

fn cmd_recovery(args: &[&str]) {
    match args.first().copied() {
        Some("status") | None => {
            kprintln!("{}", pkg::update::status_report());
        }
        Some("enter") => {
            if security::capabilities::session_require(
                security::capabilities::Capabilities::PKG_INSTALL,
            )
            .is_err()
            {
                kprint_colored!(Colors::RED, "[Recovery] ");
                kprintln!("PKG_INSTALL capability required");
                return;
            }
            match pkg::update::request_recovery("operator requested recovery mode") {
                Ok(()) => {
                    kprint_colored!(Colors::GREEN, "[Recovery] ");
                    kprintln!("recovery requested. Use 'reboot recovery' or reboot normally.");
                }
                Err(error) => {
                    kprint_colored!(Colors::RED, "[Recovery] ");
                    kprintln!("{}", error);
                }
            }
        }
        Some("resume") => {
            if security::capabilities::session_require(
                security::capabilities::Capabilities::PKG_INSTALL,
            )
            .is_err()
            {
                kprint_colored!(Colors::RED, "[Recovery] ");
                kprintln!("PKG_INSTALL capability required");
                return;
            }
            match pkg::update::clear_recovery_request() {
                Ok(()) => {
                    kprint_colored!(Colors::GREEN, "[Recovery] ");
                    kprintln!("manual recovery request cleared.");
                }
                Err(error) => {
                    kprint_colored!(Colors::RED, "[Recovery] ");
                    kprintln!("{}", error);
                }
            }
        }
        Some("confirm") => match pkg::update::confirm_pending_update() {
            Ok(transaction) => {
                kprint_colored!(Colors::GREEN, "[Recovery] ");
                kprintln!(
                    "{} {} confirmed. Normal boot may resume.",
                    transaction.package_name,
                    transaction.to_version
                );
            }
            Err(error) => {
                kprint_colored!(Colors::RED, "[Recovery] ");
                kprintln!("{}", error);
            }
        },
        Some("reject") => {
            let reason = args
                .get(1..)
                .filter(|parts| !parts.is_empty())
                .map(|parts| parts.join(" "))
                .unwrap_or_else(|| String::from("operator rejected pending update"));
            match pkg::update::reject_pending_update(&reason) {
                Ok(transaction) => {
                    kprint_colored!(Colors::YELLOW, "[Recovery] ");
                    kprintln!(
                        "{} {} marked failed. Recovery remains active.",
                        transaction.package_name,
                        transaction.to_version
                    );
                }
                Err(error) => {
                    kprint_colored!(Colors::RED, "[Recovery] ");
                    kprintln!("{}", error);
                }
            }
        }
        Some("rollback") => match pkg::update::rollback_current_update() {
            Ok(transaction) => {
                kprint_colored!(Colors::GREEN, "[Recovery] ");
                kprintln!(
                    "{} {} rolled back to the pre-apply state.",
                    transaction.package_name,
                    transaction.to_version
                );
            }
            Err(error) => {
                kprint_colored!(Colors::RED, "[Recovery] ");
                kprintln!("{}", error);
            }
        },
        Some(other) => {
            kprintln!("Unknown subcommand: {}", other);
            kprintln!("Usage: recovery [status|enter|resume|confirm|reject [reason]|rollback]");
        }
    }
}

fn cmd_unknown(command: &str) {
    kprint_colored!(Colors::RED, "[WarOS] ERROR:");
    kprintln!(
        " command '{}' not found. Type 'help' for available commands.",
        command
    );
}

#[derive(Clone, Copy)]
enum HttpOutputMode<'a> {
    Print,
    Save(&'a str),
}

const CURL_PREVIEW_MAX_BYTES: usize = 4 * 1024;
const CURL_PREVIEW_MAX_LINES: usize = 48;

fn require_session_capability(
    capability: security::capabilities::Capabilities,
    capability_name: &str,
    action: &str,
) -> bool {
    if security::capabilities::session_require(capability).is_ok() {
        true
    } else {
        kprint_colored!(Colors::RED, "[WarOS] ");
        kprintln!(
            "Permission denied. '{}' requires {} capability.",
            action,
            capability_name
        );
        false
    }
}

fn persist_ui_preferences_if_allowed(
    capability: security::capabilities::Capabilities,
) -> Result<bool, fs::FsError> {
    if security::capabilities::session_require(capability).is_err() {
        return Ok(false);
    }
    crate::ui::save_preferences().map(|()| true)
}

fn ipv4_lease_status_line(maintenance: &net::NetworkMaintenanceReport) -> String {
    match maintenance.canonical_lease_state {
        "lease-pending" => {
            if maintenance.dhcp_manual_active {
                alloc::format!(
                    "lease pending on the active interface (manual DHCP is in progress with {} ms remaining)",
                    maintenance.dhcp_owner_remaining_ms
                )
            } else if maintenance.dhcp_auto_pump_active {
                alloc::format!(
                    "lease pending on the active interface (auto DHCP pump awaiting {} with {} ms remaining; run 'net dhcp' for full acquisition if needed)",
                    dhcp_wait_label(maintenance.dhcp_auto_pump_waiting_for),
                    maintenance.dhcp_auto_pump_remaining_ms
                )
            } else {
                String::from(
                    "lease pending on the active interface (run 'net dhcp' for full acquisition if needed)",
                )
            }
        }
        state if state.starts_with("timeout-") => {
            alloc::format!("{state} (run 'net dhcp' to retry)")
        }
        _ => String::from("no active IPv4 lease"),
    }
}

fn ipv4_ifconfig_line(maintenance: &net::NetworkMaintenanceReport) -> String {
    match maintenance.canonical_lease_state {
        "lease-pending" => {
            if maintenance.dhcp_manual_active {
                alloc::format!(
                    "pending DHCP acquisition on the active interface (manual DHCP in progress; {} ms remaining)",
                    maintenance.dhcp_owner_remaining_ms
                )
            } else if maintenance.dhcp_auto_pump_active {
                alloc::format!(
                    "pending DHCP acquisition on the active interface (auto pump awaiting {})",
                    dhcp_wait_label(maintenance.dhcp_auto_pump_waiting_for)
                )
            } else {
                String::from("pending DHCP acquisition on the active interface")
            }
        }
        state if state.starts_with("timeout-") => {
            alloc::format!("unconfigured ({state}; run 'net dhcp' to retry)")
        }
        _ => String::from("unconfigured (no DHCP lease - try 'net dhcp' or 'net retry')"),
    }
}

fn dhcp_wait_label(waiting_for: &str) -> &'static str {
    match waiting_for {
        "offer" => "OFFER",
        "ack" => "ACK",
        "discover" => "DISCOVER",
        "lease" => "LEASE",
        "timeout" => "TIMEOUT",
        "offline" => "OFFLINE",
        _ => "none",
    }
}

fn ensure_supported_wired_nic() -> Result<(), &'static str> {
    let Some(hardware) = net::hardware_status() else {
        return Err("no supported active wired NIC is present");
    };

    if hardware.link_state == net::LinkState::Down {
        Err("supported wired NIC is present but the link is down")
    } else {
        Ok(())
    }
}

fn ensure_ip_connectivity_for_host(host: &str) -> Result<net::DhcpConfig, &'static str> {
    ensure_supported_wired_nic()?;
    let config = match net::network_config() {
        Some(config) => config,
        None => {
            let maintenance = net::maintenance_report();
            return Err(match maintenance.canonical_lease_state {
                "lease-pending" if maintenance.dhcp_manual_active => {
                    "active wired NIC is still acquiring an IPv4 lease (manual DHCP command is in progress); wait a moment"
                }
                "lease-pending" if maintenance.dhcp_auto_pump_active => {
                    match maintenance.dhcp_auto_pump_waiting_for {
                        "offer" => "active wired NIC is still acquiring an IPv4 lease (bounded auto DHCP pump is awaiting an OFFER); wait a moment or run 'net dhcp'",
                        "ack" => "active wired NIC is still acquiring an IPv4 lease (bounded auto DHCP pump is awaiting an ACK); wait a moment or run 'net dhcp'",
                        _ => "active wired NIC is still acquiring an IPv4 lease; wait a moment or run 'net dhcp'",
                    }
                }
                "lease-pending" => {
                    "active wired NIC is still acquiring an IPv4 lease; wait a moment or run 'net dhcp'"
                }
                state if state.starts_with("timeout-") => {
                    "active wired NIC did not complete DHCP acquisition; run 'net dhcp' to retry"
                }
                _ => "no active IPv4 lease on the active wired NIC",
            });
        }
    };
    if net::ipv4::Ipv4Addr::parse(host).is_none() && config.dns_server.is_none() {
        return Err("no DNS server is configured for hostname lookups");
    }
    Ok(config)
}

fn ensure_http_request_ready(url: &str) -> Result<(), String> {
    let parts = net::http::parse_url(url).map_err(|error| error.to_string())?;
    ensure_ip_connectivity_for_host(&parts.host).map_err(String::from)?;
    Ok(())
}

fn parse_wget_args<'a>(args: &'a [&'a str]) -> Result<(&'a str, Option<&'a str>), ()> {
    match args {
        [url] => Ok((*url, None)),
        [url, output] => Ok((*url, Some(*output))),
        ["-O" | "-o", output, url] => Ok((*url, Some(*output))),
        [url, "-O" | "-o", output] => Ok((*url, Some(*output))),
        _ => Err(()),
    }
}

fn parse_curl_args<'a>(args: &'a [&'a str]) -> Result<(&'a str, HttpOutputMode<'a>), ()> {
    match args {
        [url] => Ok((*url, HttpOutputMode::Print)),
        ["-o", output, url] => Ok((*url, HttpOutputMode::Save(*output))),
        [url, "-o", output] => Ok((*url, HttpOutputMode::Save(*output))),
        _ => Err(()),
    }
}

struct DerivedDownloadPath {
    path: String,
    used_fallback: bool,
}

fn default_download_path(
    url: &str,
    headers: &[(String, String)],
) -> Result<DerivedDownloadPath, &'static str> {
    let parts = net::http::parse_url(url).map_err(|_| "download URL is invalid")?;
    let path = strip_url_path_suffixes(parts.path.as_str());
    let filename = fs::basename(path);
    if filename != "." && filename != ".." && !filename.is_empty() && filename != "/" {
        return Ok(DerivedDownloadPath {
            path: filename.to_string(),
            used_fallback: false,
        });
    }

    let host = sanitize_download_component(&parts.host);
    let extension = preferred_download_extension(headers);
    let stem = if path.ends_with('/') || filename.is_empty() || filename == "/" {
        "index"
    } else {
        "download"
    };
    Ok(DerivedDownloadPath {
        path: alloc::format!("{host}.{stem}.{extension}"),
        used_fallback: true,
    })
}

fn strip_url_path_suffixes(path: &str) -> &str {
    let path = path.split('?').next().unwrap_or(path);
    path.split('#').next().unwrap_or(path)
}

fn sanitize_download_component(component: &str) -> String {
    let mut sanitized = String::with_capacity(component.len());
    let mut previous_was_separator = false;
    for ch in component.chars() {
        let mapped = if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
            previous_was_separator = false;
            ch
        } else if previous_was_separator {
            continue;
        } else {
            previous_was_separator = true;
            '_'
        };
        sanitized.push(mapped);
    }
    let sanitized = sanitized.trim_matches(|ch| ch == '.' || ch == '_').to_string();
    if sanitized.is_empty() || sanitized == "." || sanitized == ".." {
        String::from("download")
    } else {
        sanitized
    }
}

fn preferred_download_extension(headers: &[(String, String)]) -> &'static str {
    match response_content_type(headers).as_deref() {
        Some("text/html") | Some("application/xhtml+xml") => "html",
        Some("text/plain") => "txt",
        Some("text/css") => "css",
        Some("application/javascript")
        | Some("application/x-javascript")
        | Some("text/javascript") => "js",
        Some("application/json") => "json",
        Some(content_type) if content_type.ends_with("+json") => "json",
        Some("application/xml") | Some("image/svg+xml") => "xml",
        Some(content_type) if content_type.ends_with("+xml") => "xml",
        Some("application/x-www-form-urlencoded") => "txt",
        _ => "bin",
    }
}

fn http_status_is_success(status_code: u16) -> bool {
    (200..=299).contains(&status_code)
}

fn report_http_request_error(url: &str, error: &net::NetError) {
    serial_println!("[SHELL] http request failed: {} -> {}", url, error);
    kprint_colored!(Colors::RED, "[ERR]");
    match error {
        net::NetError::ProtocolError(message)
            if message.starts_with("TLS ")
                || message.contains("certificate")
                || message.contains("InvalidSignatureScheme")
                || message.contains("DecodeError")
                || message.contains("decode-error")
                || message.contains("handshake") =>
        {
            kprintln!(" TLS failure: {}.", error);
            if message.contains("InvalidSignatureScheme") {
                kprintln!(
                    " The server offered a signature scheme outside the currently validated embedded host policy."
                );
            }
            if url.starts_with("https://") {
                kprintln!(
                    " HTTPS support is currently limited to {} and uses embedded roots with hostname validation and no RTC-backed expiry check.",
                    net::tls::supported_hosts_summary()
                );
            }
        }
        net::NetError::ProtocolError(message) if message.starts_with("HTTP ") => {
            kprintln!(" HTTP failure: {}.", error);
        }
        _ => {
            kprintln!(" network failure: {}.", error);
        }
    }
}

fn format_optional_ipv4(value: Option<net::ipv4::Ipv4Addr>) -> String {
    value
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| String::from("none"))
}

fn firmware_search_roots_summary() -> String {
    let mut summary = String::new();
    for root in hal::net::firmware::firmware_search_roots() {
        if !summary.is_empty() {
            summary.push_str(", ");
        }
        summary.push_str(root);
    }
    summary
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
        fs::FsError::FileTooLarge => {
            kprint_colored!(Colors::RED, "[ERR]");
            kprintln!(
                " {} No partial file was written; use a smaller response or wait for a future streaming download path.",
                error
            );
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

fn process_state_name(state: exec::process::ProcessState) -> &'static str {
    match state {
        exec::process::ProcessState::Ready => "ready",
        exec::process::ProcessState::Running => "running",
        exec::process::ProcessState::Blocked => "blocked",
        exec::process::ProcessState::Stopped => "stopped",
        exec::process::ProcessState::Zombie => "zombie",
    }
}

fn priority_name(priority: exec::process::Priority) -> &'static str {
    match priority {
        exec::process::Priority::RealTime => "rt",
        exec::process::Priority::System => "sys",
        exec::process::Priority::Quantum => "quant",
        exec::process::Priority::Interactive => "int",
        exec::process::Priority::Normal => "norm",
        exec::process::Priority::Batch => "batch",
        exec::process::Priority::Idle => "idle",
    }
}

fn image_kind_name(kind: exec::process::ProcessImageKind) -> &'static str {
    match kind {
        exec::process::ProcessImageKind::KernelShell => "shell",
        exec::process::ProcessImageKind::ShellCommand => "cmd",
        exec::process::ProcessImageKind::ShellScript => "script",
        exec::process::ProcessImageKind::Elf => "elf",
    }
}

fn parse_priority(text: &str) -> Option<exec::process::Priority> {
    match text {
        "rt" | "realtime" => Some(exec::process::Priority::RealTime),
        "sys" | "system" => Some(exec::process::Priority::System),
        "quant" | "quantum" => Some(exec::process::Priority::Quantum),
        "int" | "interactive" => Some(exec::process::Priority::Interactive),
        "norm" | "normal" => Some(exec::process::Priority::Normal),
        "batch" => Some(exec::process::Priority::Batch),
        "idle" => Some(exec::process::Priority::Idle),
        _ => None,
    }
}

struct CpuInspection {
    vendor: String,
    brand: Option<String>,
    family: u32,
    model: u32,
    stepping: u32,
    max_basic_leaf: u32,
    max_extended_leaf: u32,
    topology: CpuTopology,
    frequency_mhz: Option<(u32, u32, u32)>,
    cache_summary: Option<String>,
    address_bits: Option<(u8, u8)>,
    feature_summary: Option<String>,
    hypervisor_present: bool,
}

#[derive(Clone, Copy)]
struct CpuTopology {
    logical_processors: u32,
    threads_per_core: u32,
    cores_per_package: u32,
    apic_id: u32,
}

fn inspect_cpu() -> CpuInspection {
    let vendor_leaf = __cpuid(0);
    let feature_leaf = __cpuid(1);
    let max_basic_leaf = vendor_leaf.eax;
    let max_extended_leaf = __cpuid(0x8000_0000).eax;
    let vendor_bytes = vendor_string_bytes(vendor_leaf.ebx, vendor_leaf.edx, vendor_leaf.ecx);
    let vendor = str::from_utf8(&vendor_bytes)
        .unwrap_or("Unknown")
        .to_string();

    CpuInspection {
        vendor,
        brand: cpu_brand_string(max_extended_leaf),
        family: cpu_family(feature_leaf.eax),
        model: cpu_model(feature_leaf.eax),
        stepping: feature_leaf.eax & 0x0F,
        max_basic_leaf,
        max_extended_leaf,
        topology: cpu_topology(max_basic_leaf, feature_leaf),
        frequency_mhz: cpu_frequency_mhz(max_basic_leaf),
        cache_summary: cpu_cache_summary(max_basic_leaf),
        address_bits: cpu_address_bits(max_extended_leaf),
        feature_summary: cpu_feature_summary(max_basic_leaf, max_extended_leaf, feature_leaf),
        hypervisor_present: feature_leaf.ecx & (1 << 31) != 0,
    }
}

fn cpu_brand_string(max_extended_leaf: u32) -> Option<String> {
    if max_extended_leaf < 0x8000_0004 {
        return None;
    }

    let mut bytes = [0u8; 48];
    for (index, leaf) in (0x8000_0002..=0x8000_0004).enumerate() {
        let result = __cpuid(leaf);
        let offset = index * 16;
        bytes[offset..offset + 4].copy_from_slice(&result.eax.to_le_bytes());
        bytes[offset + 4..offset + 8].copy_from_slice(&result.ebx.to_le_bytes());
        bytes[offset + 8..offset + 12].copy_from_slice(&result.ecx.to_le_bytes());
        bytes[offset + 12..offset + 16].copy_from_slice(&result.edx.to_le_bytes());
    }

    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    let brand = str::from_utf8(&bytes[..end]).ok()?.trim();
    if brand.is_empty() {
        None
    } else {
        Some(brand.to_string())
    }
}

fn cpu_topology(max_basic_leaf: u32, feature_leaf: core::arch::x86_64::CpuidResult) -> CpuTopology {
    let mut logical_processors = ((feature_leaf.ebx >> 16) & 0xFF).max(1);
    let mut threads_per_core = 1u32;
    let mut apic_id = (feature_leaf.ebx >> 24) & 0xFF;

    let topology_leaf = if max_basic_leaf >= 0x1F {
        Some(0x1F)
    } else if max_basic_leaf >= 0x0B {
        Some(0x0B)
    } else {
        None
    };

    if let Some(leaf) = topology_leaf {
        for subleaf in 0..8u32 {
            let level = __cpuid_count(leaf, subleaf);
            let level_logicals = level.ebx & 0xFFFF;
            let level_type = (level.ecx >> 8) & 0xFF;
            if level_logicals == 0 || level_type == 0 {
                break;
            }
            match level_type {
                1 => threads_per_core = level_logicals.max(1),
                2 => logical_processors = level_logicals.max(threads_per_core),
                _ => {}
            }
            apic_id = level.edx;
        }
    }

    CpuTopology {
        logical_processors,
        threads_per_core,
        cores_per_package: (logical_processors / threads_per_core.max(1)).max(1),
        apic_id,
    }
}

fn cpu_frequency_mhz(max_basic_leaf: u32) -> Option<(u32, u32, u32)> {
    if max_basic_leaf < 0x16 {
        return None;
    }

    let frequency = __cpuid(0x16);
    if frequency.eax == 0 && frequency.ebx == 0 && frequency.ecx == 0 {
        None
    } else {
        Some((
            frequency.eax & 0xFFFF,
            frequency.ebx & 0xFFFF,
            frequency.ecx & 0xFFFF,
        ))
    }
}

fn cpu_cache_summary(max_basic_leaf: u32) -> Option<String> {
    if max_basic_leaf < 0x04 {
        return None;
    }

    let mut parts = Vec::new();
    for subleaf in 0..8u32 {
        let leaf = __cpuid_count(0x04, subleaf);
        let cache_type = leaf.eax & 0x1F;
        if cache_type == 0 {
            break;
        }

        let level = (leaf.eax >> 5) & 0x7;
        let ways = ((leaf.ebx >> 22) & 0x3FF) + 1;
        let partitions = ((leaf.ebx >> 12) & 0x3FF) + 1;
        let line_size = (leaf.ebx & 0xFFF) + 1;
        let sets = leaf.ecx + 1;
        let size_kib = ways
            .saturating_mul(partitions)
            .saturating_mul(line_size)
            .saturating_mul(sets)
            / 1024;
        parts.push(alloc::format!(
            "L{}{} {} KiB",
            level,
            cpu_cache_type_suffix(cache_type),
            size_kib
        ));
    }

    (!parts.is_empty()).then(|| parts.join(", "))
}

fn cpu_cache_type_suffix(cache_type: u32) -> &'static str {
    match cache_type {
        1 => "d",
        2 => "i",
        3 => "",
        _ => "?",
    }
}

fn cpu_address_bits(max_extended_leaf: u32) -> Option<(u8, u8)> {
    if max_extended_leaf < 0x8000_0008 {
        return None;
    }

    let leaf = __cpuid(0x8000_0008);
    Some(((leaf.eax & 0xFF) as u8, ((leaf.eax >> 8) & 0xFF) as u8))
}

fn cpu_feature_summary(
    max_basic_leaf: u32,
    max_extended_leaf: u32,
    feature_leaf: core::arch::x86_64::CpuidResult,
) -> Option<String> {
    let mut features = Vec::new();
    let mut push_feature = |enabled: bool, label: &'static str| {
        if enabled {
            features.push(label);
        }
    };

    push_feature(feature_leaf.edx & (1 << 23) != 0, "MMX");
    push_feature(feature_leaf.edx & (1 << 25) != 0, "SSE");
    push_feature(feature_leaf.edx & (1 << 26) != 0, "SSE2");
    push_feature(feature_leaf.ecx & (1 << 0) != 0, "SSE3");
    push_feature(feature_leaf.ecx & (1 << 9) != 0, "SSSE3");
    push_feature(feature_leaf.ecx & (1 << 19) != 0, "SSE4.1");
    push_feature(feature_leaf.ecx & (1 << 20) != 0, "SSE4.2");
    push_feature(feature_leaf.ecx & (1 << 21) != 0, "x2APIC");
    push_feature(feature_leaf.ecx & (1 << 25) != 0, "AES-NI");
    push_feature(feature_leaf.ecx & (1 << 26) != 0, "XSAVE");
    push_feature(feature_leaf.ecx & (1 << 28) != 0, "AVX");
    push_feature(feature_leaf.edx & (1 << 28) != 0, "HTT");

    if max_basic_leaf >= 0x07 {
        let leaf7 = __cpuid_count(0x07, 0);
        push_feature(leaf7.ebx & (1 << 0) != 0, "FSGSBASE");
        push_feature(leaf7.ebx & (1 << 3) != 0, "BMI1");
        push_feature(leaf7.ebx & (1 << 5) != 0, "AVX2");
        push_feature(leaf7.ebx & (1 << 8) != 0, "BMI2");
        push_feature(leaf7.ebx & (1 << 18) != 0, "RDSEED");
        push_feature(leaf7.ebx & (1 << 29) != 0, "SHA");
    }

    if max_extended_leaf >= 0x8000_0001 {
        let ext = __cpuid(0x8000_0001);
        push_feature(ext.edx & (1 << 20) != 0, "NX");
        push_feature(ext.edx & (1 << 29) != 0, "LM");
        push_feature(ext.ecx & (1 << 5) != 0, "LZCNT");
    }

    (!features.is_empty()).then(|| features.join(" "))
}

fn mib_from_frames(frames: usize) -> usize {
    (frames * 4) / 1024
}

fn percentage(numerator: usize, denominator: usize) -> usize {
    if denominator == 0 {
        0
    } else {
        numerator.saturating_mul(100) / denominator
    }
}

fn format_memory_mib(mib: usize) -> String {
    if mib >= 1024 {
        alloc::format!("{mib} MiB ({} GiB)", mib / 1024)
    } else {
        alloc::format!("{mib} MiB")
    }
}

fn cpu_topology_summary(cpu: &CpuInspection) -> String {
    alloc::format!(
        "logical={} cores/package={} threads/core={}",
        cpu.topology.logical_processors,
        cpu.topology.cores_per_package,
        cpu.topology.threads_per_core
    )
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

fn wait_ms(duration_ms: u64) {
    let mut remaining = duration_ms.saturating_mul(u64::from(PIT_FREQUENCY_HZ)) / 1_000;
    remaining = remaining.max(1);
    while remaining > 0 {
        let _ = net::poll();
        remaining = remaining.saturating_sub(wait_for_runtime_tick());
    }
}

fn runtime_ticks() -> u64 {
    interrupts::tick_count()
}

fn wait_for_runtime_tick() -> u64 {
    let start_tick = interrupts::tick_count();
    if crate::arch::x86_64::pit::wait_for_tick_advance(start_tick, 1) {
        interrupts::tick_count().saturating_sub(start_tick).max(1)
    } else {
        1
    }
}

fn print_timer_note(runtime_ticks: u64) {
    let irq_ticks = interrupts::irq_tick_count();
    if irq_ticks < runtime_ticks {
        kprint_colored!(Colors::YELLOW, "Timer note: ");
        kprintln!(
            "monotonic uptime is using PIT-observed fallback; raw IRQ ticks={}.",
            irq_ticks
        );
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NetworkInventoryState {
    SupportedActive,
    SupportedDetected,
    UnsupportedDetected,
    WifiProbeOnly,
}

fn network_capabilities(device: &hal::HardwareDevice) -> Option<&hal::device::NetworkCapabilities> {
    match &device.info.capabilities {
        hal::DeviceCapabilities::Network(capabilities) => Some(capabilities),
        _ => None,
    }
}

fn classify_network_inventory(device: &hal::HardwareDevice) -> Option<NetworkInventoryState> {
    let capabilities = network_capabilities(device)?;
    if device.status == hal::DeviceStatus::Active {
        return Some(NetworkInventoryState::SupportedActive);
    }
    if capabilities.has_wifi {
        return Some(NetworkInventoryState::WifiProbeOnly);
    }
    if capabilities.has_ethernet
        && hal::net::nic_select::supported_wired_driver(
            device.info.vendor_id,
            device.info.product_id,
        )
        .is_some()
    {
        return Some(NetworkInventoryState::SupportedDetected);
    }
    capabilities
        .has_ethernet
        .then_some(NetworkInventoryState::UnsupportedDetected)
}

fn network_inventory_label(state: NetworkInventoryState) -> &'static str {
    match state {
        NetworkInventoryState::SupportedActive => "active",
        NetworkInventoryState::SupportedDetected => "supported",
        NetworkInventoryState::UnsupportedDetected => "unsupported",
        NetworkInventoryState::WifiProbeOnly => "wireless-det",
    }
}

fn network_expected_path(device: &hal::HardwareDevice) -> &str {
    match classify_network_inventory(device) {
        Some(NetworkInventoryState::SupportedActive) => device.driver.name(),
        Some(NetworkInventoryState::SupportedDetected) => {
            hal::net::nic_select::supported_wired_driver(
                device.info.vendor_id,
                device.info.product_id,
            )
            .unwrap_or("supported")
        }
        Some(NetworkInventoryState::UnsupportedDetected) => "unsupported",
        Some(NetworkInventoryState::WifiProbeOnly) => "wireless-class (no driver)",
        None => device.driver.name(),
    }
}

fn network_link_summary(info: &net::NetworkDeviceInfo) -> String {
    match info.link_state {
        net::LinkState::Up => alloc::format!(
            "up {} Mbps {} duplex",
            info.link_speed_mbps,
            if info.full_duplex { "full" } else { "half" }
        ),
        net::LinkState::Down => String::from("down"),
        net::LinkState::Unknown => String::from("unknown"),
    }
}

fn print_network_inventory_section(title: &str, devices: &[hal::HardwareDevice]) {
    if devices.is_empty() {
        return;
    }
    kprintln!("{}", title);
    for device in devices {
        let state = classify_network_inventory(device)
            .map(network_inventory_label)
            .unwrap_or("unknown");
        let vendor_label = pci_vendor_name(device.info.vendor_id);
        kprintln!(
            "    {:<12} {:<12} {:<18} {:04X}:{:04X} {} [{}]",
            format_bus_location(&device.info.bus),
            state,
            network_expected_path(device),
            device.info.vendor_id,
            device.info.product_id,
            device.info.name,
            vendor_label
        );
    }
}

fn pci_vendor_name(vendor_id: u16) -> &'static str {
    match vendor_id {
        0x8086 => "Intel",
        0x10EC => "Realtek",
        0x14E4 => "Broadcom",
        0x168C => "Qualcomm Atheros",
        0x1AF4 => "Red Hat (virtio)",
        0x10DE => "NVIDIA",
        0x1002 => "AMD/ATI",
        0x1969 => "Qualcomm Atheros (Attansic)",
        0x11AB => "Marvell",
        0x15B3 => "Mellanox",
        0x1077 => "QLogic",
        0x8088 => "Intel (alt)",
        _ => "unknown vendor",
    }
}

fn boot_path_summary() -> &'static str {
    "bootloader handoff (firmware selects the next boot target)"
}

fn format_bus_location(location: &hal::BusLocation) -> String {
    match location {
        hal::BusLocation::Pci {
            bus,
            device,
            function,
        } => alloc::format!("PCI {:02X}:{:02X}.{}", bus, device, function),
        hal::BusLocation::Usb {
            controller: _,
            port,
            address,
        } => alloc::format!("Port {} Addr {}", port, address),
        hal::BusLocation::Platform => String::from("Platform"),
        hal::BusLocation::Virtual => String::from("Virtual"),
    }
}

fn format_pci_bar(bar: net::pci::PciBar) -> String {
    match bar {
        net::pci::PciBar::Io(base) => alloc::format!("io:0x{:04X}", base),
        net::pci::PciBar::Memory32(base) => alloc::format!("mmio32:0x{:08X}", base),
        net::pci::PciBar::Memory64(base) => alloc::format!("mmio64:0x{:016X}", base),
        net::pci::PciBar::Unused => String::from("none"),
    }
}

fn format_pci_bars(device: &net::pci::PciDevice) -> String {
    let mut bars = Vec::new();
    let mut index = 0usize;
    while index < device.bars.len() {
        let bar = device.bar(index);
        if !matches!(bar, net::pci::PciBar::Unused) {
            bars.push(alloc::format!("BAR{}={}", index, format_pci_bar(bar)));
        }
        index += match bar {
            net::pci::PciBar::Memory64(_) => 2,
            _ => 1,
        };
    }

    if bars.is_empty() {
        String::from("none")
    } else {
        bars.join(" | ")
    }
}

fn format_bus_and_ids(device: &hal::HardwareDevice) -> String {
    alloc::format!(
        "{} {:04X}:{:04X}",
        format_bus_location(&device.info.bus),
        device.info.vendor_id,
        device.info.product_id
    )
}

fn summarize_device_capabilities(device: &hal::HardwareDevice) -> String {
    match &device.info.capabilities {
        hal::DeviceCapabilities::Network(cap) => alloc::format!(
            "ethernet={} wifi={} max={} Mbps",
            cap.has_ethernet,
            cap.has_wifi,
            cap.max_speed_mbps
        ),
        hal::DeviceCapabilities::Storage(cap) => alloc::format!(
            "{} MiB sector={} removable={} ro={} trim={}",
            cap.capacity_bytes / (1024 * 1024),
            cap.sector_size,
            cap.is_removable,
            cap.is_readonly,
            cap.supports_trim
        ),
        hal::DeviceCapabilities::Display(cap) => {
            alloc::format!("{}x{} {}bpp", cap.width, cap.height, cap.bpp)
        }
        hal::DeviceCapabilities::Input(cap) => alloc::format!(
            "kbd={} ptr={} touch={} layout={}",
            cap.has_keyboard,
            cap.has_pointer,
            cap.has_touch,
            cap.layout.short_name()
        ),
        hal::DeviceCapabilities::Usb(cap) => alloc::format!(
            "speed={} class={:02X}/{:02X}/{:02X} ep={}",
            usb_speed_name(cap.speed),
            cap.class,
            cap.subclass,
            cap.protocol,
            cap.num_endpoints
        ),
        hal::DeviceCapabilities::Quantum(cap) => {
            alloc::format!("{} qubits simulator={}", cap.num_qubits, cap.is_simulator)
        }
        hal::DeviceCapabilities::None => String::from("generic"),
    }
}

fn hal_device_for_pci(pci: &net::pci::PciDevice) -> Option<hal::HardwareDevice> {
    hal::devices().into_iter().find(|device| {
        matches!(
            device.info.bus,
            hal::BusLocation::Pci {
                bus,
                device,
                function
            } if bus == pci.bus && device == pci.device && function == pci.function
        )
    })
}

fn usb_speed_name(speed: hal::device::UsbSpeed) -> &'static str {
    match speed {
        hal::device::UsbSpeed::Low => "low",
        hal::device::UsbSpeed::Full => "full",
        hal::device::UsbSpeed::High => "high",
        hal::device::UsbSpeed::Super => "super",
        hal::device::UsbSpeed::SuperPlus => "super+",
        hal::device::UsbSpeed::Super2x2 => "super2x2",
    }
}

fn cmd_env() {
    let env = ENV.lock();
    for (key, value) in env.iter() {
        kprintln!("{}={}", key, value);
    }
}

fn cmd_export(command_line: &str) {
    let rest = command_line
        .split_once(char::is_whitespace)
        .map_or("", |(_, r)| r)
        .trim();
    if rest.is_empty() {
        cmd_env();
        return;
    }
    if let Some((key, value)) = rest.split_once('=') {
        ENV.lock().insert(key.trim().to_string(), value.to_string());
    } else {
        kprintln!("Usage: export KEY=VALUE");
    }
}

fn cmd_unset(args: &[&str]) {
    for &key in args {
        ENV.lock().remove(key);
    }
}

fn cmd_alias(command_line: &str) {
    let rest = command_line
        .split_once(char::is_whitespace)
        .map_or("", |(_, r)| r)
        .trim();
    if rest.is_empty() {
        let aliases = ALIASES.lock();
        for (name, value) in aliases.iter() {
            kprintln!("alias {}='{}'", name, value);
        }
        return;
    }
    if let Some((name, value)) = rest.split_once('=') {
        let value = value.trim_matches('\'').trim_matches('"');
        ALIASES
            .lock()
            .insert(name.trim().to_string(), value.to_string());
    } else {
        kprintln!("Usage: alias name=command");
    }
}

fn cmd_unalias(args: &[&str]) {
    for &name in args {
        ALIASES.lock().remove(name);
    }
}

// ─── WarShield Security Commands ──────────────────────────────────────────

fn cmd_security(args: &[&str]) {
    match args.first().copied() {
        Some("status") | None => {
            kprintln!("{}", security::format_status());
        }
        Some("profile") => {
            if let Some(name) = args.get(1).copied() {
                if let Some(profile) = security::policy::profiles::SecurityProfile::from_name(name)
                {
                    if security::capabilities::session_require(
                        security::capabilities::Capabilities::SECURITY_ADMIN,
                    )
                    .is_err()
                    {
                        kprint_colored!(Colors::RED, "Error: ");
                        kprintln!("SECURITY_ADMIN capability required");
                        return;
                    }
                    security::policy::profiles::apply(profile);
                    kprint_colored!(Colors::GREEN, "Security profile applied: ");
                    kprintln!("{} — {}", profile.name(), profile.description());
                } else {
                    kprintln!("Unknown profile. Available: minimal, standard, server, paranoid");
                }
            } else {
                let current = security::policy::profiles::current();
                kprintln!("Current: {} — {}", current.name(), current.description());
                kprintln!("Available profiles:");
                for profile in [
                    security::policy::profiles::SecurityProfile::Minimal,
                    security::policy::profiles::SecurityProfile::Standard,
                    security::policy::profiles::SecurityProfile::Server,
                    security::policy::profiles::SecurityProfile::Paranoid,
                ] {
                    kprintln!(
                        "  {:<8} — {}",
                        profile.name().to_ascii_lowercase(),
                        profile.description()
                    );
                }
            }
        }
        Some(other) => {
            kprintln!("Unknown subcommand: {}", other);
            kprintln!("Usage: security [status|profile <name>]");
        }
    }
}

fn cmd_capabilities(args: &[&str]) {
    match args.first().copied() {
        None => {
            let pid = exec::current_pid().unwrap_or(0);
            let caps = security::capabilities::current_capabilities();
            let session_caps =
                security::capabilities::session_capabilities_for_uid(auth::session::current_uid());
            if let Some(caps) = caps {
                kprint_colored!(Colors::CYAN, "Process Capabilities");
                kprintln!(" (pid {})", pid);
                kprintln!(
                    "  Session user: {} (role={})",
                    auth::session::current_username(),
                    auth::session::current_role().as_str()
                );
                kprintln!("{}", security::capabilities::format_capabilities(caps));
                if caps != session_caps {
                    kprint_colored!(
                        Colors::DIM,
                        "  Session baseline differs from the live process capability set.\n"
                    );
                }
                kprintln!("  Drops are one-way for the current process under the current spawn/exec model.");
            } else {
                kprint_colored!(Colors::CYAN, "Session Capabilities\n");
                kprintln!(
                    "  Session user: {} (role={})",
                    auth::session::current_username(),
                    auth::session::current_role().as_str()
                );
                kprintln!(
                    "{}",
                    security::capabilities::format_capabilities(session_caps)
                );
            }
        }
        Some("drop") => {
            if args.len() < 2 {
                kprintln!("Usage: capabilities drop <CAP> [CAP...]");
                return;
            }
            let mut to_drop = security::capabilities::Capabilities::empty();
            for name in &args[1..] {
                let Some(capability) = security::capabilities::parse_capability(name) else {
                    kprintln!("Unknown capability '{}'.", name);
                    kprintln!(
                        "Known capabilities: {}",
                        security::capabilities::all_capability_names().join(", ")
                    );
                    return;
                };
                to_drop |= capability;
            }
            security::capabilities::drop_capabilities(to_drop);
            kprint_colored!(Colors::GREEN, "Dropped: ");
            kprintln!("{:?}", to_drop);
            if let Some(caps) = security::capabilities::current_capabilities() {
                kprintln!("{}", security::capabilities::format_capabilities(caps));
            }
        }
        Some(other) => {
            kprintln!("Unknown subcommand: {}", other);
            kprintln!("Usage: capabilities [drop <CAP> [CAP...]]");
        }
    }
}

fn cmd_audit(args: &[&str]) {
    match args.first().copied() {
        Some("log") => {
            let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(20);
            let events = security::audit::last_n(n);
            if events.is_empty() {
                kprintln!("No audit events recorded.");
                return;
            }
            kprint_colored!(Colors::CYAN, "WarAudit Log");
            kprintln!(" (last {})", n);
            for (id, _ts, desc) in &events {
                kprint_colored!(Colors::DIM, "  #{:<5} ", id);
                kprintln!("{}", desc);
            }
        }
        Some("stats") => {
            kprint_colored!(Colors::CYAN, "WarAudit Statistics\n");
            kprintln!("  Total events: {}", security::audit::total_count());
            let stats = security::audit::stats();
            for (category, count) in &stats {
                kprintln!("  {:<12} {}", category, count);
            }
            let (ok, fail) = security::audit::auth_stats();
            kprintln!("  Auth OK: {}, Failed: {}", ok, fail);
        }
        _ => {
            kprintln!("Usage: audit log [n] | audit stats");
        }
    }
}

fn cmd_firewall(args: &[&str]) {
    match args.first().copied() {
        Some("status") | None => {
            kprint_colored!(Colors::CYAN, "WarGuard Firewall\n");
            kprintln!("{}", security::firewall::format_status());
        }
        Some("rules") => {
            kprint_colored!(Colors::CYAN, "Firewall Rules:\n");
            kprint!("{}", security::firewall::format_rules());
        }
        Some("add") => {
            // firewall add allow|deny in|out tcp|udp|icmp [port]
            if args.len() < 5 {
                kprintln!("Usage: firewall add allow|deny in|out tcp|udp|icmp [port]");
                return;
            }
            if security::capabilities::require_capability(
                security::capabilities::Capabilities::NET_ADMIN,
            )
            .is_err()
            {
                kprint_colored!(Colors::RED, "Error: ");
                kprintln!("NET_ADMIN capability required");
                return;
            }
            let action = match args[1] {
                "allow" => security::firewall::rules::Action::Allow,
                "deny" => security::firewall::rules::Action::Deny,
                _ => {
                    kprintln!("Invalid action: use allow or deny");
                    return;
                }
            };
            let direction = match args[2] {
                "in" => security::firewall::rules::Direction::Inbound,
                "out" => security::firewall::rules::Direction::Outbound,
                _ => {
                    kprintln!("Invalid direction: use in or out");
                    return;
                }
            };
            let protocol = match args[3] {
                "tcp" => security::firewall::rules::Protocol::Tcp,
                "udp" => security::firewall::rules::Protocol::Udp,
                "icmp" => security::firewall::rules::Protocol::Icmp,
                "any" => security::firewall::rules::Protocol::Any,
                _ => {
                    kprintln!("Invalid protocol: use tcp, udp, icmp, or any");
                    return;
                }
            };
            let port = args.get(4).and_then(|s| s.parse::<u16>().ok());
            let desc = alloc::format!(
                "User rule: {} {} {} port {:?}",
                action,
                direction,
                protocol,
                port
            );
            let id = security::firewall::add_rule(direction, protocol, port, action, desc);
            kprint_colored!(Colors::GREEN, "Rule added: ");
            kprintln!("id={}", id);
        }
        Some("remove") => {
            if let Some(id) = args.get(1).and_then(|s| s.parse::<u32>().ok()) {
                if security::capabilities::require_capability(
                    security::capabilities::Capabilities::NET_ADMIN,
                )
                .is_err()
                {
                    kprint_colored!(Colors::RED, "Error: ");
                    kprintln!("NET_ADMIN capability required");
                    return;
                }
                if security::firewall::remove_rule(id) {
                    kprintln!("Rule {} removed", id);
                } else {
                    kprintln!("Rule {} not found", id);
                }
            } else {
                kprintln!("Usage: firewall remove <id>");
            }
        }
        Some("log") => {
            // Show recent firewall audit events
            let events = security::audit::last_n(50);
            kprint_colored!(Colors::CYAN, "Firewall Log:\n");
            let mut found = false;
            for (id, _ts, desc) in &events {
                if desc.starts_with("FW_MATCH") || desc.starts_with("NET_CONN") {
                    kprint_colored!(Colors::DIM, "  #{:<5} ", id);
                    kprintln!("{}", desc);
                    found = true;
                }
            }
            if !found {
                kprintln!("  No firewall events recorded.");
            }
        }
        Some(other) => {
            kprintln!("Unknown subcommand: {}", other);
            kprintln!("Usage: firewall [status|rules|add|remove|log]");
        }
    }
}

fn cmd_integrity(args: &[&str]) {
    match args.first().copied() {
        Some("build") => {
            let count = security::vault::build_database();
            kprint_colored!(Colors::GREEN, "Integrity database built: ");
            kprintln!("{} files hashed", count);
        }
        Some("check") => {
            let violations = security::vault::verify_all();
            if violations.is_empty() {
                kprint_colored!(Colors::GREEN, "All monitored files intact.\n");
            } else {
                kprint_colored!(Colors::RED, "INTEGRITY VIOLATIONS DETECTED:\n");
                for v in &violations {
                    kprintln!(
                        "  {} — expected: {}... actual: {}...",
                        v.path,
                        &v.expected[..16.min(v.expected.len())],
                        &v.actual[..16.min(v.actual.len())],
                    );
                }
            }
        }
        _ => {
            kprintln!("Usage: integrity build | integrity check");
        }
    }
}

fn cmd_encrypt(args: &[&str]) {
    let Some(path) = args.first().copied() else {
        kprintln!("Usage: encrypt <file>");
        return;
    };

    let data = match fs::read_current(path) {
        Ok((_, data)) => data,
        Err(e) => {
            kprint_colored!(Colors::RED, "Error: ");
            kprintln!("cannot read {}: {:?}", path, e);
            return;
        }
    };

    kprint_colored!(Colors::DIM, "Password: ");
    let password = crate::auth::login::read_line_hidden();
    kprintln!();
    if password.is_empty() {
        kprintln!("Cancelled.");
        return;
    }

    let encrypted = security::crypt::file_encryption::encrypt(&data, &password);
    let enc_path = alloc::format!("{}.enc", path);
    match fs::write_current(&enc_path, &encrypted) {
        Ok(_) => {
            let _ = fs::delete_current(path);
            kprint_colored!(Colors::GREEN, "Encrypted: ");
            kprintln!("{} -> {} ({} bytes)", path, enc_path, encrypted.len());
            security::audit::log_event(security::audit::events::AuditEvent::FileCreated {
                path: enc_path,
                uid: crate::exec::current_uid(),
            });
        }
        Err(e) => {
            kprint_colored!(Colors::RED, "Error writing: ");
            kprintln!("{:?}", e);
        }
    }
}

fn cmd_decrypt(args: &[&str]) {
    let Some(path) = args.first().copied() else {
        kprintln!("Usage: decrypt <file.enc>");
        return;
    };

    let data = match fs::read_current(path) {
        Ok((_, data)) => data,
        Err(e) => {
            kprint_colored!(Colors::RED, "Error: ");
            kprintln!("cannot read {}: {:?}", path, e);
            return;
        }
    };

    kprint_colored!(Colors::DIM, "Password: ");
    let password = crate::auth::login::read_line_hidden();
    kprintln!();
    if password.is_empty() {
        kprintln!("Cancelled.");
        return;
    }

    match security::crypt::file_encryption::decrypt(&data, &password) {
        Ok(plaintext) => {
            let out_path = if path.ends_with(".enc") {
                &path[..path.len() - 4]
            } else {
                path
            };
            match fs::write_current(out_path, &plaintext) {
                Ok(_) => {
                    let _ = fs::delete_current(path);
                    kprint_colored!(Colors::GREEN, "Decrypted: ");
                    kprintln!("{} -> {} ({} bytes)", path, out_path, plaintext.len());
                }
                Err(e) => {
                    kprint_colored!(Colors::RED, "Error writing: ");
                    kprintln!("{:?}", e);
                }
            }
        }
        Err(e) => {
            kprint_colored!(Colors::RED, "Decryption failed: ");
            kprintln!("{}", e);
        }
    }
}

fn cmd_qkd(args: &[&str]) {
    match args.first().copied() {
        Some("bb84") => {
            let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(256);
            kprint_colored!(Colors::CYAN, "BB84 Quantum Key Distribution:\n");
            let result = security::crypt::qkd::simulate_bb84(n);
            kprintln!("  Qubits sent:     {}", result.qubits_sent);
            kprintln!("  Matching bases:  {}", result.matching_bases);
            kprintln!(
                "  Error rate:      {:.1}% (threshold: 11%)",
                result.error_rate * 100.0
            );
            kprintln!(
                "  Eve detected:    {}",
                if result.eve_detected { "YES" } else { "NO" }
            );
            kprintln!("  Final key:       {} bits", result.final_key_bits);

            if !result.key.is_empty() && !result.eve_detected {
                // Save key to filesystem
                let key_id = security::crypt::qkd::stored_key_count() + 1;
                let key_path = alloc::format!("/etc/quantum_keys/session_{}", key_id);
                let _ = fs::write_current("/etc/quantum_keys/.dir", &[]);
                let _ = fs::write_current(&key_path, &result.key);
                kprint_colored!(Colors::GREEN, "  Key saved to ");
                kprintln!("{}", key_path);
            } else if result.eve_detected {
                kprint_colored!(Colors::RED, "  Channel compromised — key discarded\n");
            }
        }
        _ => {
            kprintln!("Usage: qkd bb84 [n_qubits]");
            kprintln!("  Simulate BB84 quantum key distribution protocol");
        }
    }
}
