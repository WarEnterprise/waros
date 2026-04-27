use core::fmt::Write;
use core::panic::PanicInfo;

use ::x86_64::instructions::interrupts;

use crate::arch::x86_64;
use crate::boot::trace;
use crate::drivers::serial::COM1_PORT;
use crate::display::console::{Colors, CONSOLE};
use crate::serial_println;

#[panic_handler]
fn panic(info: &PanicInfo<'_>) -> ! {
    interrupts::disable();
    raw_panic_banner();

    if let Some(mut guard) = CONSOLE.try_lock() {
        if let Some(console) = guard.as_mut() {
            console.clear_screen();
            console.set_color(Colors::RED);
            let _ = console.write_str("================================================================================\n");
            let _ = console.write_str("  KERNEL PANIC - WarOS has stopped\n");
            let _ = console.write_str("================================================================================\n");
            console.reset_color();
            let _ = console.write_str("\n");
            let _ = console.write_fmt(format_args!("  Details: {}\n", info));

            if let Some(location) = info.location() {
                let _ = console.write_fmt(format_args!(
                    "  Location: {}:{}\n",
                    location.file(),
                    location.line()
                ));
            }

            let _ = console.write_str("\n");
            let _ = trace::write_panic_breadcrumbs(console);
            let _ = console.write_str("\n");
            console.set_color(Colors::DIM);
            let _ = console.write_str("  System halted. Please reboot.\n");
            let _ = console.write_str("================================================================================\n");
            console.reset_color();
        }
    }

    serial_println!("\n=== KERNEL PANIC ===");
    serial_println!("{}", info);
    trace::write_serial_panic_breadcrumbs();
    x86_64::hlt_loop()
}

fn raw_panic_banner() {
    const MESSAGE: &[u8] = b"\r\n[PANIC-EARLY]\r\n";
    for &byte in MESSAGE {
        crate::arch::x86_64::port::outb(0xE9, byte);
        let mut spin = 0usize;
        while crate::arch::x86_64::port::inb(COM1_PORT + 5) & 0x20 == 0 && spin < 1_000_000 {
            core::hint::spin_loop();
            spin += 1;
        }
        crate::arch::x86_64::port::outb(COM1_PORT, byte);
    }
}
