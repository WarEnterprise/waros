#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]
#![feature(alloc_error_handler)]
#![deny(unsafe_op_in_unsafe_fn)]

extern crate alloc;

mod auth;
mod arch;
mod boot;
mod display;
mod disk;
mod drivers;
mod exec;
mod fs;
mod gui;
mod hal;
mod memory;
mod net;
mod panic;
mod pkg;
mod quantum;
mod shell;
mod task;

use core::alloc::Layout;
use core::sync::atomic::{AtomicU64, Ordering};

use bootloader_api::config::{BootloaderConfig, Mapping};
use bootloader_api::{entry_point, BootInfo};
use x86_64::instructions::interrupts as cpu_interrupts;

use crate::display::console::Colors;

pub const KERNEL_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const BUILD_DATE: &str = "2026-03-23";
static BOOT_COMPLETE_MS: AtomicU64 = AtomicU64::new(0);

pub static BOOTLOADER_CONFIG: BootloaderConfig = {
    let mut config = BootloaderConfig::new_default();
    config.mappings.physical_memory = Some(Mapping::Dynamic);
    config
};

entry_point!(kernel_main, config = &BOOTLOADER_CONFIG);

fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
    if let Err(error) = try_kernel_main(boot_info) {
        fatal(error);
    }

    loop {
        let user = if auth::first_boot_pending() {
            let user = auth::login::first_boot_setup();
            auth::clear_first_boot_pending();
            user
        } else {
            auth::login::login_screen()
        };

        auth::session::start(user);
        shell::run();
    }
}

fn try_kernel_main(boot_data: &'static mut BootInfo) -> Result<(), &'static str> {
    drivers::serial::init();
    serial_println!("WarOS: entering kernel bootstrap");
    arch::x86_64::fpu::init();

    let boot_context = boot::bootstrap(boot_data)?;
    memory::register_physical_memory_mapping(boot_context.physical_memory_offset);
    let framebuffer_info = boot::uefi::framebuffer_info(boot_context.framebuffer);

    display::console::init(boot_context.framebuffer);
    display::branding::show_banner();

    boot_ok("Serial debug on COM1");
    boot_ok("FPU/SSE initialized");
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
    boot_ok("GDT loaded");

    exec::syscall::init();
    boot_ok("WarSyscall initialized");

    arch::x86_64::idt::init();
    boot_ok("IDT loaded (exceptions + timer + keyboard)");

    unsafe {
        // SAFETY: The PIC is initialized once during early boot before interrupts are enabled.
        arch::x86_64::pic::init();
    }
    boot_ok("PIC remapped (IRQ 32-47)");

    arch::x86_64::pit::init();
    boot_ok_fmt(
        format_args!("PIT timer: {} Hz", arch::x86_64::pit::PIT_FREQUENCY_HZ),
        format_args!("PIT timer: {} Hz", arch::x86_64::pit::PIT_FREQUENCY_HZ),
    );
    cpu_interrupts::enable();

    memory::init(boot_context.memory_regions)?;
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
    boot_ok("Paging: 4-level page tables active");

    {
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

    hal::init_registry();
    hal::register_core_devices(framebuffer_info.width as u32, framebuffer_info.height as u32);
    hal::display::register_framebuffer(framebuffer_info);
    boot_ok("WarHAL device registry initialized");

    fs::init();
    boot_ok("WarFS: filesystem core ready");

    match hal::acpi::init_global() {
        Ok(_) => boot_ok("ACPI: power management available"),
        Err(error) => boot_notice(alloc::format!("ACPI: not available ({:?})", error).as_str()),
    }

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
                format_args!("Disk: {} MB, WarFS v{} formatted", report.size_mb, report.version),
                format_args!("Disk: {} MB, WarFS v{} formatted", report.size_mb, report.version),
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
            let message = alloc::format!("virtio-blk unavailable ({}), running RAM-only mode", error);
            boot_notice(message.as_str());
        }
    }
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
    boot_ok("Task scheduler: cooperative background tasks ready");
    exec::init();
    boot_ok("WarExec core ready");

    let network = net::init().map_err(|_| "network initialization failed")?;
    let pci_inventory = net::pci_devices();
    hal::bus::pci::enumerate_and_register(&pci_inventory);
    if hal::storage::register_active_storage().is_some() {
        boot_ok("WarHAL: persistent storage registered");
    }
    let usb_controllers = hal::usb::probe_controllers();
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

    drivers::keyboard::init();
    hal::input::init();
    boot_ok("Keyboard driver active");
    boot_ok("Quantum subsystem ready (18 qubits max)");

    let boot_complete_ms = boot_elapsed_ms();
    BOOT_COMPLETE_MS.store(boot_complete_ms, Ordering::Relaxed);
    fs::seed_system_files().map_err(|_| "failed to seed filesystem system files")?;
    boot_ok("WarFS system files seeded");
    pkg::init().map_err(|_| "failed to seed package repository")?;
    boot_ok("WarPkg bootstrap repository ready");
    boot_notice("WarExec smoke: launching /bin/warexec-smoke.elf");

    // Keep the one-shot bootstrap ELF proof non-preemptible while it manipulates the
    // single-CPU scheduler/process state, otherwise timer IRQ re-entry can deadlock the
    // narrow synchronous smoke path before the shell-ready marker is emitted.
    match cpu_interrupts::without_interrupts(|| exec::smoke::run()) {
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
    match cpu_interrupts::without_interrupts(|| exec::smoke::run_abi_read_smoke()) {
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
    match cpu_interrupts::without_interrupts(|| exec::smoke::run_abi_offset_smoke()) {
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
    match cpu_interrupts::without_interrupts(|| exec::smoke::run_abi_argv_smoke()) {
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
    match cpu_interrupts::without_interrupts(|| exec::smoke::run_abi_exec_smoke()) {
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
    match cpu_interrupts::without_interrupts(|| exec::smoke::run_abi_heap_smoke()) {
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

    display::branding::boot_complete_animation();
    display::branding::show_separator();
    kprint_colored!(Colors::DIM, "Boot complete in {} ms.\n", boot_complete_ms);
    boot_notice("WarOS shell online. Type 'help' for available commands.");
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
    kprint_colored!(Colors::RED, "[ERR]");
    kprintln!(" {}", message);
    serial_println!("[ERR] {}", message);
    arch::x86_64::hlt_loop()
}

#[must_use]
pub fn boot_complete_ms() -> u64 {
    BOOT_COMPLETE_MS.load(Ordering::Relaxed)
}

fn boot_elapsed_ms() -> u64 {
    arch::x86_64::pit::elapsed_millis(crate::arch::x86_64::interrupts::tick_count())
}

#[alloc_error_handler]
fn alloc_error(layout: Layout) -> ! {
    fatal(match layout.size() {
        0 => "zero-sized allocation failed",
        _ => "kernel heap exhausted",
    })
}
