use pc_keyboard::{
    layouts, DecodedKey, HandleControl, KeyCode, Keyboard as PcKeyboard, ScancodeSet1,
};
use spin::{Lazy, Mutex};

pub static KEYBOARD: Lazy<Mutex<Keyboard>> = Lazy::new(|| Mutex::new(Keyboard::new()));

/// PS/2 keyboard state and ring buffer for shell input.
pub struct Keyboard {
    inner: PcKeyboard<layouts::Us104Key, ScancodeSet1>,
    buffer: [u8; 256],
    read_pos: usize,
    write_pos: usize,
}

impl Keyboard {
    fn new() -> Self {
        Self {
            inner: PcKeyboard::new(
                ScancodeSet1::new(),
                layouts::Us104Key,
                HandleControl::Ignore,
            ),
            buffer: [0; 256],
            read_pos: 0,
            write_pos: 0,
        }
    }

    fn push(&mut self, byte: u8) {
        let next = (self.write_pos + 1) % self.buffer.len();
        if next != self.read_pos {
            self.buffer[self.write_pos] = byte;
            self.write_pos = next;
        }
    }

    /// Decode a PS/2 scancode and enqueue any resulting character.
    pub fn handle_scancode(&mut self, scancode: u8) {
        if let Ok(Some(event)) = self.inner.add_byte(scancode) {
            if let Some(decoded) = self.inner.process_keyevent(event) {
                if let Some(byte) = key_to_byte(decoded) {
                    self.push(byte);
                }
            }
        }
    }

    /// Read one character from the keyboard input buffer.
    pub fn read_char(&mut self) -> Option<u8> {
        if self.read_pos == self.write_pos {
            return None;
        }

        let byte = self.buffer[self.read_pos];
        self.read_pos = (self.read_pos + 1) % self.buffer.len();
        Some(byte)
    }
}

/// Initialize the keyboard driver state.
pub fn init() {
    *KEYBOARD.lock() = Keyboard::new();
}

/// Called from the IRQ1 handler to decode a hardware scancode.
pub fn handle_scancode(scancode: u8) {
    KEYBOARD.lock().handle_scancode(scancode);
}

/// Read one buffered character for the shell.
pub fn read_char() -> Option<u8> {
    KEYBOARD.lock().read_char()
}

fn key_to_byte(key: DecodedKey) -> Option<u8> {
    match key {
        DecodedKey::Unicode(character) if character.is_ascii() => Some(character as u8),
        DecodedKey::RawKey(KeyCode::Return) | DecodedKey::RawKey(KeyCode::NumpadEnter) => {
            Some(b'\n')
        }
        DecodedKey::RawKey(KeyCode::Backspace) => Some(0x08),
        DecodedKey::RawKey(KeyCode::Tab) => Some(b'\t'),
        DecodedKey::RawKey(KeyCode::Escape) => Some(0x1b),
        _ => None,
    }
}
