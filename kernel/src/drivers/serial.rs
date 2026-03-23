use core::fmt;

use spin::Mutex;
use uart_16550::SerialPort;
use x86_64::instructions::interrupts;

pub const COM1_PORT: u16 = 0x3F8;

pub static SERIAL1: Mutex<SerialPort> = Mutex::new(unsafe { SerialPort::new(COM1_PORT) });

/// Initialize the COM1 serial port for kernel logging.
pub fn init() {
    interrupts::without_interrupts(|| {
        SERIAL1.lock().init();
    });
}

/// Print formatted output to the serial port.
pub fn _print(args: fmt::Arguments<'_>) {
    use core::fmt::Write;

    interrupts::without_interrupts(|| {
        let _ = SERIAL1.lock().write_fmt(args);
    });
}

#[macro_export]
macro_rules! serial_print {
    ($($arg:tt)*) => {
        $crate::drivers::serial::_print(format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! serial_println {
    () => {
        $crate::serial_print!("\n")
    };
    ($($arg:tt)*) => {
        $crate::drivers::serial::_print(format_args!("{}\n", format_args!($($arg)*)))
    };
}
