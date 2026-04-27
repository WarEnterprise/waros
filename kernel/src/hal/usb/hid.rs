#![allow(dead_code)]

use alloc::format;
use alloc::string::String;
use core::sync::atomic::{AtomicU32, Ordering};

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
const LEFT_GUI: u8 = 1 << 3;
const RIGHT_CTRL: u8 = 1 << 4;
const RIGHT_SHIFT: u8 = 1 << 5;
const RIGHT_ALT: u8 = 1 << 6;
const RIGHT_GUI: u8 = 1 << 7;
const VALID_MODIFIER_BITS: u8 =
    LEFT_CTRL
        | LEFT_SHIFT
        | LEFT_ALT
        | LEFT_GUI
        | RIGHT_CTRL
        | RIGHT_SHIFT
        | RIGHT_ALT
        | RIGHT_GUI;
const BOOT_KEY_SLOT_COUNT: usize = 6;
const HID_TRACE_ENABLED: bool = false;

static HID_REPORT_TRACE_COUNT: AtomicU32 = AtomicU32::new(0);
static HID_KEY_TRACE_COUNT: AtomicU32 = AtomicU32::new(0);

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
    // 0 => report starts at byte 0, 1 => report has leading report-id byte.
    report_offset: Option<u8>,
}

impl KeyboardBootState {
    pub fn reset(&mut self) {
        self.modifiers = 0;
        self.pressed = [0; BOOT_KEY_SLOT_COUNT];
        self.report_offset = None;
    }
}

#[derive(Debug, Clone, Copy)]
pub struct HidTraceContext {
    pub trace_id: u64,
    pub slot_id: u8,
    pub endpoint_id: u8,
    pub completion_code: u8,
    pub report_len: usize,
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

pub fn process_keyboard_boot_report(
    state: &mut KeyboardBootState,
    report: &[u8],
    trace: HidTraceContext,
) -> usize {
    if report.len() < 8 {
        if HID_TRACE_ENABLED {
            crate::serial_println!("[kbd] hid report TOO SHORT len={}", report.len());
        }
        // Reset parser state so stale pressed/modifier data cannot survive
        // malformed/short reports after reconnects or transfer glitches.
        state.reset();
        return 0;
    }

    let Some(report) = select_boot_report_view(state, report) else {
        state.reset();
        return 0;
    };

    let previous_modifiers = state.modifiers;
    let previous_pressed = state.pressed;
    state.modifiers = report[0] & VALID_MODIFIER_BITS;
    state.pressed = normalize_pressed_keys(&report[2..8]);
    let changed = previous_modifiers != state.modifiers || previous_pressed != state.pressed;

    let mut emitted = 0usize;
    for bit in 0..8 {
        let mask = 1 << bit;
        if (previous_modifiers ^ state.modifiers) & mask == 0 {
            continue;
        }
        push_key_event(
            trace,
            MODIFIER_USAGE_BASE + bit,
            state.modifiers & mask != 0,
            state.modifiers,
        );
        emitted += 1;
    }

    for &usage in &state.pressed {
        if usage != 0 && !previous_pressed.contains(&usage) {
            push_key_event(trace, usage, true, state.modifiers);
            emitted += 1;
        }
    }

    for &usage in &previous_pressed {
        if usage != 0 && !state.pressed.contains(&usage) {
            push_key_event(trace, usage, false, state.modifiers);
            emitted += 1;
        }
    }

    if changed && HID_TRACE_ENABLED {
        let n = HID_REPORT_TRACE_COUNT.fetch_add(1, Ordering::Relaxed);
        if emitted > 0 || n < 8 || n & 0x3F == 0 {
            crate::serial_println!(
                "[kbd] hid report change len={} emitted={} mod=0x{:02X} keys=[{:02X} {:02X} {:02X} {:02X} {:02X} {:02X}]",
                report.len(),
                emitted,
                state.modifiers,
                state.pressed[0],
                state.pressed[1],
                state.pressed[2],
                state.pressed[3],
                state.pressed[4],
                state.pressed[5]
            );
        }
    }

    emitted
}

fn select_boot_report_view<'a>(state: &mut KeyboardBootState, report: &'a [u8]) -> Option<&'a [u8]> {
    let valid0 = is_valid_boot_view(report, 0);
    let valid1 = is_valid_boot_view(report, 1);

    if let Some(offset) = state.report_offset {
        if is_valid_boot_view(report, offset) {
            let start = usize::from(offset);
            return Some(&report[start..start + 8]);
        }
        let alt = if offset == 0 { 1 } else { 0 };
        if is_valid_boot_view(report, alt) {
            state.report_offset = Some(alt);
            let start = usize::from(alt);
            return Some(&report[start..start + 8]);
        }
        state.report_offset = None;
    }

    match (valid0, valid1) {
        (true, false) => {
            state.report_offset = Some(0);
            Some(&report[..8])
        }
        (false, true) => {
            state.report_offset = Some(1);
            Some(&report[1..9])
        }
        (true, true) => {
            let keys0_nonzero = report[2..8].iter().any(|usage| *usage != 0);
            let keys1_nonzero = report[3..9].iter().any(|usage| *usage != 0);

            let choose = if keys0_nonzero && !keys1_nonzero {
                Some(0u8)
            } else if keys1_nonzero && !keys0_nonzero {
                Some(1u8)
            } else if report.len() >= 9
                && report[2] == 0
                && (report[1] & !VALID_MODIFIER_BITS) == 0
                && report[0] != 0
                && report[0].count_ones() <= 2
                && (keys1_nonzero || (!keys0_nonzero && !keys1_nonzero))
            {
                // Prefer [report-id, modifiers, reserved, keys...] for 9-byte reports.
                // This avoids treating the report-id byte as a stuck modifier on idle frames.
                Some(1u8)
            } else {
                None
            };

            if let Some(offset) = choose {
                state.report_offset = Some(offset);
                let start = usize::from(offset);
                Some(&report[start..start + 8])
            } else {
                // Ambiguous frame: choose a deterministic view instead of
                // dropping input. This avoids long stalls/stuck modifiers on
                // keyboards that alternate between 8-byte and 9-byte framing.
                let fallback = if report.len() >= 9 { 1u8 } else { 0u8 };
                state.report_offset = Some(fallback);
                let start = usize::from(fallback);
                Some(&report[start..start + 8])
            }
        }
        (false, false) => None,
    }
}

