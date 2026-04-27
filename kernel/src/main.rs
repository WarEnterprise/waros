#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]
#![feature(alloc_error_handler)]
#![deny(unsafe_op_in_unsafe_fn)]

extern crate alloc;

mod arch;
mod auth;
mod boot;
mod disk;
mod display;
mod drivers;
mod exec;
mod fs;
mod gui;
mod hal;
mod interactive;
mod memory;
mod net;
mod panic;
mod pkg;
mod quantum;
mod security;
mod shell;
mod task;
mod ui;

use core::alloc::Layout;
use core::sync::atomic::{AtomicU64, Ordering};

use bootloader_api::config::{BootloaderConfig, Mapping};
use bootloader_api::{entry_point, BootInfo};
use x86_64::instructions::interrupts as cpu_interrupts;
use x86_64::registers::rflags;

use crate::boot::trace::{self, BootStage, BreadcrumbTag};
use crate::display::console::Colors;

pub const KERNEL_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const BUILD_DATE: &str = "2026-03-23";
pub const BUILD_TAG: &str = "WAROS-INPUT-STABILITY-27";
pub static HEARTBEAT: AtomicU64 = AtomicU64::new(0);
static BOOT_COMPLETE_MS: AtomicU64 = AtomicU64::new(0);
static LAST_BOOT_STEP_MS: AtomicU64 = AtomicU64::new(0);
static EARLY_DEBUG_READY: AtomicU64 = AtomicU64::new(0);
static LAST_HEARTBEAT_LOG: AtomicU64 = AtomicU64::new(0);
const HEARTBEAT_LOG_INTERVAL: u64 = 100;

pub static BOOTLOADER_CONFIG: BootloaderConfig = {
    let mut config = BootloaderConfig::new_default();
    config.mappings.physical_memory = Some(Mapping::Dynamic);
    config
};

entry_point!(kernel_main, config = &BOOTLOADER_CONFIG);

fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
    early_boot_marker("entry");
    if let Err(error) = try_kernel_main(boot_info) {
        early_boot_marker("fatal");
        fatal(error);
    }

    interactive::progress("boot-handoff-begin");

    // ── DIAGNOSTIC: set to true to bypass all auth/exec/shell and prove ──
    // ── basic hardware (framebuffer + timer + input) works.              ──
    const MINIMAL_INTERACTIVE: bool = false;
    if MINIMAL_INTERACTIVE {
        interactive::minimal_interactive_mode();
    }

    // Defer heavy boot-proof cleanup until after the first successful interactive cycle.
    // Real hardware can still be timing-sensitive immediately after boot proofs.
    let mut first_interactive_cycle = true;

    loop {
        interactive::progress("main-loop-top");
        heartbeat_log_if_due("kernel-main-loop");
        // Skip recovery check in the very first handoff to avoid blocking on
        // persistent-state paths before the login UI is visible.
        if !first_interactive_cycle && pkg::update::should_enter_recovery() {
            auth::session::start_recovery();
            boot_notice("Recovery shell active. Use 'recovery status' to inspect update health.");
            shell::run();
            exec::reset_shell_process();
            shell::history::clear();
            continue;
        }

        // Ensure interrupts are ON before entering the interactive input loop.
        if !cpu_interrupts::are_enabled() {
            serial_println!("[TRACE] WARNING: interrupts OFF before login; re-enabling");
            cpu_interrupts::enable();
        }
        if !display::console::rendering_enabled() {
            serial_println!("[TRACE] WARNING: console rendering OFF before login; re-enabling");
            display::console::set_rendering_enabled(true);
            display::console::claim_screen_owner(
                display::console::ScreenOwner::Boot,
                "main-loop-rendering-reenable",
            );
            let _ = display::console::clear_screen_for(
                display::console::ScreenOwner::Boot,
                "main-loop-rendering-reenable",
            );
        }
        interactive::progress("interrupts-verified");

        // Defer USB polling to the interactive input service path. For some real
        // xHCI controllers the immediate post-boot poll can stall handoff.
        interactive::progress("usb-poll-deferred");

        interactive::progress("auth-select-begin");
        let first_boot = auth::first_boot_pending();
        interactive::progress("auth-select-done");

        let user = if first_boot {
            interactive::progress("first-boot-setup-enter");
            auth::login::first_boot_setup()
        } else {
            interactive::progress("login-screen-enter");
            auth::login::login_screen()
        };
        serial_println!(
            "[interactive_handoff] stage=boot event=auth returned user={} ticks={} IF={}",
            user.username,
            crate::arch::x86_64::interrupts::tick_count(),
            if cpu_interrupts::are_enabled() { "on" } else { "off" }
        );

        if first_boot {
            auth::clear_first_boot_pending();
            serial_println!(
                "[TRACE] auth: first-boot account ready user={} first_boot_pending=forced-false",
                user.username
            );
        }

        serial_println!(
            "[TRACE] auth: starting interactive session for {}",
            user.username
        );
        auth::session::start(user.clone());

        if first_boot {
            serial_println!(
                "[TRACE] auth: rendering first-boot session handoff for {}",
                user.username
            );
            auth::login::render_session_ready(&user, true);
            serial_println!("[TRACE] auth: first-boot session handoff rendered");
        }

        if first_interactive_cycle {
            interactive::progress("post-auth-first-cycle-cleanup");
            exec::reset_shell_process();
            first_interactive_cycle = false;
        }

        shell::run();
        exec::reset_shell_process();
        shell::history::clear();
    }
}

