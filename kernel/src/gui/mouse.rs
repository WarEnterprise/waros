use core::sync::atomic::{AtomicBool, Ordering};

use spin::Mutex;

use crate::arch::x86_64::port;

#[derive(Clone, Copy)]
pub struct MouseSnapshot {
    pub x: i32,
    pub y: i32,
    pub left: bool,
    pub right: bool,
    pub middle: bool,
    pub dirty: bool,
}

pub struct MouseState {
    pub x: i32,
    pub y: i32,
    pub left: bool,
    pub right: bool,
    pub middle: bool,
    pub dirty: bool,
    packet: [u8; 3],
    packet_index: usize,
    screen_width: i32,
    screen_height: i32,
}

impl MouseState {
    pub const fn new() -> Self {
        Self {
            x: 640,
            y: 360,
            left: false,
            right: false,
            middle: false,
            dirty: true,
            packet: [0; 3],
            packet_index: 0,
            screen_width: 1280,
            screen_height: 720,
        }
    }

    fn handle_byte(&mut self, byte: u8) {
        if self.packet_index == 0 && (byte & 0x08) == 0 {
            return;
        }

        self.packet[self.packet_index] = byte;
        self.packet_index += 1;

        if self.packet_index == 3 {
            self.packet_index = 0;
            self.process_packet();
        }
    }

    fn process_packet(&mut self) {
        let flags = self.packet[0];
        if flags & 0xC0 != 0 {
            return;
        }

        let mut dx = self.packet[1] as i32;
        let mut dy = self.packet[2] as i32;
        if flags & 0x10 != 0 {
            dx |= !0xFF;
        }
        if flags & 0x20 != 0 {
            dy |= !0xFF;
        }

        self.x = (self.x + dx).clamp(0, self.screen_width.saturating_sub(1));
        self.y = (self.y - dy).clamp(0, self.screen_height.saturating_sub(1));
        self.left = flags & 0x01 != 0;
        self.right = flags & 0x02 != 0;
        self.middle = flags & 0x04 != 0;
        self.dirty = true;
    }

    fn snapshot(&mut self) -> MouseSnapshot {
        let snapshot = MouseSnapshot {
            x: self.x,
            y: self.y,
            left: self.left,
            right: self.right,
            middle: self.middle,
            dirty: self.dirty,
        };
        self.dirty = false;
        snapshot
    }

    fn set_bounds(&mut self, width: i32, height: i32) {
        self.screen_width = width.max(1);
        self.screen_height = height.max(1);
        self.x = self.x.clamp(0, self.screen_width - 1);
        self.y = self.y.clamp(0, self.screen_height - 1);
        self.dirty = true;
    }
}

pub static MOUSE: Mutex<MouseState> = Mutex::new(MouseState::new());
static INITIALIZED: AtomicBool = AtomicBool::new(false);

pub fn init_mouse(screen_width: i32, screen_height: i32) {
    MOUSE.lock().set_bounds(screen_width, screen_height);
    if INITIALIZED.swap(true, Ordering::SeqCst) {
        return;
    }

    wait_ps2_input();
    port::outb(0x64, 0xA8);

    wait_ps2_input();
    port::outb(0x64, 0x20);
    wait_ps2_output();
    let config = port::inb(0x60);

    wait_ps2_input();
    port::outb(0x64, 0x60);
    wait_ps2_input();
    port::outb(0x60, config | 0x02);

    write_mouse_command(0xF6);
    let _ = read_mouse_ack();
    write_mouse_command(0xF4);
    let _ = read_mouse_ack();
}

pub fn handle_byte(byte: u8) {
    MOUSE.lock().handle_byte(byte);
}

#[must_use]
pub fn take_snapshot() -> MouseSnapshot {
    MOUSE.lock().snapshot()
}

fn wait_ps2_input() {
    for _ in 0..100_000 {
        if port::inb(0x64) & 0x02 == 0 {
            return;
        }
        core::hint::spin_loop();
    }
}

fn wait_ps2_output() {
    for _ in 0..100_000 {
        if port::inb(0x64) & 0x01 != 0 {
            return;
        }
        core::hint::spin_loop();
    }
}

fn write_mouse_command(command: u8) {
    wait_ps2_input();
    port::outb(0x64, 0xD4);
    wait_ps2_input();
    port::outb(0x60, command);
}

fn read_mouse_ack() -> u8 {
    wait_ps2_output();
    port::inb(0x60)
}
