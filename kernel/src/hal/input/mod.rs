#![allow(dead_code)]

use alloc::collections::VecDeque;
use alloc::string::String;

use spin::{Lazy, Mutex};

use crate::drivers::keyboard;
use crate::hal::DEVICES;

use super::device::{
    BusLocation, DeviceCapabilities, DeviceCategory, DeviceId, DeviceInfo, DeviceStatus,
    DriverState, InputCapabilities, KeyboardLayout,
};
use super::traits::{KeyEvent, MouseEvent};

static KEY_QUEUE: Lazy<Mutex<VecDeque<KeyEvent>>> = Lazy::new(|| Mutex::new(VecDeque::new()));
static MOUSE_QUEUE: Lazy<Mutex<VecDeque<MouseEvent>>> = Lazy::new(|| Mutex::new(VecDeque::new()));
static INPUT_DEVICE: Lazy<Mutex<Option<DeviceId>>> = Lazy::new(|| Mutex::new(None));

pub fn init() -> DeviceId {
    let hal_id = DEVICES.lock().register_or_update(
        DeviceInfo {
            name: String::from("PS/2 Keyboard"),
            category: DeviceCategory::Input,
            bus: BusLocation::Platform,
            vendor_id: 0,
            product_id: 0,
            capabilities: DeviceCapabilities::Input(InputCapabilities {
                has_keyboard: true,
                has_pointer: false,
                has_touch: false,
                layout: current_layout(),
            }),
        },
        DriverState::Loaded(String::from("ps2-waros")),
        DeviceStatus::Active,
    );

    *INPUT_DEVICE.lock() = Some(hal_id);
    hal_id
}

#[must_use]
pub fn current_layout() -> KeyboardLayout {
    match keyboard::current_layout() {
        keyboard::KeyboardLayout::UsQwerty => KeyboardLayout::UsQwerty,
        keyboard::KeyboardLayout::BrazilAbnt2 => KeyboardLayout::BrazilAbnt2,
    }
}

pub fn set_layout(layout: KeyboardLayout) -> Result<(), &'static str> {
    match layout {
        KeyboardLayout::UsQwerty => keyboard::set_layout(keyboard::KeyboardLayout::UsQwerty),
        KeyboardLayout::BrazilAbnt2 => {
            keyboard::set_layout(keyboard::KeyboardLayout::BrazilAbnt2);
        }
        KeyboardLayout::German
        | KeyboardLayout::French
        | KeyboardLayout::Japanese
        | KeyboardLayout::UkQwerty
        | KeyboardLayout::Custom => return Err("layout not implemented yet"),
    }

    if let Some(id) = *INPUT_DEVICE.lock() {
        DEVICES.lock().update_capabilities(
            id,
            DeviceCapabilities::Input(InputCapabilities {
                has_keyboard: true,
                has_pointer: false,
                has_touch: false,
                layout,
            }),
        );
    }

    Ok(())
}

pub fn set_layout_by_name(name: &str) -> Result<KeyboardLayout, &'static str> {
    let layout = match name {
        "us" => KeyboardLayout::UsQwerty,
        "br" => KeyboardLayout::BrazilAbnt2,
        "de" => KeyboardLayout::German,
        "fr" => KeyboardLayout::French,
        "jp" => KeyboardLayout::Japanese,
        "uk" => KeyboardLayout::UkQwerty,
        _ => return Err("unknown layout"),
    };

    set_layout(layout)?;
    Ok(layout)
}

#[must_use]
pub fn supported_layouts() -> &'static [(&'static str, &'static str, bool)] {
    &[
        ("us", "US QWERTY", true),
        ("br", "Brazilian ABNT2", true),
        ("de", "German QWERTZ", false),
        ("fr", "French AZERTY", false),
        ("jp", "Japanese", false),
        ("uk", "UK QWERTY", false),
    ]
}

pub fn push_key_event(event: KeyEvent) {
    let mut queue = KEY_QUEUE.lock();
    if queue.len() < 256 {
        queue.push_back(event);
    }
}