fn is_valid_boot_view(report: &[u8], offset: u8) -> bool {
    let start = usize::from(offset);
    if report.len() < start + 8 {
        return false;
    }
    let view = &report[start..start + 8];
    if view[0] & !VALID_MODIFIER_BITS != 0 {
        return false;
    }
    if view[1] != 0 {
        return false;
    }

    // Accept only keyboard usage-page key slots (exclude modifier usages in the
    // 0xE0..=0xE7 range because they must come from the modifier byte).
    view[2..8]
        .iter()
        .all(|usage| *usage == 0 || (*usage >= 0x04 && *usage <= 0xDF))
}

fn normalize_pressed_keys(slots: &[u8]) -> [u8; BOOT_KEY_SLOT_COUNT] {
    let mut normalized = [0u8; BOOT_KEY_SLOT_COUNT];
    let mut next = 0usize;
    for &usage in slots {
        if usage < 0x04 || usage > 0xDF {
            continue;
        }
        if normalized.contains(&usage) {
            continue;
        }
        if next >= BOOT_KEY_SLOT_COUNT {
            break;
        }
        normalized[next] = usage;
        next += 1;
    }
    normalized
}

pub fn reset_keyboard_boot_state(state: &mut KeyboardBootState) {
    state.reset();
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

fn push_key_event(trace: HidTraceContext, usage: u8, pressed: bool, modifiers: u8) {
    if HID_TRACE_ENABLED {
        let n = HID_KEY_TRACE_COUNT.fetch_add(1, Ordering::Relaxed);
        if pressed || n < 16 || n & 0x3F == 0 {
            crate::serial_println!(
                "[kbd] hid key usage=0x{:02X} pressed={} mod=0x{:02X} n={}",
                usage,
                if pressed { "true" } else { "false" },
                modifiers,
                n
            );
        }
    }
    let shift = modifiers & (LEFT_SHIFT | RIGHT_SHIFT) != 0;
    input::trace_stage_decode(
        trace.trace_id,
        input::INPUT_SOURCE_USB,
        usage,
        pressed,
        if pressed {
            input::keycode_to_ascii(usage, shift, input::current_layout())
        } else {
            None
        },
    );
    input::push_key_event_traced(trace.trace_id, input::INPUT_SOURCE_USB, KeyEvent {
        scancode: usage,
        keycode: usage,
        pressed,
        shift,
        ctrl: modifiers & (LEFT_CTRL | RIGHT_CTRL) != 0,
        alt: modifiers & (LEFT_ALT | RIGHT_ALT) != 0,
    });
}
