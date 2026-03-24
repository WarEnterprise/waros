#![allow(dead_code)]

use alloc::format;
use alloc::string::String;

use crate::hal::input;
use crate::hal::traits::{KeyEvent, MouseEvent};
use crate::hal::DEVICES;

use super::super::device::{
    BusLocation, DeviceCapabilities, DeviceCategory, DeviceId, DeviceStatus, DriverState,
    InputCapabilities, KeyboardLayout,
};
use super::descriptors::UsbInterface;

const MODIFIER_USAGE_BASE: u8 = 0xE0;
const LEFT_CTRL: u8 = 1 << 0;
const LEFT_SHIFT: u8 = 1 << 1;
const LEFT_ALT: u8 = 1 << 2;
const RIGHT_CTRL: u8 = 1 << 4;
const RIGHT_SHIFT: u8 = 1 << 5;
const RIGHT_ALT: u8 = 1 << 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HidKind {
    Keyboard,
    Mouse,
    Combined,
    Unknown,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct KeyboardBootState {
    modifiers: u8,
    pressed: [u8; 6],
}

#[must_use]
pub fn classify_interface(interface: &UsbInterface) -> HidKind {
    if interface.boot_keyboard() {
        HidKind::Keyboard
    } else if interface.boot_mouse() {
        HidKind::Mouse
    } else if interface.is_hid() {
        HidKind::Combined
    } else {
        HidKind::Unknown
    }
}

pub fn process_keyboard_boot_report(state: &mut KeyboardBootState, report: &[u8]) -> usize {
    if report.len() < 8 {
        return 0;
    }

    let previous_modifiers = state.modifiers;
    let previous_pressed = state.pressed;
    state.modifiers = report[0];
    state.pressed.copy_from_slice(&report[2..8]);

    let mut emitted = 0usize;
    for bit in 0..8 {
        let mask = 1 << bit;
        if (previous_modifiers ^ state.modifiers) & mask == 0 {
            continue;
        }
        push_key_event(
            MODIFIER_USAGE_BASE + bit,
            state.modifiers & mask != 0,
            state.modifiers,
        );
        emitted += 1;
    }

    for &usage in &state.pressed {
        if usage != 0 && !previous_pressed.contains(&usage) {
            push_key_event(usage, true, state.modifiers);
            emitted += 1;
        }
    }

    for &usage in &previous_pressed {
        if usage != 0 && !state.pressed.contains(&usage) {
            push_key_event(usage, false, state.modifiers);
            emitted += 1;
        }
    }

    emitted
}

pub fn process_mouse_boot_report(report: &[u8]) -> Option<MouseEvent> {
    if report.len() < 3 {
        return None;
    }

    let event = MouseEvent {
        dx: i16::from(report[1] as i8),
        dy: i16::from(report[2] as i8),
        left_button: report[0] & 0x01 != 0,
        right_button: report[0] & 0x02 != 0,
        middle_button: report[0] & 0x04 != 0,
        scroll_delta: report.get(3).copied().map_or(0, |value| value as i8),
    };
    input::push_mouse_event(event);
    Some(event)
}

pub fn register_hid_device(
    controller: DeviceId,
    port: u8,
    address: u8,
    vendor_id: u16,
    product_id: u16,
    name: &str,
    kind: HidKind,
) -> DeviceId {
    let (has_keyboard, has_pointer, driver_name, label) = match kind {
        HidKind::Keyboard => (true, false, "usb-hid-kbd", "USB Keyboard"),
        HidKind::Mouse => (false, true, "usb-hid-mouse", "USB Mouse"),
        HidKind::Combined => (true, true, "usb-hid", "USB HID"),
        HidKind::Unknown => (false, false, "usb-hid", "USB HID"),
    };

    DEVICES.lock().register_or_update(
        crate::hal::device::DeviceInfo {
            name: if name.is_empty() {
                format!("{} (port {} addr {})", label, port, address)
            } else {
                format!("{}: {}", label, name)
            },
            category: DeviceCategory::Input,
            bus: BusLocation::Usb {
                controller,
                port,
                address,
            },
            vendor_id,
            product_id,
            capabilities: DeviceCapabilities::Input(InputCapabilities {
                has_keyboard,
                has_pointer,
                has_touch: false,
                layout: KeyboardLayout::UsQwerty,
            }),
        },
        DriverState::Loaded(String::from(driver_name)),
        DeviceStatus::Active,
    )
}

fn push_key_event(usage: u8, pressed: bool, modifiers: u8) {
    input::push_key_event(KeyEvent {
        scancode: usage,
        keycode: usage,
        pressed,
        shift: modifiers & (LEFT_SHIFT | RIGHT_SHIFT) != 0,
        ctrl: modifiers & (LEFT_CTRL | RIGHT_CTRL) != 0,
        alt: modifiers & (LEFT_ALT | RIGHT_ALT) != 0,
    });
}