pub fn push_mouse_event(event: MouseEvent) {
    let mut queue = MOUSE_QUEUE.lock();
    if queue.len() < 256 {
        queue.push_back(event);
    }
}

#[must_use]
pub fn read_char() -> Option<u8> {
    while let Some(event) = poll_keyboard() {
        if !event.pressed {
            continue;
        }

        if let Some(byte) = keycode_to_ascii(event.keycode, event.shift, current_layout()) {
            return Some(byte);
        }
    }

    keyboard::read_char()
}

#[must_use]
pub fn poll_keyboard() -> Option<KeyEvent> {
    KEY_QUEUE.lock().pop_front()
}

#[must_use]
pub fn poll_mouse() -> Option<MouseEvent> {
    MOUSE_QUEUE.lock().pop_front()
}

#[must_use]
pub fn keycode_to_ascii(keycode: u8, shift: bool, layout: KeyboardLayout) -> Option<u8> {
    match layout {
        KeyboardLayout::UsQwerty => us_qwerty_map(keycode, shift),
        KeyboardLayout::BrazilAbnt2 => abnt2_map(keycode, shift),
        KeyboardLayout::German
        | KeyboardLayout::French
        | KeyboardLayout::Japanese
        | KeyboardLayout::UkQwerty
        | KeyboardLayout::Custom => us_qwerty_map(keycode, shift),
    }
}

fn us_qwerty_map(keycode: u8, shift: bool) -> Option<u8> {
    match keycode {
        0x04..=0x1D => {
            let offset = keycode - 0x04;
            let base = if shift { b'A' } else { b'a' };
            Some(base + offset)
        }
        0x1E => Some(if shift { b'!' } else { b'1' }),
        0x1F => Some(if shift { b'@' } else { b'2' }),
        0x20 => Some(if shift { b'#' } else { b'3' }),
        0x21 => Some(if shift { b'$' } else { b'4' }),
        0x22 => Some(if shift { b'%' } else { b'5' }),
        0x23 => Some(if shift { b'^' } else { b'6' }),
        0x24 => Some(if shift { b'&' } else { b'7' }),
        0x25 => Some(if shift { b'*' } else { b'8' }),
        0x26 => Some(if shift { b'(' } else { b'9' }),
        0x27 => Some(if shift { b')' } else { b'0' }),
        0x28 => Some(b'\n'),
        0x29 => Some(0x1B),
        0x2A => Some(0x08),
        0x2B => Some(b'\t'),
        0x2C => Some(b' '),
        0x2D => Some(if shift { b'_' } else { b'-' }),
        0x2E => Some(if shift { b'+' } else { b'=' }),
        0x2F => Some(if shift { b'{' } else { b'[' }),
        0x30 => Some(if shift { b'}' } else { b']' }),
        0x31 => Some(if shift { b'|' } else { b'\\' }),
        0x33 => Some(if shift { b':' } else { b';' }),
        0x34 => Some(if shift { b'"' } else { b'\'' }),
        0x35 => Some(if shift { b'~' } else { b'`' }),
        0x36 => Some(if shift { b'<' } else { b',' }),
        0x37 => Some(if shift { b'>' } else { b'.' }),
        0x38 => Some(if shift { b'?' } else { b'/' }),
        _ => None,
    }
}

fn abnt2_map(keycode: u8, shift: bool) -> Option<u8> {
    match keycode {
        0x1F => Some(if shift { b'"' } else { b'2' }),
        0x20 => Some(if shift { b'#' } else { b'3' }),
        0x21 => Some(if shift { b'$' } else { b'4' }),
        0x22 => Some(if shift { b'%' } else { b'5' }),
        0x2D => Some(if shift { b'_' } else { b'-' }),
        0x2E => Some(if shift { b'+' } else { b'=' }),
        0x2F => Some(if shift { b'^' } else { b'[' }),
        0x30 => Some(if shift { b'{' } else { b']' }),
        0x33 => Some(if shift { b':' } else { b';' }),
        0x34 => Some(if shift { b'"' } else { b'~' }),
        0x35 => Some(if shift { b'`' } else { b'\'' }),
        _ => us_qwerty_map(keycode, shift),
    }
}
