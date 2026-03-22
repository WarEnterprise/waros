use core::panic::PanicInfo;
use core::fmt::Write;

use ::x86_64::instructions::interrupts;

use crate::arch::x86_64;
use crate::display::console::{Colors, CONSOLE};
use crate::serial_println;

#[panic_handler]
fn panic(info: &PanicInfo<'_>) -> ! {
    interrupts::disable();

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
            console.set_color(Colors::DIM);
            let _ = console.write_str("  System halted. Please reboot.\n");
            let _ = console.write_str("================================================================================\n");
            console.reset_color();
        }
    }

    serial_println!("\n=== KERNEL PANIC ===");
    serial_println!("{}", info);
    x86_64::hlt_loop()
}
