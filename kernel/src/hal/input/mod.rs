#![allow(dead_code)]

use alloc::collections::VecDeque;
use alloc::string::String;
use core::fmt;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, AtomicU64, Ordering};

use spin::{Lazy, Mutex};

use crate::drivers::keyboard;
use crate::hal::DEVICES;

use super::device::{
    BusLocation, DeviceCapabilities, DeviceCategory, DeviceId, DeviceInfo, DeviceStatus,
    DriverState, InputCapabilities, KeyboardLayout,
};
use super::traits::{KeyEvent, MouseEvent};

struct QueuedKeyEvent {
    trace_id: u64,
    source: u8,
    event: KeyEvent,
}

#[derive(Clone, Copy)]
pub struct InputKeypress {
    pub trace_id: u64,
    pub source: u8,
    pub keycode: u8,
    pub byte: u8,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

static KEY_QUEUE: Lazy<Mutex<VecDeque<QueuedKeyEvent>>> =
    Lazy::new(|| Mutex::new(VecDeque::new()));
static MOUSE_QUEUE: Lazy<Mutex<VecDeque<MouseEvent>>> = Lazy::new(|| Mutex::new(VecDeque::new()));
static INPUT_DEVICE: Lazy<Mutex<Option<DeviceId>>> = Lazy::new(|| Mutex::new(None));
static INPUT_WAIT_FALLBACK_LOGGED: AtomicBool = AtomicBool::new(false);
static INPUT_PS2_RECOVERY_ATTEMPTED: AtomicBool = AtomicBool::new(false);
static INPUT_USB_RESCAN_ATTEMPTED: AtomicBool = AtomicBool::new(false);
static INPUT_PS2_DECODE_RECOVERY_TICK: AtomicU64 = AtomicU64::new(0);
static INPUT_STALL_COUNT: AtomicU32 = AtomicU32::new(0);
static INPUT_QUEUE_DROPPED: AtomicU32 = AtomicU32::new(0);
static INPUT_TRACE_SEQUENCE: AtomicU64 = AtomicU64::new(1);
const INPUT_TRACE_ENABLED: bool = false;
const INPUT_USB_RESCAN_PERIOD_STALLS: u32 = 5000;
const INPUT_USB_TOPOLOGY_RESCAN_PERIOD_STALLS: u32 = 300;
const MAX_USB_EVENTS_PER_READ: usize = 64;
const SHELL_WAIT_SPIN_RETRIES: usize = 512;
const SHELL_WAIT_SPIN_PASSES: usize = 3;
const SHELL_FAST_WAIT_SPIN_RETRIES: usize = 2048;
const SHELL_FAST_WAIT_PASSES: usize = 4;
const SHELL_EMPTY_PROMPT_WAIT_SPIN_RETRIES: usize = 1024;
const SHELL_EMPTY_PROMPT_WAIT_PASSES: usize = 4;
const SHELL_PROMPT_WARM_WAIT_SPIN_RETRIES: usize = 4096;
const SHELL_PROMPT_WARM_WAIT_PASSES: usize = 8;

pub const KEY_ARROW_UP: u8 = 0x80;
pub const KEY_ARROW_DOWN: u8 = 0x81;
pub const KEY_ARROW_LEFT: u8 = 0x82;
pub const KEY_ARROW_RIGHT: u8 = 0x83;
pub const INPUT_SOURCE_NONE: u8 = 0;
pub const INPUT_SOURCE_PS2: u8 = 1;
pub const INPUT_SOURCE_USB: u8 = 2;

pub fn init() -> DeviceId {
    INPUT_PS2_RECOVERY_ATTEMPTED.store(false, Ordering::Relaxed);
    INPUT_USB_RESCAN_ATTEMPTED.store(false, Ordering::Relaxed);
    INPUT_PS2_DECODE_RECOVERY_TICK.store(0, Ordering::Relaxed);
    INPUT_STALL_COUNT.store(0, Ordering::Relaxed);
    INPUT_QUEUE_DROPPED.store(0, Ordering::Relaxed);
    INPUT_WAIT_FALLBACK_LOGGED.store(false, Ordering::Relaxed);
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
        KeyboardLayout::BrazilAbnt2
        | KeyboardLayout::German
        | KeyboardLayout::Spanish
        | KeyboardLayout::French
        | KeyboardLayout::Japanese
        | KeyboardLayout::UkQwerty
        | KeyboardLayout::Custom => return Err("layout not implemented across supported keyboard paths yet"),
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
        "es" => KeyboardLayout::Spanish,
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
        ("br", "Brazilian ABNT2", false),
        ("de", "German QWERTZ", false),
        ("es", "Spanish", false),
        ("fr", "French AZERTY", false),
        ("jp", "Japanese", false),
        ("uk", "UK QWERTY", false),
    ]
}