fn try_kernel_main(boot_data: &'static mut BootInfo) -> Result<(), &'static str> {
    early_boot_marker("serial-init");
    drivers::serial::init();
    early_boot_marker("serial-ready");
    serial_println!("WarOS: entering kernel bootstrap");
    trace::set_stage(BootStage::KernelEntry, BreadcrumbTag::KernelEntry);
    early_boot_marker("fpu");
    arch::x86_64::fpu::init();

    early_boot_marker("bootstrap");
    let boot_context = boot::bootstrap(boot_data)?;
    early_boot_marker("bootstrap-ok");
    trace::set_stage(BootStage::BootHandoff, BreadcrumbTag::BootContext);
    memory::register_boot_memory_regions(boot_context.memory_regions);
    memory::register_physical_memory_mapping(boot_context.physical_memory_offset);
    trace::record_physical_memory_mapping(
        boot_context.physical_memory_offset.as_u64(),
        memory::max_physical_address(),
    );
    trace::record_rsdp_addr(boot_context.rsdp_addr);
    trace::record_tag(BreadcrumbTag::PhysMapRegistered);
    let framebuffer_info = boot::uefi::framebuffer_info(boot_context.framebuffer);

    early_boot_marker("console-init");
    display::console::init(boot_context.framebuffer);
    display::console::claim_screen_owner(
        display::console::ScreenOwner::Boot,
        "boot-console-init",
    );
    early_boot_marker("console-ok");
    display::branding::show_banner();
    display::stage_trace::show_build_tag();
    trace::record_tag(BreadcrumbTag::ConsoleReady);

    boot_ok("Serial debug on COM1");
    boot_ok("FPU/SSE initialized");
    kprintln!("[BUILD_TAG] {}", BUILD_TAG);
    serial_println!("[BUILD_TAG] {}", BUILD_TAG);
    boot_ok_fmt(
        format_args!(
            "Framebuffer: {}x{} @ {} bpp",
            framebuffer_info.width,
            framebuffer_info.height,
            framebuffer_info.bytes_per_pixel * 8
        ),
        format_args!(
            "Framebuffer: {}x{} @ {} bpp",
            framebuffer_info.width,
            framebuffer_info.height,
            framebuffer_info.bytes_per_pixel * 8
        ),
    );

    arch::x86_64::gdt::init();
    trace::set_stage(BootStage::DescriptorTables, BreadcrumbTag::GdtLoaded);
    boot_ok("GDT loaded");

    exec::syscall::init();
    trace::record_tag(BreadcrumbTag::SyscallReady);
    boot_ok("WarSyscall initialized");

    arch::x86_64::idt::init();
    trace::record_tag(BreadcrumbTag::IdtLoaded);
    boot_ok("IDT loaded (exceptions + timer + keyboard)");

    unsafe {
        // SAFETY: The PIC is initialized once during early boot before interrupts are enabled.
        arch::x86_64::pic::init();
    }
    trace::record_tag(BreadcrumbTag::PicReady);
    boot_ok("PIC remapped (IRQ 32-47)");

    arch::x86_64::pit::init();
    trace::record_tag(BreadcrumbTag::PitReady);
    boot_ok_fmt(
        format_args!("PIT timer: {} Hz", arch::x86_64::pit::PIT_FREQUENCY_HZ),
        format_args!("PIT timer: {} Hz", arch::x86_64::pit::PIT_FREQUENCY_HZ),
    );
    reset_boot_timing_trace();
    boot_time_mark("pit-ready");

    trace::set_stage(BootStage::Memory, BreadcrumbTag::MemoryMapRegistered);
    early_boot_marker("memory-init");
    memory::init(boot_context.memory_regions)?;
    early_boot_marker("memory-ok");
    trace::record_tag(BreadcrumbTag::PhysicalAllocatorReady);
    let stats = memory::stats();
    boot_ok_fmt(
        format_args!(
            "Physical memory: {} MiB ({} frames available)",
            (stats.free_frames * 4) / 1024,
            stats.free_frames
        ),
        format_args!(
            "Physical memory: {} MiB ({} frames available)",
            (stats.free_frames * 4) / 1024,
            stats.free_frames
        ),
    );

    let mut mapper = unsafe {
        // SAFETY: The bootloader mapped physical memory at the configured offset exposed
        // through `boot_info.physical_memory_offset`, which `boot::bootstrap` validated.
        memory::paging::init(boot_context.physical_memory_offset)
    };
    trace::record_tag(BreadcrumbTag::PagingMapperReady);
    boot_ok("Paging: 4-level page tables active");

    {
        early_boot_marker("heap-init");
        let mut allocator_guard = memory::FRAME_ALLOCATOR.lock();
        let Some(frame_allocator) = allocator_guard.as_mut() else {
            return Err("frame allocator missing after initialization");
        };
        if let Err(error) = memory::heap::init_heap(&mut mapper, frame_allocator) {
            serial_println!(
                "[ERR] heap initialization failed ({} MiB requested): {:?}",
                memory::heap::HEAP_SIZE / (1024 * 1024),
                error
            );
            return Err("kernel heap initialization failed");
        }
    }
    early_boot_marker("heap-ok");
    trace::record_tag(BreadcrumbTag::HeapReady);
    boot_ok_fmt(
        format_args!(
            "Kernel heap: {} MiB allocated",
            memory::heap::HEAP_SIZE / (1024 * 1024)
        ),
        format_args!(
            "Kernel heap: {} MiB allocated",
            memory::heap::HEAP_SIZE / (1024 * 1024)
        ),
    );
    boot_heap_mark("heap-ready");
    match display::console::enable_shadow_buffer() {
        Ok(()) => serial_println!("[EARLY] framebuffer shadow buffer enabled after heap init"),
        Err(error) => serial_println!(
            "[EARLY] framebuffer shadow buffer unavailable after heap init: {}",
            error
        ),
    }
    boot_heap_mark("console-shadow");

    hal::init_registry();
    hal::register_core_devices(
        framebuffer_info.width as u32,
        framebuffer_info.height as u32,
    );
    hal::display::register_framebuffer(framebuffer_info);
    trace::record_tag(BreadcrumbTag::HalReady);
    boot_ok("WarHAL device registry initialized");
    boot_heap_mark("hal-ready");

    fs::init();
    trace::record_tag(BreadcrumbTag::FsReady);
    boot_ok("WarFS: filesystem core ready");
    boot_heap_mark("fs-ready");

    trace::set_stage(BootStage::Acpi, BreadcrumbTag::AcpiInit);
    match hal::acpi::init_global(boot_context.rsdp_addr) {
        Ok(_) => boot_ok("ACPI: power management available"),
        Err(error) => boot_notice(alloc::format!("ACPI: not available ({:?})", error).as_str()),
    }

    trace::set_stage(BootStage::Storage, BreadcrumbTag::DiskReady);
    let disk_report = {
        let mut filesystem = fs::FILESYSTEM.lock();
        disk::init(&mut filesystem)
    };
    match disk_report {
        Ok(Some(report)) if report.formatted => {
            boot_notice(&alloc::format!(
                "Disk: {} MB, no WarFS found, formatting complete",
                report.size_mb
            ));
            boot_ok_fmt(
                format_args!(
                    "Disk: {} MB, WarFS v{} formatted",
                    report.size_mb, report.version
                ),
                format_args!(
                    "Disk: {} MB, WarFS v{} formatted",
                    report.size_mb, report.version
                ),
            );
        }
        Ok(Some(report)) => {
            boot_ok_fmt(
                format_args!(
                    "Disk: {} MB, WarFS v{}, {} files loaded",
                    report.size_mb, report.version, report.loaded_files
                ),
                format_args!(
                    "Disk: {} MB, WarFS v{}, {} files loaded",
                    report.size_mb, report.version, report.loaded_files
                ),
            );
        }
        Ok(None) => boot_notice("No virtio-blk disk (running RAM-only mode)"),
        Err(error) => {
            let message =
                alloc::format!("virtio-blk unavailable ({}), running RAM-only mode", error);
            boot_notice(message.as_str());
        }
    }
    trace::record_tag(BreadcrumbTag::AuthReady);
    let auth_report = auth::init().map_err(|_| "user database initialization failed")?;
    if auth_report.first_boot {
        boot_ok("User database initialized (root account seeded)");
        boot_notice("First boot setup pending: create your admin account after boot");
    } else {
        boot_ok_fmt(
            format_args!("User database loaded ({} users)", auth_report.users),
            format_args!("User database loaded ({} users)", auth_report.users),
        );
    }

    task::init();
    trace::set_stage(BootStage::Exec, BreadcrumbTag::TaskReady);
    boot_ok("Task scheduler: cooperative background tasks ready");
    exec::init();
    trace::record_tag(BreadcrumbTag::ExecReady);
    boot_ok("WarExec core ready");

    security::init();
    trace::record_tag(BreadcrumbTag::SecurityReady);
    boot_heap_mark("security-ready");

    trace::set_stage(BootStage::Hardware, BreadcrumbTag::NetInit);
    let network = boot_time_scope("net::init", net::init)
        .map_err(|_| "network initialization failed")?;
    let pci_inventory = net::pci_devices();
    hal::bus::pci::enumerate_and_register(&pci_inventory);
    if hal::storage::register_active_storage().is_some() {
        boot_ok("WarHAL: persistent storage registered");
    }
    trace::record_tag(BreadcrumbTag::UsbProbe);
    let usb_controllers = boot_time_scope("usb::probe_controllers", hal::usb::probe_controllers);
    hal::net::register_detected_nics();
    boot_ok_fmt(
        format_args!("PCI scan: {} devices found", network.pci_devices),
        format_args!("PCI scan: {} devices found", network.pci_devices),
    );
    if usb_controllers > 0 {
        boot_ok_fmt(
            format_args!("USB controllers discovered: {}", usb_controllers),
            format_args!("USB controllers discovered: {}", usb_controllers),
        );
    } else {
        boot_notice("USB: no host controller detected");
    }
    boot_ok_fmt(
        format_args!("Serial link: {}", network.serial_status),
        format_args!("Serial link: {}", network.serial_status),
    );
    if let Some(ref device) = network.hardware {
        let _ = hal::net::register_active_network();
        let mac = net::format_mac(&device.mac);
        match device.transport {
            net::NetworkTransport::Io(io_base) => boot_ok_fmt(
                format_args!("{}: MAC {} (I/O 0x{:04X})", device.driver, mac, io_base),
                format_args!("{}: MAC {} (I/O 0x{:04X})", device.driver, mac, io_base),
            ),
            net::NetworkTransport::Mmio(mmio_base) => boot_ok_fmt(
                format_args!("{}: MAC {} (MMIO 0x{:08X})", device.driver, mac, mmio_base),
                format_args!("{}: MAC {} (MMIO 0x{:08X})", device.driver, mac, mmio_base),
            ),
        };
        match device.link_state {
            net::LinkState::Up => boot_ok_fmt(
                format_args!("Network link: up {} Mbps", device.link_speed_mbps),
                format_args!("Network link: up {} Mbps", device.link_speed_mbps),
            ),
            net::LinkState::Down => boot_notice("Network link: down"),
            net::LinkState::Unknown => boot_notice("Network link: unknown"),
        }
    } else {
        boot_notice("No supported NIC detected");
    }
    if let Some(config) = network.network_config {
        boot_ok_fmt(
            format_args!(
                "DHCP: {} gw {}",
                config.cidr_string(),
                config.gateway.unwrap_or(net::ipv4::Ipv4Addr::ZERO)
            ),
            format_args!(
                "DHCP: {} gw {}",
                config.cidr_string(),
                config.gateway.unwrap_or(net::ipv4::Ipv4Addr::ZERO)
            ),
        );
        if let Some(dns_server) = config.dns_server {
            boot_ok_fmt(
                format_args!("DNS: {}", dns_server),
                format_args!("DNS: {}", dns_server),
            );
        }
    } else if network.hardware.is_some() {
        boot_notice("DHCP: no lease acquired");
    }
    boot_heap_mark("net-ready");
    boot_heap_mark("usb-ready");

    drivers::keyboard::init();
    hal::input::init();
    ui::init();
    trace::set_stage(BootStage::Input, BreadcrumbTag::InputReady);
    cpu_interrupts::enable();
    trace::record_tag(BreadcrumbTag::InterruptsEnabled);
    boot_ok("Keyboard driver active");
    boot_ok("Quantum subsystem ready (18 qubits max)");

    trace::set_stage(BootStage::Proofs, BreadcrumbTag::ProofsActive);
    boot_notice("WarShield TLS proof: validating trusted and rejected certificate paths");
    match boot_time_scope("proof::tls", || {
        cpu_interrupts::without_interrupts(net::tls::run_validation_proof)
    }) {
        Ok(()) => boot_ok_fmt(
            format_args!(
                "WarShield TLS proof: certificate validation wired ({})",
                net::tls::trust_policy_summary()
            ),
            format_args!(
                "WarShield TLS proof: certificate validation wired ({})",
                net::tls::trust_policy_summary()
            ),
        ),
        Err(error) => {
            let message = alloc::format!("WarShield TLS proof: {}", error);
            boot_notice(message.as_str());
        }
    }
    boot_heap_mark("after-tls-proof");

    let boot_complete_ms = boot_elapsed_ms();
    BOOT_COMPLETE_MS.store(boot_complete_ms, Ordering::Relaxed);
    fs::seed_system_files().map_err(|_| "failed to seed filesystem system files")?;
    boot_ok("WarFS system files seeded");
    pkg::init().map_err(|_| "failed to seed package repository")?;
    boot_ok("WarPkg bootstrap repository ready");
    let boot_health = pkg::update::prepare_boot();
    let update_summary = alloc::format!("WarPkg update health: {}", boot_health.summary);
    boot_notice(update_summary.as_str());
    if boot_health.recovery_requested {
        boot_notice("Recovery mode will be entered after boot because update health requires operator action");
    }

    boot_notice("WarPkg proof: verifying signed bundle and tamper rejection");
    match boot_time_scope("proof::pkg-signature", || {
        cpu_interrupts::without_interrupts(pkg::smoke::run_signature_proof)
    }) {
        Ok(()) => boot_ok_fmt(
            format_args!(
                "WarPkg proof: signed bundle verification wired ({})",
                pkg::trust_root_summary()
            ),
            format_args!(
                "WarPkg proof: signed bundle verification wired ({})",
                pkg::trust_root_summary()
            ),
        ),
        Err(error) => {
            let message = alloc::format!("WarPkg proof: {}", error);
            boot_notice(message.as_str());
        }
    }

    match pkg::update::proof_available() {
        Ok(true) => {
            boot_notice(
                "WarPkg Pass 4 proof: exercising offline update, boot health, rollback, tamper rejection, and recovery status",
            );
            match boot_time_scope("proof::pkg-pass4", || {
                cpu_interrupts::without_interrupts(pkg::smoke::run_offline_update_proof)
            }) {
                Ok(()) => boot_ok("WarPkg Pass 4 proof passed"),
                Err(error) => {
                    let message = alloc::format!("WarPkg Pass 4 proof: {}", error);
                    boot_notice(message.as_str());
                }
            }
        }
        Ok(false) => {
            boot_notice("WarPkg Pass 4 proof skipped: active update or recovery state present");
        }
        Err(_) => {
            boot_notice("WarPkg Pass 4 proof skipped: update state unavailable");
        }
    }

    boot_notice("WarShield capability proof: checking inherit-only launch and deny-after-drop");
    match boot_time_scope("proof::capabilities", || {
        cpu_interrupts::without_interrupts(security::capabilities::run_transition_proof)
    }) {
        Ok(()) => boot_ok("WarShield capability proof passed"),
        Err(error) => {
            let message = alloc::format!("WarShield capability proof: {}", error);
            boot_notice(message.as_str());
        }
    }
    boot_heap_mark("after-capabilities-proof");

    boot_notice("WarExec smoke: launching /bin/warexec-smoke.elf");
    boot_heap_mark("before-warexec-smoke");

    match boot_time_scope("proof::warexec-smoke", || {
        exec::smoke::run()
    }) {
        Ok(exit_code) if exit_code == exec::smoke::SMOKE_ELF_EXIT_CODE => boot_ok_fmt(
            format_args!(
                "WarExec smoke: {} exited with code {}",
                exec::smoke::SMOKE_ELF_PATH,
                exit_code
            ),
            format_args!(
                "WarExec smoke: {} exited with code {}",
                exec::smoke::SMOKE_ELF_PATH,
                exit_code
            ),
        ),
        Ok(exit_code) => {
            let message = alloc::format!(
                "WarExec smoke: {} exited with unexpected code {}",
                exec::smoke::SMOKE_ELF_PATH,
                exit_code
            );
            boot_notice(message.as_str());
        }
        Err(error) => {
            let message = alloc::format!(
                "WarExec smoke: failed to execute {} ({:?})",
                exec::smoke::SMOKE_ELF_PATH,
                error
            );
            boot_notice(message.as_str());
        }
    }

    boot_notice("WarExec ABI proof: launching /bin/warexec-read-smoke.elf");
    boot_time_mark("proof::abi-suite-start");
    match exec::smoke::run_abi_read_smoke() {
        Ok(exit_code) if exit_code == exec::smoke::ABI_READ_SMOKE_ELF_EXIT_CODE => boot_ok_fmt(
            format_args!(
                "WarExec ABI proof: {} exited with code {}",
                exec::smoke::ABI_READ_SMOKE_ELF_PATH,
                exit_code
            ),
            format_args!(
                "WarExec ABI proof: {} exited with code {}",
                exec::smoke::ABI_READ_SMOKE_ELF_PATH,
                exit_code
            ),
        ),
        Ok(exit_code) => {
            let message = alloc::format!(
                "WarExec ABI proof: {} exited with unexpected code {}",
                exec::smoke::ABI_READ_SMOKE_ELF_PATH,
                exit_code
            );
            boot_notice(message.as_str());
        }
        Err(error) => {
            let message = alloc::format!(
                "WarExec ABI proof: failed to execute {} ({:?})",
                exec::smoke::ABI_READ_SMOKE_ELF_PATH,
                error
            );
            boot_notice(message.as_str());
        }
    }

    boot_notice("WarExec ABI proof: launching /bin/warexec-offset-smoke.elf");
    match exec::smoke::run_abi_offset_smoke() {
        Ok(exit_code) if exit_code == exec::smoke::ABI_OFFSET_SMOKE_ELF_EXIT_CODE => boot_ok_fmt(
            format_args!(
                "WarExec ABI proof: {} exited with code {}",
                exec::smoke::ABI_OFFSET_SMOKE_ELF_PATH,
                exit_code
            ),
            format_args!(
                "WarExec ABI proof: {} exited with code {}",
                exec::smoke::ABI_OFFSET_SMOKE_ELF_PATH,
                exit_code
            ),
        ),
        Ok(exit_code) => {
            let message = alloc::format!(
                "WarExec ABI proof: {} exited with unexpected code {}",
                exec::smoke::ABI_OFFSET_SMOKE_ELF_PATH,
                exit_code
            );
            boot_notice(message.as_str());
        }
        Err(error) => {
            let message = alloc::format!(
                "WarExec ABI proof: failed to execute {} ({:?})",
                exec::smoke::ABI_OFFSET_SMOKE_ELF_PATH,
                error
            );
            boot_notice(message.as_str());
        }
    }

    boot_notice("WarExec ABI proof: launching /bin/warexec-argv-smoke.elf");
    match exec::smoke::run_abi_argv_smoke() {
        Ok(exit_code) if exit_code == exec::smoke::ABI_ARGV_SMOKE_ELF_EXIT_CODE => boot_ok_fmt(
            format_args!(
                "WarExec ABI proof: {} exited with code {}",
                exec::smoke::ABI_ARGV_SMOKE_ELF_PATH,
                exit_code
            ),
            format_args!(
                "WarExec ABI proof: {} exited with code {}",
                exec::smoke::ABI_ARGV_SMOKE_ELF_PATH,
                exit_code
            ),
        ),
        Ok(exit_code) => {
            let message = alloc::format!(
                "WarExec ABI proof: {} exited with unexpected code {}",
                exec::smoke::ABI_ARGV_SMOKE_ELF_PATH,
                exit_code
            );
            boot_notice(message.as_str());
        }
        Err(error) => {
            let message = alloc::format!(
                "WarExec ABI proof: failed to execute {} ({:?})",
                exec::smoke::ABI_ARGV_SMOKE_ELF_PATH,
                error
            );
            boot_notice(message.as_str());
        }
    }

    boot_notice("WarExec ABI proof: launching /bin/warexec-exec-parent.elf");
    match exec::smoke::run_abi_exec_smoke() {
        Ok(exit_code) if exit_code == exec::smoke::ABI_EXEC_CHILD_ELF_EXIT_CODE => boot_ok_fmt(
            format_args!(
                "WarExec ABI proof: {} exited with code {}",
                exec::smoke::ABI_EXEC_CHILD_ELF_PATH,
                exit_code
            ),
            format_args!(
                "WarExec ABI proof: {} exited with code {}",
                exec::smoke::ABI_EXEC_CHILD_ELF_PATH,
                exit_code
            ),
        ),
        Ok(exit_code) => {
            let message = alloc::format!(
                "WarExec ABI proof: {} exited with unexpected code {}",
                exec::smoke::ABI_EXEC_CHILD_ELF_PATH,
                exit_code
            );
            boot_notice(message.as_str());
        }
        Err(error) => {
            let message = alloc::format!(
                "WarExec ABI proof: failed to execute {} ({:?})",
                exec::smoke::ABI_EXEC_PARENT_ELF_PATH,
                error
            );
            boot_notice(message.as_str());
        }
    }

    boot_notice("WarExec ABI proof: launching /bin/warexec-heap-smoke.elf");
    match exec::smoke::run_abi_heap_smoke() {
        Ok(exit_code) if exit_code == exec::smoke::ABI_HEAP_SMOKE_ELF_EXIT_CODE => boot_ok_fmt(
            format_args!(
                "WarExec ABI proof: {} exited with code {}",
                exec::smoke::ABI_HEAP_SMOKE_ELF_PATH,
                exit_code
            ),
            format_args!(
                "WarExec ABI proof: {} exited with code {}",
                exec::smoke::ABI_HEAP_SMOKE_ELF_PATH,
                exit_code
            ),
        ),
        Ok(exit_code) => {
            let message = alloc::format!(
                "WarExec ABI proof: {} exited with unexpected code {}",
                exec::smoke::ABI_HEAP_SMOKE_ELF_PATH,
                exit_code
            );
            boot_notice(message.as_str());
        }
        Err(error) => {
            let message = alloc::format!(
                "WarExec ABI proof: failed to execute {} ({:?})",
                exec::smoke::ABI_HEAP_SMOKE_ELF_PATH,
                error
            );
            boot_notice(message.as_str());
        }
    }

    boot_notice("WarExec ABI proof: launching /bin/warexec-fault-smoke.elf");
    match exec::smoke::run_abi_fault_smoke() {
        Ok(exit_code) if exit_code == exec::smoke::ABI_FAULT_SMOKE_ELF_EXIT_CODE => boot_ok_fmt(
            format_args!(
                "WarExec ABI proof: {} exited with code {}",
                exec::smoke::ABI_FAULT_SMOKE_ELF_PATH,
                exit_code
            ),
            format_args!(
                "WarExec ABI proof: {} exited with code {}",
                exec::smoke::ABI_FAULT_SMOKE_ELF_PATH,
                exit_code
            ),
        ),
        Ok(exit_code) => {
            let message = alloc::format!(
                "WarExec ABI proof: {} exited with unexpected code {}",
                exec::smoke::ABI_FAULT_SMOKE_ELF_PATH,
                exit_code
            );
            boot_notice(message.as_str());
        }
        Err(error) => {
            let message = alloc::format!(
                "WarExec ABI proof: failed to execute {} ({:?})",
                exec::smoke::ABI_FAULT_SMOKE_ELF_PATH,
                error
            );
            boot_notice(message.as_str());
        }
    }

    boot_notice("WarExec ABI proof: launching /bin/warexec-wait-smoke.elf");
    match exec::smoke::run_abi_wait_smoke() {
        Ok(exit_code) if exit_code == exec::smoke::ABI_WAIT_PARENT_ELF_EXIT_CODE => boot_ok_fmt(
            format_args!(
                "WarExec ABI proof: {} exited with code {}",
                exec::smoke::ABI_WAIT_PARENT_ELF_PATH,
                exit_code
            ),
            format_args!(
                "WarExec ABI proof: {} exited with code {}",
                exec::smoke::ABI_WAIT_PARENT_ELF_PATH,
                exit_code
            ),
        ),
        Ok(exit_code) => {
            let message = alloc::format!(
                "WarExec ABI proof: {} exited with unexpected code {}",
                exec::smoke::ABI_WAIT_PARENT_ELF_PATH,
                exit_code
            );
            boot_notice(message.as_str());
        }
        Err(error) => {
            let message = alloc::format!(
                "WarExec ABI proof: failed to execute {} ({:?})",
                exec::smoke::ABI_WAIT_PARENT_ELF_PATH,
                error
            );
            boot_notice(message.as_str());
        }
    }

    boot_notice("WarExec ABI proof: launching /bin/warexec-stat-smoke.elf");
    match exec::smoke::run_abi_stat_smoke() {
        Ok(exit_code) if exit_code == exec::smoke::ABI_STAT_SMOKE_ELF_EXIT_CODE => boot_ok_fmt(
            format_args!(
                "WarExec ABI proof: {} exited with code {}",
                exec::smoke::ABI_STAT_SMOKE_ELF_PATH,
                exit_code
            ),
            format_args!(
                "WarExec ABI proof: {} exited with code {}",
                exec::smoke::ABI_STAT_SMOKE_ELF_PATH,
                exit_code
            ),
        ),
        Ok(exit_code) => {
            let message = alloc::format!(
                "WarExec ABI proof: {} exited with unexpected code {}",
                exec::smoke::ABI_STAT_SMOKE_ELF_PATH,
                exit_code
            );
            boot_notice(message.as_str());
        }
        Err(error) => {
            let message = alloc::format!(
                "WarExec ABI proof: failed to execute {} ({:?})",
                exec::smoke::ABI_STAT_SMOKE_ELF_PATH,
                error
            );
            boot_notice(message.as_str());
        }
    }

    boot_notice("WarExec ABI proof: launching /bin/warexec-readdir-smoke.elf");
    match exec::smoke::run_abi_readdir_smoke() {
        Ok(exit_code) if exit_code == exec::smoke::ABI_READDIR_SMOKE_ELF_EXIT_CODE => boot_ok_fmt(
            format_args!(
                "WarExec ABI proof: {} exited with code {}",
                exec::smoke::ABI_READDIR_SMOKE_ELF_PATH,
                exit_code
            ),
            format_args!(
                "WarExec ABI proof: {} exited with code {}",
                exec::smoke::ABI_READDIR_SMOKE_ELF_PATH,
                exit_code
            ),
        ),
        Ok(exit_code) => {
            let message = alloc::format!(
                "WarExec ABI proof: {} exited with unexpected code {}",
                exec::smoke::ABI_READDIR_SMOKE_ELF_PATH,
                exit_code
            );
            boot_notice(message.as_str());
        }
        Err(error) => {
            let message = alloc::format!(
                "WarExec ABI proof: failed to execute {} ({:?})",
                exec::smoke::ABI_READDIR_SMOKE_ELF_PATH,
                error
            );
            boot_notice(message.as_str());
        }
    }

    boot_notice("WarExec ABI proof: launching /bin/warexec-path-smoke.elf");
    match exec::smoke::run_abi_path_smoke() {
        Ok(exit_code) if exit_code == exec::smoke::ABI_PATH_SMOKE_ELF_EXIT_CODE => boot_ok_fmt(
            format_args!(
                "WarExec ABI proof: {} exited with code {}",
                exec::smoke::ABI_PATH_SMOKE_ELF_PATH,
                exit_code
            ),
            format_args!(
                "WarExec ABI proof: {} exited with code {}",
                exec::smoke::ABI_PATH_SMOKE_ELF_PATH,
                exit_code
            ),
        ),
        Ok(exit_code) => {
            let message = alloc::format!(
                "WarExec ABI proof: {} exited with unexpected code {}",
                exec::smoke::ABI_PATH_SMOKE_ELF_PATH,
                exit_code
            );
            boot_notice(message.as_str());
        }
        Err(error) => {
            let message = alloc::format!(
                "WarExec ABI proof: failed to execute {} ({:?})",
                exec::smoke::ABI_PATH_SMOKE_ELF_PATH,
                error
            );
            boot_notice(message.as_str());
        }
    }

    boot_notice("WarExec ABI proof: launching /bin/warexec-write-smoke.elf");
    match exec::smoke::run_abi_write_smoke() {
        Ok(exit_code) if exit_code == exec::smoke::ABI_WRITE_SMOKE_ELF_EXIT_CODE => boot_ok_fmt(
            format_args!(
                "WarExec ABI proof: {} exited with code {}",
                exec::smoke::ABI_WRITE_SMOKE_ELF_PATH,
                exit_code
            ),
            format_args!(
                "WarExec ABI proof: {} exited with code {}",
                exec::smoke::ABI_WRITE_SMOKE_ELF_PATH,
                exit_code
            ),
        ),
        Ok(exit_code) => {
            let message = alloc::format!(
                "WarExec ABI proof: {} exited with unexpected code {}",
                exec::smoke::ABI_WRITE_SMOKE_ELF_PATH,
                exit_code
            );
            boot_notice(message.as_str());
        }
        Err(error) => {
            let message = alloc::format!(
                "WarExec ABI proof: failed to execute {} ({:?})",
                exec::smoke::ABI_WRITE_SMOKE_ELF_PATH,
                error
            );
            boot_notice(message.as_str());
        }
    }

    boot_time_mark("proof::abi-suite-end");
    // Re-arm IRQ delivery before the cosmetic boot animation and the subsequent
    // login/shell loops, which both rely on interrupt-driven wakeups.
    cpu_interrupts::enable();
    serial_println!(
        "[TRACE] late boot: post-abi-proof ticks={} interrupts={}",
        crate::arch::x86_64::interrupts::tick_count(),
        if interrupts_enabled() { "on" } else { "off" }
    );
    boot_time_scope("boot::animation", display::branding::boot_complete_animation);
    serial_println!(
        "[TRACE] late boot: boot animation complete ticks={} interrupts={}",
        crate::arch::x86_64::interrupts::tick_count(),
        if interrupts_enabled() { "on" } else { "off" }
    );
    display::branding::show_separator();
    kprint_colored!(Colors::DIM, "Boot complete in {} ms.\n", boot_complete_ms);
    if let Ok(Some(message)) = pkg::update::note_shell_ready() {
        boot_notice(message.as_str());
    }
    trace::set_stage(BootStage::Interactive, BreadcrumbTag::InteractiveReady);
    serial_println!("[TRACE] A before interactive auth ready");
    boot_notice("WarOS interactive auth ready. Opening preferences/login.");
    serial_println!("[TRACE] B after interactive auth ready");
    serial_println!("[TRACE] C before next line");
    serial_println!("[TRACE] boot: interactive auth handoff ready");
    serial_println!("[TRACE] D after next line");
    kprintln!();
    Ok(())
}

fn boot_ok(message: &str) {
    let elapsed = boot_elapsed_ms();
    kprint_colored!(Colors::GREEN, "[OK]");
    crate::kprint!(" {}", message);
    kprint_colored!(Colors::DIM, " ({:>3} ms)", elapsed);
    kprintln!();
    serial_println!("[OK] {} ({} ms)", message, elapsed);
}

fn boot_ok_fmt(screen_message: core::fmt::Arguments<'_>, serial_message: core::fmt::Arguments<'_>) {
    let elapsed = boot_elapsed_ms();
    kprint_colored!(Colors::GREEN, "[OK]");
    crate::kprint!(" ");
    crate::kprint!("{}", screen_message);
    kprint_colored!(Colors::DIM, " ({:>3} ms)", elapsed);
    kprintln!();
    serial_println!("[OK] {} ({} ms)", serial_message, elapsed);
}

fn boot_notice(message: &str) {
    kprint_colored!(Colors::BLUE, "[INFO]");
    kprintln!(" {}", message);
    serial_println!("[INFO] {}", message);
}

fn fatal(message: &str) -> ! {
    early_boot_write("[FATAL] ");
    early_boot_write(message);
    early_boot_write("\n");
    kprint_colored!(Colors::RED, "[ERR]");
    kprintln!(" {}", message);
    serial_println!("[ERR] {}", message);
    arch::x86_64::hlt_loop()
}

fn early_boot_marker(label: &str) {
    early_boot_write("[EARLY-BOOT] ");
    early_boot_write(label);
    early_boot_write("\n");
}

fn early_boot_write(message: &str) {
    early_debug_init();
    for byte in message.bytes() {
        if byte == b'\n' {
            early_debug_put_byte(b'\r');
        }
        early_debug_put_byte(byte);
    }
}

fn early_debug_init() {
    if EARLY_DEBUG_READY
        .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        let base = drivers::serial::COM1_PORT;
        crate::arch::x86_64::port::outb(base + 1, 0x00);
        crate::arch::x86_64::port::outb(base + 3, 0x80);
        crate::arch::x86_64::port::outb(base, 0x03);
        crate::arch::x86_64::port::outb(base + 1, 0x00);
        crate::arch::x86_64::port::outb(base + 3, 0x03);
        crate::arch::x86_64::port::outb(base + 2, 0xC7);
        crate::arch::x86_64::port::outb(base + 4, 0x0B);
    }
}

