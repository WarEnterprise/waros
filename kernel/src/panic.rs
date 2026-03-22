use core::panic::PanicInfo;

use crate::arch::x86_64;
use crate::display::console;
use crate::serial_println;

#[panic_handler]
fn panic(info: &PanicInfo<'_>) -> ! {
    serial_println!("[KERNEL PANIC] {}", info);
    console::try_write_fmt(format_args!("\n[KERNEL PANIC] {}\n", info));
    x86_64::hlt_loop()
}