pub fn push_key_event(event: KeyEvent) {
    push_key_event_traced(next_trace_id(), INPUT_SOURCE_USB, event);
}

pub fn push_key_event_traced(trace_id: u64, source: u8, event: KeyEvent) {
    if event.keycode == 0 {
        return;
    }
    let source = match source {
        INPUT_SOURCE_PS2 | INPUT_SOURCE_USB => source,
        _ => INPUT_SOURCE_NONE,
    };

    // Reset stall counter whenever real input arrives
    INPUT_STALL_COUNT.store(0, Ordering::Relaxed);
    // Track source for diagnostic overlay
    LAST_INPUT_SOURCE.store(source, Ordering::Relaxed);
    LAST_INPUT_KEYCODE.store(event.keycode, Ordering::Relaxed);
    let mut queue = KEY_QUEUE.lock();
    if queue.len() < 256 {
        trace_stage_queue_push(trace_id, source, event.keycode, event.pressed, queue.len() + 1);
        queue.push_back(QueuedKeyEvent { trace_id, source, event });
    } else {
        INPUT_QUEUE_DROPPED.fetch_add(1, Ordering::Relaxed);
        if INPUT_TRACE_ENABLED {
            crate::serial_println!(
                "[INPUT_STAGE_QUEUE_PUSH] id={} source={} dropped=queue-full keycode=0x{:02X} pressed={} depth={}",
                trace_id,
                source_label(source),
                event.keycode,
                if event.pressed { "true" } else { "false" },
                queue.len()
            );
        }
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
    // Delegate to try_read_char which handles both USB and PS/2.
    try_read_char()
}

#[must_use]
pub fn input_poll() -> bool {
    // Runtime USB poll is lightweight when no events are pending.
    crate::hal::usb::poll_runtime();
    let mut activity = !KEY_QUEUE.lock().is_empty();
    if keyboard::poll_hardware() {
        activity = true;
    }
    if keyboard::has_buffered_input() {
        activity = true;
    }
    activity
}

pub fn wait_for_activity() {
    // Layer 1: Poll USB for HID events (always — this is the primary path on modern hardware)
    if input_poll() {
        return;
    }

    let snapshot = keyboard::debug_snapshot();
    let stall_count = INPUT_STALL_COUNT.fetch_add(1, Ordering::Relaxed);
    let ps2_primary_path_healthy = snapshot.i8042_present && !snapshot.i8042_init_failed;

    // Layer 3: PS/2 recovery attempt (once) — only if i8042 is present
    if snapshot.i8042_present
        && snapshot.irq_scancodes == 0
        && snapshot.polled_scancodes == 0
        && !INPUT_PS2_RECOVERY_ATTEMPTED.swap(true, Ordering::Relaxed)
    {
        keyboard::ensure_controller_ready("input-wait-no-ps2-activity");
        let _ = keyboard::poll_hardware();
        if INPUT_TRACE_ENABLED {
            crate::serial_println!("[input] no PS/2 activity yet; controller rearm attempted");
        }
    }

    // Layer 3b: PS/2 decode-state recovery.
    // If raw scancodes are flowing but no byte has ever been translated, the
    // controller/decoder may be out of sync (common on some notebooks after
    // firmware handoff). Reset decoder state in process context and rearm once
    // per second.
    if snapshot.i8042_present
        && !snapshot.i8042_init_failed
        && snapshot.translated_bytes == 0
        && (snapshot.irq_scancodes != 0 || snapshot.polled_scancodes != 0)
    {
        let now_tick = crate::arch::x86_64::interrupts::tick_count();
        let last_tick = INPUT_PS2_DECODE_RECOVERY_TICK.load(Ordering::Relaxed);
        if now_tick.saturating_sub(last_tick) >= 100 {
            INPUT_PS2_DECODE_RECOVERY_TICK.store(now_tick, Ordering::Relaxed);
            keyboard::recover_decode_state("input-wait-no-translated-bytes");
            keyboard::ensure_controller_ready("input-wait-no-translated-bytes");
            let _ = keyboard::poll_hardware();
        }
    }

    // Layer 4: If no i8042 (or failed init), trigger USB recovery periodically.
    // First attempt is immediate; subsequent attempts every 200 stall cycles.
    // Use controller reprobe when no controller/keyboard is currently tracked.
    if !ps2_primary_path_healthy {
        let first_try = !INPUT_USB_RESCAN_ATTEMPTED.swap(true, Ordering::Relaxed);
        let periodic =
            stall_count > 0 && stall_count % INPUT_USB_RESCAN_PERIOD_STALLS == 0;
        if first_try || periodic {
            let controllers_before = crate::hal::usb::controller_count();
            let (hid_total_before, hid_armed_before) = crate::hal::usb::hid_keyboard_count();
            if INPUT_TRACE_ENABLED {
                crate::serial_println!(
                    "[input] no-input failsafe: usb recover stall_count={} ps2_present={} ps2_failed={} usb_ctrl={} hid_kbd={} hid_armed={}",
                    stall_count,
                    snapshot.i8042_present,
                    snapshot.i8042_init_failed,
                    controllers_before,
                    hid_total_before,
                    hid_armed_before
                );
            }
            if controllers_before == 0 || hid_total_before == 0 {
                let reprobed = crate::hal::usb::probe_controllers();
                if INPUT_TRACE_ENABLED {
                    crate::serial_println!(
                        "[input] no-input failsafe: usb probe controllers={}",
                        reprobed
                    );
                }
            }
            crate::hal::usb::poll_runtime();
        }
    }

    // Layer 4b: USB topology recovery only when PS/2 is unavailable/failed.
    // Keep auth/login input loops free from heavy USB port scans while PS/2
    // remains healthy; pending rescans are serviced outside the input hot path.
    let periodic_usb_topology_recovery =
        !ps2_primary_path_healthy
            && stall_count > 0
            && stall_count % INPUT_USB_TOPOLOGY_RESCAN_PERIOD_STALLS == 0;
    if periodic_usb_topology_recovery {
        let usb_controllers = crate::hal::usb::controller_count();
        let (usb_hid_keyboards, usb_hid_armed) = crate::hal::usb::hid_keyboard_count();
        if usb_controllers == 0 || usb_hid_keyboards == 0 {
            let _ = crate::hal::usb::probe_controllers();
        } else if usb_hid_armed == 0 {
            let _ = crate::hal::usb::service_pending_topology();
            crate::hal::usb::poll_runtime();
        }
    }

    // Layer 5: Forced PS/2 polling window (if applicable)
    if snapshot.i8042_present
        && (keyboard::polling_mode_forced()
            || keyboard::maybe_force_polling_mode("wait-for-activity"))
        && keyboard::poll_for_input_window(8)
    {
        return;
    }

    // Give input a few fast retries before yielding for a full tick.
    // This trims human-visible lag without introducing heavy background work.
    if stall_count < 3 {
        for _ in 0..4 {
            core::hint::spin_loop();
            if input_poll() {
                return;
            }
        }
    }

    // Layer 6: Brief wait via timer tick (prevents tight spin consuming 100% CPU)
    let start_tick = crate::arch::x86_64::interrupts::tick_count();
    let waited = crate::arch::x86_64::pit::wait_for_tick_advance(start_tick, 1);
    if INPUT_TRACE_ENABLED
        && !waited
        && !INPUT_WAIT_FALLBACK_LOGGED.swap(true, Ordering::Relaxed)
    {
        crate::serial_println!(
            "[input] wait loop: timer IRQ not advancing; continuing with polled retry"
        );
    }

    // Log stall status periodically (every 500 iterations ≈ few seconds)
    if INPUT_TRACE_ENABLED && (stall_count == 500 || stall_count == 2000) {
        crate::serial_println!(
            "[input] stall_count={} ps2_present={} ps2_init_failed={} irq_count={} polled_count={} usb_queue_len={}",
            stall_count,
            snapshot.i8042_present,
            snapshot.i8042_init_failed,
            snapshot.irq_scancodes,
            snapshot.polled_scancodes,
            KEY_QUEUE.lock().len()
        );
    }
}

/// Lightweight wait profile used only while the shell prompt owns the screen.
/// Keep the prompt responsive by avoiding heavy recovery/reprobe paths here.
pub fn wait_for_shell_activity() {
    if input_poll() || has_pending_input() {
        return;
    }

    for _ in 0..SHELL_WAIT_SPIN_PASSES {
        for _ in 0..SHELL_WAIT_SPIN_RETRIES {
            core::hint::spin_loop();
            if input_poll() || has_pending_input() {
                return;
            }
        }
        if input_poll() || has_pending_input() {
            return;
        }
    }

    let start_tick = crate::arch::x86_64::interrupts::tick_count();
    let _ = crate::arch::x86_64::pit::wait_for_tick_advance(start_tick, 1);
    let _ = input_poll();
    for _ in 0..SHELL_WAIT_SPIN_RETRIES {
        core::hint::spin_loop();
        if input_poll() || has_pending_input() {
            return;
        }
    }
}

/// Aggressive shell-only wait profile used while a command line is active or
/// immediately after shell input. Stay in userspace-facing fast mode longer
/// before yielding for a full tick.
pub fn wait_for_shell_activity_fast() {
    if input_poll() || has_pending_input() {
        return;
    }

    for _ in 0..SHELL_FAST_WAIT_PASSES {
        for _ in 0..SHELL_FAST_WAIT_SPIN_RETRIES {
            core::hint::spin_loop();
            if input_poll() || has_pending_input() {
                return;
            }
        }
        if input_poll() || has_pending_input() {
            return;
        }
    }

    let start_tick = crate::arch::x86_64::interrupts::tick_count();
    let _ = crate::arch::x86_64::pit::wait_for_tick_advance(start_tick, 1);
    let _ = input_poll();
}

/// Semi-hot wait profile used only while the shell prompt is empty but still
/// armed for a bounded period after reprompt or clearing the line.
pub fn wait_for_shell_activity_empty_prompt() {
    if input_poll() || has_pending_input() {
        return;
    }

    for _ in 0..SHELL_EMPTY_PROMPT_WAIT_PASSES {
        for _ in 0..SHELL_EMPTY_PROMPT_WAIT_SPIN_RETRIES {
            core::hint::spin_loop();
            if input_poll() || has_pending_input() {
                return;
            }
        }
        if input_poll() || has_pending_input() {
            return;
        }
    }

    let start_tick = crate::arch::x86_64::interrupts::tick_count();
    let _ = crate::arch::x86_64::pit::wait_for_tick_advance(start_tick, 1);
    let _ = input_poll();
}

/// Post-reprompt wait profile used only while the shell prompt is in its
/// bounded warm state. This stays more aggressive than cold idle without
/// touching global input or boot-stage behavior.
pub fn wait_for_shell_activity_prompt_warm() {
    if input_poll() || has_pending_input() {
        return;
    }

    for _ in 0..SHELL_PROMPT_WARM_WAIT_PASSES {
        for _ in 0..SHELL_PROMPT_WARM_WAIT_SPIN_RETRIES {
            core::hint::spin_loop();
            if input_poll() || has_pending_input() {
                return;
            }
        }
        if input_poll() || has_pending_input() {
            return;
        }
    }

    let start_tick = crate::arch::x86_64::interrupts::tick_count();
    let _ = crate::arch::x86_64::pit::wait_for_tick_advance(start_tick, 1);
    let _ = input_poll();
    for _ in 0..SHELL_WAIT_SPIN_RETRIES {
        core::hint::spin_loop();
        if input_poll() || has_pending_input() {
            return;
        }
    }
}

#[must_use]
fn poll_keyboard() -> Option<QueuedKeyEvent> {
    KEY_QUEUE.lock().pop_front()
}

#[must_use]
pub fn poll_mouse() -> Option<MouseEvent> {
    MOUSE_QUEUE.lock().pop_front()
}

/// Live diagnostic snapshot for on-screen display — no heap, no format!.
#[derive(Clone, Copy)]
pub struct InputDiagnostic {
    pub ticks: u64,
    pub interrupts_enabled: bool,
    pub ps2_present: bool,
    pub ps2_init_failed: bool,
    pub ps2_irq_count: u32,
    pub ps2_polled_count: u32,
    pub ps2_translated: u32,
    pub ps2_consumed: u32,
    pub ps2_last_status: u8,
    pub ps2_last_scancode: u8,
    pub ps2_last_byte: u8,
    pub ps2_forced_polling: bool,
    pub ps2_translation_enabled: bool,
    pub ps2_dynamic_switch: bool,
    pub ps2_active_set: u8,
    pub ps2_selected_set: u8,
    pub ps2_prefix_e0_before: bool,
    pub ps2_prefix_f0_before: bool,
    pub ps2_set1_byte: Option<u8>,
    pub ps2_set2_byte: Option<u8>,
    pub ps2_fallback_byte: Option<u8>,
    pub ps2_emitted_from_fallback: bool,
    pub key_queue_len: usize,
    pub usb_controllers: usize,
    pub usb_hid_keyboards: usize,
    pub usb_hid_armed: usize,
    pub stall_count: u32,
    pub dropped_key_events: u32,
    pub last_source: u8, // 0=none, 1=ps2, 2=usb
    pub last_keycode: u8,
}

/// Collect a point-in-time snapshot of the entire input subsystem for on-screen display.
#[must_use]
pub fn diagnostic_snapshot() -> InputDiagnostic {
    let kbd_snap = keyboard::debug_snapshot();
    let queue_len = KEY_QUEUE.lock().len();
    let usb_count = crate::hal::usb::controller_count();
    let (hid_total, hid_armed) = crate::hal::usb::hid_keyboard_count();
    InputDiagnostic {
        ticks: crate::arch::x86_64::interrupts::tick_count(),
        interrupts_enabled: x86_64::instructions::interrupts::are_enabled(),
        ps2_present: kbd_snap.i8042_present,
        ps2_init_failed: kbd_snap.i8042_init_failed,
        ps2_irq_count: kbd_snap.irq_scancodes,
        ps2_polled_count: kbd_snap.polled_scancodes,
        ps2_translated: kbd_snap.translated_bytes,
        ps2_consumed: kbd_snap.consumed_bytes,
        ps2_last_status: kbd_snap.last_status,
        ps2_last_scancode: kbd_snap.last_scancode,
        ps2_last_byte: kbd_snap.last_byte,
        ps2_forced_polling: kbd_snap.forced_polling,
        ps2_translation_enabled: kbd_snap.ps2_translation_enabled,
        ps2_dynamic_switch: kbd_snap.ps2_dynamic_switch,
        ps2_active_set: kbd_snap.ps2_active_set,
        ps2_selected_set: kbd_snap.ps2_selected_set,
        ps2_prefix_e0_before: kbd_snap.ps2_prefix_e0_before,
        ps2_prefix_f0_before: kbd_snap.ps2_prefix_f0_before,
        ps2_set1_byte: kbd_snap.ps2_set1_byte,
        ps2_set2_byte: kbd_snap.ps2_set2_byte,
        ps2_fallback_byte: kbd_snap.ps2_fallback_byte,
        ps2_emitted_from_fallback: kbd_snap.ps2_emitted_from_fallback,
        key_queue_len: queue_len,
        usb_controllers: usb_count,
        usb_hid_keyboards: hid_total,
        usb_hid_armed: hid_armed,
        stall_count: INPUT_STALL_COUNT.load(Ordering::Relaxed),
        dropped_key_events: INPUT_QUEUE_DROPPED.load(Ordering::Relaxed),
        last_source: LAST_INPUT_SOURCE.load(Ordering::Relaxed),
        last_keycode: LAST_INPUT_KEYCODE.load(Ordering::Relaxed),
    }
}

/// Non-blocking read with explicit USB + PS/2 aggressive poll.
/// Returns Some(byte) if a key was consumed, None otherwise.
/// Does NOT block — caller controls the loop.
#[must_use]
pub fn has_pending_input() -> bool {
    !KEY_QUEUE.lock().is_empty() || keyboard::has_buffered_input()
}

#[must_use]
pub fn try_read_keypress() -> Option<InputKeypress> {
    let _ = input_poll();

    let usb_pending = !KEY_QUEUE.lock().is_empty();
    let ps2_pending = keyboard::has_buffered_input();

    // When both sources are active, alternate preference by last delivered
    // source to prevent one path from starving the other.
    if usb_pending && ps2_pending {
        let last = LAST_INPUT_SOURCE.load(Ordering::Relaxed);
        if last == INPUT_SOURCE_USB {
            if let Some(keypress) = pop_ps2_keypress() {
                return Some(keypress);
            }
            if let Some(keypress) = pop_usb_keypress(MAX_USB_EVENTS_PER_READ) {
                return Some(keypress);
            }
        } else {
            if let Some(keypress) = pop_usb_keypress(MAX_USB_EVENTS_PER_READ) {
                return Some(keypress);
            }
            if let Some(keypress) = pop_ps2_keypress() {
                return Some(keypress);
            }
        }
    } else {
        if let Some(keypress) = pop_usb_keypress(MAX_USB_EVENTS_PER_READ) {
            return Some(keypress);
        }
        if let Some(keypress) = pop_ps2_keypress() {
            return Some(keypress);
        }
    }

    // One follow-up poll in-process before reporting idle.
    let _ = input_poll();
    if let Some(keypress) = pop_usb_keypress(MAX_USB_EVENTS_PER_READ) {
        return Some(keypress);
    }
    pop_ps2_keypress()
}

fn pop_usb_keypress(max_events: usize) -> Option<InputKeypress> {
    let mut scanned = 0usize;
    while scanned < max_events {
        let Some(event) = poll_keyboard() else {
            break;
        };
        scanned = scanned.saturating_add(1);
        if !event.event.pressed {
            continue;
        }
        if let Some(byte) =
            keycode_to_ascii(event.event.keycode, event.event.shift, current_layout())
        {
            LAST_INPUT_SOURCE.store(event.source, Ordering::Relaxed);
            LAST_INPUT_KEYCODE.store(event.event.keycode, Ordering::Relaxed);
            trace_stage_queue_pop(
                event.trace_id,
                event.source,
                event.event.keycode,
                byte,
                KEY_QUEUE.lock().len(),
            );
            return Some(InputKeypress {
                trace_id: event.trace_id,
                source: event.source,
                keycode: event.event.keycode,
                byte,
                shift: event.event.shift,
                ctrl: event.event.ctrl,
                alt: event.event.alt,
            });
        }
        if INPUT_TRACE_ENABLED {
            crate::serial_println!(
                "[INPUT_STAGE_QUEUE_POP] id={} source={} dropped=unmapped keycode=0x{:02X} depth={}",
                event.trace_id,
                source_label(event.source),
                event.event.keycode,
                KEY_QUEUE.lock().len()
            );
        }
    }
    None
}

fn pop_ps2_keypress() -> Option<InputKeypress> {
    if keyboard::polling_mode_forced() || keyboard::maybe_force_polling_mode("try-read") {
        let _ = keyboard::poll_for_input_window(2);
    }
    let keypress = keyboard::read_keypress()?;
    LAST_INPUT_SOURCE.store(INPUT_SOURCE_PS2, Ordering::Relaxed);
    LAST_INPUT_KEYCODE.store(keypress.byte, Ordering::Relaxed);
    Some(InputKeypress {
        trace_id: keypress.trace_id,
        source: INPUT_SOURCE_PS2,
        keycode: keypress.byte,
        byte: keypress.byte,
        shift: keypress.shift,
        ctrl: keypress.ctrl,
        alt: keypress.alt,
    })
}

#[must_use]
pub fn try_read_char() -> Option<u8> {
    try_read_keypress().map(|keypress| keypress.byte)
}

pub fn trace_stage_hw(trace_id: u64, source: u8, detail: &str) {
    if !INPUT_TRACE_ENABLED {
        let _ = (trace_id, source, detail);
        return;
    }
    crate::serial_println!(
        "[INPUT_STAGE_HW] id={} source={} {}",
        trace_id,
        source_label(source),
        detail
    );
}

pub fn trace_stage_decode(
    trace_id: u64,
    source: u8,
    keycode: u8,
    pressed: bool,
    byte: Option<u8>,
) {
    if !INPUT_TRACE_ENABLED {
        let _ = (trace_id, source, keycode, pressed, byte);
        return;
    }
    crate::serial_println!(
        "[INPUT_STAGE_DECODE] id={} source={} keycode=0x{:02X} pressed={} byte={}",
        trace_id,
        source_label(source),
        keycode,
        if pressed { "true" } else { "false" },
        format_byte(byte)
    );
}

pub fn trace_stage_queue_push(
    trace_id: u64,
    source: u8,
    keycode: u8,
    pressed: bool,
    depth: usize,
) {
    if !INPUT_TRACE_ENABLED {
        let _ = (trace_id, source, keycode, pressed, depth);
        return;
    }
    crate::serial_println!(
        "[INPUT_STAGE_QUEUE_PUSH] id={} source={} keycode=0x{:02X} pressed={} depth={}",
        trace_id,
        source_label(source),
        keycode,
        if pressed { "true" } else { "false" },
        depth
    );
}

pub fn trace_stage_queue_pop(
    trace_id: u64,
    source: u8,
    keycode: u8,
    byte: u8,
    depth: usize,
) {
    if !INPUT_TRACE_ENABLED {
        let _ = (trace_id, source, keycode, byte, depth);
        return;
    }
    crate::serial_println!(
        "[INPUT_STAGE_QUEUE_POP] id={} source={} keycode=0x{:02X} byte=0x{:02X} '{}' depth={}",
        trace_id,
        source_label(source),
        keycode,
        byte,
        printable(byte),
        depth
    );
}

pub fn trace_stage_ui(trace_id: u64, source: u8, stage: &str, consumer: &str, byte: u8) {
    if !INPUT_TRACE_ENABLED {
        let _ = (trace_id, source, stage, consumer, byte);
        return;
    }
    crate::serial_println!(
        "[INPUT_STAGE_UI] id={} source={} stage={} consumer={} byte=0x{:02X} '{}'",
        trace_id,
        source_label(source),
        stage,
        consumer,
        byte,
        printable(byte)
    );
}

#[must_use]
pub fn next_trace_id() -> u64 {
    INPUT_TRACE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
}

static LAST_INPUT_SOURCE: AtomicU8 = AtomicU8::new(0);
static LAST_INPUT_KEYCODE: AtomicU8 = AtomicU8::new(0);

#[must_use]
pub fn keycode_to_ascii(keycode: u8, shift: bool, layout: KeyboardLayout) -> Option<u8> {
    match layout {
        KeyboardLayout::UsQwerty => us_qwerty_map(keycode, shift),
        KeyboardLayout::BrazilAbnt2 => abnt2_map(keycode, shift),
        KeyboardLayout::German => de_qwertz_map(keycode, shift),
        KeyboardLayout::Spanish => es_map(keycode, shift),
        KeyboardLayout::French => fr_azerty_map(keycode, shift),
        KeyboardLayout::Japanese
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
        0x4F => Some(KEY_ARROW_RIGHT),
        0x50 => Some(KEY_ARROW_LEFT),
        0x51 => Some(KEY_ARROW_DOWN),
        0x52 => Some(KEY_ARROW_UP),
        // Keypad / extended keys frequently used on notebook embedded numpads.
        0x54 => Some(b'/'),  // keypad /
        0x55 => Some(b'*'),  // keypad *
        0x56 => Some(b'-'),  // keypad -
        0x57 => Some(b'+'),  // keypad +
        0x58 => Some(b'\n'), // keypad Enter
        0x59 => Some(b'1'),
        0x5A => Some(b'2'),
        0x5B => Some(b'3'),
        0x5C => Some(b'4'),
        0x5D => Some(b'5'),
        0x5E => Some(b'6'),
        0x5F => Some(b'7'),
        0x60 => Some(b'8'),
        0x61 => Some(b'9'),
        0x62 => Some(b'0'),
        0x63 => Some(b'.'),
        0x64 => Some(b'\\'), // non-US \ / |
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

/// German QWERTZ layout (HID keycodes to ASCII).
/// Y and Z are swapped. Non-ASCII characters (umlauts) mapped to ASCII approximations.
fn de_qwertz_map(keycode: u8, shift: bool) -> Option<u8> {
    match keycode {
        // Letters: same as US except Y(0x1C) and Z(0x1D) are swapped
        0x1C => {
            // US 'Y' key → German 'Z'
            Some(if shift { b'Z' } else { b'z' })
        }
        0x1D => {
            // US 'Z' key → German 'Y'
            Some(if shift { b'Y' } else { b'y' })
        }
        // Number row: different shift characters
        0x1E => Some(if shift { b'!' } else { b'1' }),
        0x1F => Some(if shift { b'"' } else { b'2' }),
        0x20 => Some(if shift { b'#' } else { b'3' }),
        0x21 => Some(if shift { b'$' } else { b'4' }),
        0x22 => Some(if shift { b'%' } else { b'5' }),
        0x23 => Some(if shift { b'&' } else { b'6' }),
        0x24 => Some(if shift { b'/' } else { b'7' }),
        0x25 => Some(if shift { b'(' } else { b'8' }),
        0x26 => Some(if shift { b')' } else { b'9' }),
        0x27 => Some(if shift { b'=' } else { b'0' }),
        // Punctuation remaps (ASCII approximations for non-ASCII chars)
        0x2D => Some(if shift { b'?' } else { b'-' }),   // ß → - (shift: ?)
        0x2E => Some(if shift { b'`' } else { b'=' }),   // ´ → = (approx)
        0x2F => Some(if shift { b'[' } else { b'[' }),   // ü → [ (approx)
        0x30 => Some(if shift { b'*' } else { b'+' }),   // + (shift: *)
        0x33 => Some(if shift { b'[' } else { b';' }),   // ö → ; (approx)
        0x34 => Some(if shift { b'\'' } else { b'\'' }), // ä → ' (approx)
        0x35 => Some(if shift { b'~' } else { b'^' }),   // ^ (shift: °→~)
        0x36 => Some(if shift { b';' } else { b',' }),
        0x37 => Some(if shift { b':' } else { b'.' }),
        0x38 => Some(if shift { b'_' } else { b'-' }),
        _ => us_qwerty_map(keycode, shift),
    }
}

/// Spanish keyboard layout (HID keycodes to ASCII).
/// Mostly US-like with different shift symbols on number row.
fn es_map(keycode: u8, shift: bool) -> Option<u8> {
    match keycode {
        0x1E => Some(if shift { b'!' } else { b'1' }),
        0x1F => Some(if shift { b'"' } else { b'2' }),
        0x20 => Some(if shift { b'#' } else { b'3' }),
        0x21 => Some(if shift { b'$' } else { b'4' }),
        0x22 => Some(if shift { b'%' } else { b'5' }),
        0x23 => Some(if shift { b'&' } else { b'6' }),
        0x24 => Some(if shift { b'/' } else { b'7' }),
        0x25 => Some(if shift { b'(' } else { b'8' }),
        0x26 => Some(if shift { b')' } else { b'9' }),
        0x27 => Some(if shift { b'=' } else { b'0' }),
        0x2D => Some(if shift { b'?' } else { b'\'' }),  // ' (shift: ?)
        0x2F => Some(if shift { b'^' } else { b'`' }),   // ` (shift: ^)
        0x30 => Some(if shift { b'*' } else { b'+' }),   // + (shift: *)
        0x33 => Some(if shift { b'[' } else { b';' }),   // ñ → ; (approx)
        0x36 => Some(if shift { b';' } else { b',' }),
        0x37 => Some(if shift { b':' } else { b'.' }),
        0x38 => Some(if shift { b'_' } else { b'-' }),
        _ => us_qwerty_map(keycode, shift),
    }
}

/// French AZERTY layout (HID keycodes to ASCII).
/// A↔Q and Z↔W are swapped. Number row requires shift for digits.
fn fr_azerty_map(keycode: u8, shift: bool) -> Option<u8> {
    match keycode {
        // Letter swaps: A(0x04)↔Q(0x14), Z(0x1D)↔W(0x1A)
        0x04 => Some(if shift { b'Q' } else { b'q' }), // A key → Q
        0x14 => Some(if shift { b'A' } else { b'a' }), // Q key → A
        0x1A => Some(if shift { b'Z' } else { b'z' }), // W key → Z
        0x1D => Some(if shift { b'W' } else { b'w' }), // Z key → W
        // M key position: US semicolon → M
        0x10 => Some(if shift { b'M' } else { b'm' }), // M key (AZERTY position)
        // Number row: unshifted gives symbols, shifted gives digits
        0x1E => Some(if shift { b'1' } else { b'&' }),
        0x1F => Some(if shift { b'2' } else { b'~' }),  // é → ~ (approx)
        0x20 => Some(if shift { b'3' } else { b'"' }),
        0x21 => Some(if shift { b'4' } else { b'\'' }),
        0x22 => Some(if shift { b'5' } else { b'(' }),
        0x23 => Some(if shift { b'6' } else { b'-' }),
        0x24 => Some(if shift { b'7' } else { b'`' }),  // è → ` (approx)
        0x25 => Some(if shift { b'8' } else { b'_' }),
        0x26 => Some(if shift { b'9' } else { b'^' }),  // ç → ^ (approx)
        0x27 => Some(if shift { b'0' } else { b'@' }),  // à → @ (approx)
        0x2D => Some(if shift { b'+' } else { b')' }),
        0x2E => Some(if shift { b'}' } else { b'=' }),
        0x36 => Some(if shift { b'?' } else { b',' }),
        0x33 => Some(if shift { b'.' } else { b';' }),
        0x37 => Some(if shift { b'/' } else { b':' }),
        0x38 => Some(if shift { b'>' } else { b'!' }),
        _ => us_qwerty_map(keycode, shift),
    }
}

fn source_label(source: u8) -> &'static str {
    match source {
        INPUT_SOURCE_PS2 => "ps2",
        INPUT_SOURCE_USB => "usb",
        _ => "none",
    }
}

fn printable(byte: u8) -> char {
    if byte.is_ascii_graphic() || byte == b' ' {
        byte as char
    } else {
        '.'
    }
}

struct OptionalTraceByte(Option<u8>);

impl fmt::Display for OptionalTraceByte {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(value) => write!(formatter, "0x{value:02X} '{}'", printable(value)),
            None => formatter.write_str("none"),
        }
    }
}

fn format_byte(byte: Option<u8>) -> OptionalTraceByte {
    OptionalTraceByte(byte)
}