fn early_debug_put_byte(byte: u8) {
    crate::arch::x86_64::port::outb(0xE9, byte);
    let base = drivers::serial::COM1_PORT;
    let mut spin = 0usize;
    while crate::arch::x86_64::port::inb(base + 5) & 0x20 == 0 && spin < 1_000_000 {
        core::hint::spin_loop();
        spin += 1;
    }
    crate::arch::x86_64::port::outb(base, byte);
}

fn reset_boot_timing_trace() {
    LAST_BOOT_STEP_MS.store(boot_elapsed_ms(), Ordering::Relaxed);
}

fn boot_time_mark(label: &str) {
    let now = boot_elapsed_ms();
    let previous = LAST_BOOT_STEP_MS.swap(now, Ordering::Relaxed);
    serial_println!(
        "[BOOT-TIME] mark={} total={}ms delta={}ms",
        label,
        now,
        now.saturating_sub(previous)
    );
}

fn boot_heap_mark(label: &str) {
    let stats = memory::heap::stats();
    serial_println!(
        "[BOOT-HEAP] mark={} size={} used={} free={} peak={} largest={} last_large={} large_allocs={}",
        label,
        stats.size,
        stats.used,
        stats.free,
        stats.peak_used,
        stats.largest_request,
        stats.last_large_request,
        stats.large_allocations
    );
}

