#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]
#![feature(alloc_error_handler)]
#![deny(unsafe_op_in_unsafe_fn)]

extern crate alloc;

mod arch;
mod boot;
mod display;
mod drivers;
mod fs;
mod memory;
mod net;
mod panic;
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
pub const BUILD_DATE: &str = "2026-03-22";
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

    shell::run()
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
        memory::heap::init_heap(&mut mapper, frame_allocator)
            .map_err(|_| "kernel heap initialization failed")?;
    }
    boot_ok("Kernel heap: 4 MiB allocated");

    fs::init();
    boot_ok("WarFS: in-memory filesystem ready");

    task::init();
    boot_ok("Task scheduler: cooperative background tasks ready");

    net::init();
    boot_ok("Serial link: COM2 networking ready");

    drivers::keyboard::init();
    boot_ok("Keyboard driver active");
    boot_ok("Quantum subsystem ready (18 qubits max)");

    let boot_complete_ms = boot_elapsed_ms();
    BOOT_COMPLETE_MS.store(boot_complete_ms, Ordering::Relaxed);
    fs::seed_system_files().map_err(|_| "failed to seed filesystem system files")?;

    display::branding::show_separator();
    kprint_colored!(Colors::DIM, "Boot complete in {} ms.\n", boot_complete_ms);
    boot_notice("System ready. Type 'help' for available commands.");
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
