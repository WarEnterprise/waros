use x86_64::instructions::port::Port;

/// Read one byte from an x86 I/O port.
#[must_use]
pub fn inb(port: u16) -> u8 {
    let mut port = Port::<u8>::new(port);
    // SAFETY: The caller selects the I/O port. This helper is only used for well-known
    // legacy ports such as COM1 and the PS/2 controller in the kernel.
    unsafe { port.read() }
}

/// Write one byte to an x86 I/O port.
pub fn outb(port: u16, value: u8) {
    let mut port = Port::<u8>::new(port);
    // SAFETY: The caller selects the I/O port. This helper is only used for well-known
    // legacy ports such as COM1, the PS/2 controller, and the PIC in the kernel.
    unsafe {
        port.write(value);
    }
}