fn boot_time_scope<T>(label: &str, function: impl FnOnce() -> T) -> T {
    let start = boot_elapsed_ms();
    serial_println!("[BOOT-TIME] begin={} total={}ms", label, start);
    let result = function();
    let end = boot_elapsed_ms();
    let previous = LAST_BOOT_STEP_MS.swap(end, Ordering::Relaxed);
    serial_println!(
        "[BOOT-TIME] end={} total={}ms scope={}ms delta={}ms",
        label,
        end,
        end.saturating_sub(start),
        end.saturating_sub(previous)
    );
    result
}

#[must_use]
pub fn boot_complete_ms() -> u64 {
    BOOT_COMPLETE_MS.load(Ordering::Relaxed)
}

pub fn heartbeat_tick() {
    HEARTBEAT.fetch_add(1, Ordering::Relaxed);
}

pub fn heartbeat_log_if_due(context: &str) {
    let heartbeat = HEARTBEAT.load(Ordering::Relaxed);
    let last_logged = LAST_HEARTBEAT_LOG.load(Ordering::Relaxed);
    if heartbeat.saturating_sub(last_logged) < HEARTBEAT_LOG_INTERVAL {
        return;
    }
    LAST_HEARTBEAT_LOG.store(heartbeat, Ordering::Relaxed);
    serial_println!(
        "[HEARTBEAT] n={} context={} ticks={} IF={}",
        heartbeat,
        context,
        crate::arch::x86_64::interrupts::tick_count(),
        if interrupts_enabled() { "on" } else { "off" }
    );
}

