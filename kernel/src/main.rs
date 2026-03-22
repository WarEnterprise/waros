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
mod memory;
mod panic;
mod shell;

use core::alloc::Layout;

use bootloader_api::config::{BootloaderConfig, Mapping};
use bootloader_api::{entry_point, BootInfo};
use x86_64::instructions::interrupts;

use crate::display::console::{
    self, ACCENT_COLOR, BACKGROUND_COLOR, ERROR_COLOR, INFO_COLOR, OK_COLOR,
};

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

fn try_kernel_main(boot_info: &'static mut BootInfo) -> Result<(), &'static str> {
    drivers::serial::init();
    serial_println!("WarOS: entering kernel bootstrap");

    let boot_context = boot::bootstrap(boot_info)?;
    let framebuffer_info = boot::uefi::framebuffer_info(boot_context.framebuffer);

    console::init(boot_context.framebuffer);
    display_banner();

    status_ok("Serial debug on COM1");
    status_ok_dynamic(format_args!(
        "Framebuffer: {}x{} @ {} bpp",
        framebuffer_info.width,
        framebuffer_info.height,
        framebuffer_info.bytes_per_pixel * 8
    ));

    arch::x86_64::gdt::init();
    status_ok("GDT loaded");

    arch::x86_64::idt::init();
    status_ok("IDT loaded (exceptions + timer + keyboard)");

    unsafe {
        // SAFETY: The PIC is initialized once during early boot before interrupts are enabled.
        arch::x86_64::pic::init();
    }
    status_ok("PIC remapped (IRQ 32-47)");

    memory::init(boot_context.memory_regions)?;
    let stats = memory::stats();
    status_ok_dynamic(format_args!(
        "Physical memory: {} MiB available ({} frames)",
        (stats.free_frames * 4) / 1024,
        stats.free_frames
    ));

    let mut mapper = unsafe {
        // SAFETY: The bootloader mapped physical memory at the configured offset exposed
        // through `boot_info.physical_memory_offset`, which `boot::bootstrap` validated.
        memory::paging::init(boot_context.physical_memory_offset)
    };
    status_ok("Paging: 4-level page tables active");

    {
        let mut allocator_guard = memory::FRAME_ALLOCATOR.lock();
        let Some(frame_allocator) = allocator_guard.as_mut() else {
            return Err("frame allocator missing after initialization");
        };
        memory::heap::init_heap(&mut mapper, frame_allocator)
            .map_err(|_| "kernel heap initialization failed")?;
    }
    status_ok("Kernel heap: 1 MiB allocated");

    drivers::keyboard::init();
    status_ok("Keyboard driver active");

    interrupts::enable();
    status_info("System ready. Type 'help' for commands.");
    println!();
    Ok(())
}

fn display_banner() {
    console::set_colors(ACCENT_COLOR, BACKGROUND_COLOR);
    println!("+------------------------------------------------------+");
    println!("|                                                      |");
    println!("| __        __    _    ____   ___  ____                |");
    println!("| \\ \\      / /_ _| | _/ ___| / _ \\/ ___|               |");
    println!("|  \\ \\ /\\ / / _` | |/ / |  _ | | | \\___ \\              |");
    println!("|   \\ V  V / (_| |   <| |_| || |_| |___) |             |");
    println!("|    \\_/\\_/ \\__,_|_|\\_\\\\____(_)___/|____/              |");
    println!("|                                                      |");
    println!("| Quantum-Classical Hybrid Operating System            |");
    println!("| Version 0.1.0 - War Enterprise (c) 2026              |");
    println!("|                                                      |");
    println!("+------------------------------------------------------+");
    println!();
    console::reset_colors();
}

fn status_ok(message: &str) {
    console::set_colors(OK_COLOR, BACKGROUND_COLOR);
    print!("[OK] ");
    console::reset_colors();
    println!("{message}");
    serial_println!("[OK] {message}");
}

fn status_ok_dynamic(message: core::fmt::Arguments<'_>) {
    console::set_colors(OK_COLOR, BACKGROUND_COLOR);
    print!("[OK] ");
    console::reset_colors();
    console::_print(message);
    println!();
}

fn status_info(message: &str) {
    console::set_colors(INFO_COLOR, BACKGROUND_COLOR);
    print!("[INFO] ");
    console::reset_colors();
    println!("{message}");
    serial_println!("[INFO] {message}");
}

fn fatal(message: &str) -> ! {
    console::set_colors(ERROR_COLOR, BACKGROUND_COLOR);
    println!("[FATAL] {message}");
    console::reset_colors();
    serial_println!("[FATAL] {message}");
    arch::x86_64::hlt_loop()
}

#[alloc_error_handler]
fn alloc_error(layout: Layout) -> ! {
    fatal(match layout.size() {
        0 => "zero-sized allocation failed",
        _ => "kernel heap exhausted",
    })
}