#[must_use]
pub fn heartbeat_value() -> u64 {
    HEARTBEAT.load(Ordering::Relaxed)
}

fn boot_elapsed_ms() -> u64 {
    arch::x86_64::pit::elapsed_millis(crate::arch::x86_64::interrupts::tick_count())
}

fn interrupts_enabled() -> bool {
    rflags::read().contains(rflags::RFlags::INTERRUPT_FLAG)
}

#[alloc_error_handler]
fn alloc_error(layout: Layout) -> ! {
    early_boot_write("[ALLOC-ERR] ");
    match layout.size() {
        0 => early_boot_write("zero-sized allocation failed\n"),
        _ => early_boot_write("kernel heap exhausted\n"),
    }
    let heap_stats = memory::heap::stats();
    serial_println!(
        "[ALLOC-ERR] request={} align={} size={} used={} free={} peak={} largest={} last_large={} allocs={} frees={}",
        layout.size(),
        layout.align(),
        heap_stats.size,
        heap_stats.used,
        heap_stats.free,
        heap_stats.peak_used,
        heap_stats.largest_request,
        heap_stats.last_large_request,
        heap_stats.allocations,
        heap_stats.deallocations
    );
    fatal(match layout.size() {
        0 => "zero-sized allocation failed",
        _ => "kernel heap exhausted",
    })
}
