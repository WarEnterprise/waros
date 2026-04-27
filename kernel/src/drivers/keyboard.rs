use core::fmt;
use core::sync::atomic::{AtomicBool, AtomicU16, AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use pc_keyboard::{
    layouts, DecodedKey, HandleControl, KeyCode, KeyEvent as PcKeyEvent, KeyState,
    Keyboard as PcKeyboard, ScancodeSet1,
    ScancodeSet2,
};
use spin::{Lazy, Mutex};
use x86_64::instructions::interrupts;

use crate::arch::x86_64::pic;
use crate::arch::x86_64::port;

pub static KEYBOARD: Lazy<Mutex<Keyboard>> = Lazy::new(|| Mutex::new(Keyboard::new()));

const PS2_DATA_PORT: u16 = 0x60;
const PS2_STATUS_PORT: u16 = 0x64;
const PS2_COMMAND_PORT: u16 = 0x64;

const STATUS_OUTPUT_FULL: u8 = 1 << 0;
const STATUS_INPUT_FULL: u8 = 1 << 1;
const STATUS_AUX_DATA: u8 = 1 << 5;

const CMD_READ_CONFIG: u8 = 0x20;
const CMD_WRITE_CONFIG: u8 = 0x60;
const CMD_CONTROLLER_SELF_TEST: u8 = 0xAA;
const CMD_ENABLE_FIRST_PORT: u8 = 0xAE;
const CMD_ENABLE_SCANNING: u8 = 0xF4;

const CONFIG_FIRST_PORT_IRQ: u8 = 1 << 0;
const CONFIG_FIRST_PORT_CLOCK_DISABLED: u8 = 1 << 4;
const CONFIG_TRANSLATE_FIRST_PORT: u8 = 1 << 6;

const KEYBOARD_ACK: u8 = 0xFA;
const CONTROLLER_SELF_TEST_OK: u8 = 0x55;
const NO_INPUT_FALLBACK_MS: u64 = 750;
const RAW_IRQ_RING_CAPACITY: usize = 256;
const KBD_TRACE_IO_ENABLED: bool = false;
const KBD_TRACE_PORT_IO_ENABLED: bool = false;
const PS2_DECODE_TRACE_ENABLED: bool = true;
const MODIFIER_SHIFT: u8 = 1 << 0;
const MODIFIER_CTRL: u8 = 1 << 1;
const MODIFIER_ALT: u8 = 1 << 2;

static FIRST_IRQ_SCANCODE_LOGGED: AtomicBool = AtomicBool::new(false);
static FIRST_POLLED_SCANCODE_LOGGED: AtomicBool = AtomicBool::new(false);
static INPUT_TRACE_BUDGET: AtomicU32 = AtomicU32::new(48);
static IRQ_TRIGGER_COUNT: AtomicU32 = AtomicU32::new(0);
static IRQ_SCANCODE_COUNT: AtomicU32 = AtomicU32::new(0);
static POLLED_SCANCODE_COUNT: AtomicU32 = AtomicU32::new(0);
static TRANSLATED_BYTE_COUNT: AtomicU32 = AtomicU32::new(0);
static CONSUMED_BYTE_COUNT: AtomicU32 = AtomicU32::new(0);
static LAST_SCANCODE: AtomicU8 = AtomicU8::new(0);
static LAST_BYTE: AtomicU8 = AtomicU8::new(0);
static LAST_STATUS: AtomicU8 = AtomicU8::new(0);
static LAST_SELF_TEST: AtomicU8 = AtomicU8::new(0);
static INPUT_INIT_MILLIS: AtomicU64 = AtomicU64::new(0);
static FORCED_POLLING_MODE: AtomicBool = AtomicBool::new(false);
static NO_INPUT_DETECTED_LOGGED: AtomicBool = AtomicBool::new(false);
static I8042_PRESENT: AtomicBool = AtomicBool::new(true);
static I8042_INIT_FAILED: AtomicBool = AtomicBool::new(false);
static RAW_IRQ_RING: [AtomicU16; RAW_IRQ_RING_CAPACITY] =
    [const { AtomicU16::new(0) }; RAW_IRQ_RING_CAPACITY];
static RAW_IRQ_HEAD: AtomicUsize = AtomicUsize::new(0);
static RAW_IRQ_TAIL: AtomicUsize = AtomicUsize::new(0);
static RAW_IRQ_DROPPED: AtomicU32 = AtomicU32::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyboardLayout {
    UsQwerty,
    BrazilAbnt2,
}

#[derive(Clone, Copy)]
pub struct BufferedKeypress {
    pub byte: u8,
    pub trace_id: u64,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

/// PS/2 keyboard state and ring buffer for shell input.
pub struct Keyboard {
    inner_set1: PcKeyboard<layouts::Us104Key, ScancodeSet1>,
    inner_set2: PcKeyboard<layouts::Us104Key, ScancodeSet2>,
    active_scancode_set: ActiveScancodeSet,
    allow_dynamic_switch: bool,
    translation_enabled: bool,
    set1_only_streak: u8,
    set2_only_streak: u8,
    layout: KeyboardLayout,
    buffer: [u8; 1024],
    modifiers: [u8; 1024],
    trace_ids: [u64; 1024],
    read_pos: usize,
    write_pos: usize,
    left_shift_held: bool,
    right_shift_held: bool,
    left_ctrl_held: bool,
    right_ctrl_held: bool,
    left_alt_held: bool,
    right_alt_held: bool,
    extended_e0_prefix: bool,
    extended_f0_break: bool,
    last_prefix_e0_before: bool,
    last_prefix_f0_before: bool,
    last_selected_set: ActiveScancodeSet,
    last_set1_byte: Option<u8>,
    last_set2_byte: Option<u8>,
    last_fallback_byte: Option<u8>,
    last_emitted_from_fallback: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ActiveScancodeSet {
    Unknown,
    Set1,
    Set2,
}

impl ActiveScancodeSet {
    fn as_u8(self) -> u8 {
        match self {
            Self::Unknown => 0,
            Self::Set1 => 1,
            Self::Set2 => 2,
        }
    }
}

struct KeyboardInitReport {
    status_before: u8,
    status_after: u8,
    config_before: Option<u8>,
    config_after: Option<u8>,
    self_test: Option<u8>,
    scan_ack: Option<u8>,
}

struct OptionalHexByte(Option<u8>);

#[derive(Clone, Copy)]
pub struct KeyboardDebugSnapshot {
    pub irq_triggers: u32,
    pub irq_scancodes: u32,
    pub irq_raw_pending: usize,
    pub irq_raw_dropped: u32,
    pub polled_scancodes: u32,
    pub translated_bytes: u32,
    pub consumed_bytes: u32,
    pub last_scancode: u8,
    pub last_byte: u8,
    pub last_status: u8,
    pub last_self_test: u8,
    pub forced_polling: bool,
    pub i8042_present: bool,
    pub i8042_init_failed: bool,
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
}

impl Keyboard {
    fn new() -> Self {
        Self {
            inner_set1: PcKeyboard::new(
                ScancodeSet1::new(),
                layouts::Us104Key,
                HandleControl::Ignore,
            ),
            inner_set2: PcKeyboard::new(
                ScancodeSet2::new(),
                layouts::Us104Key,
                HandleControl::Ignore,
            ),
            active_scancode_set: ActiveScancodeSet::Unknown,
            allow_dynamic_switch: true,
            translation_enabled: false,
            set1_only_streak: 0,
            set2_only_streak: 0,
            layout: KeyboardLayout::UsQwerty,
            buffer: [0; 1024],
            modifiers: [0; 1024],
            trace_ids: [0; 1024],
            read_pos: 0,
            write_pos: 0,
            left_shift_held: false,
            right_shift_held: false,
            left_ctrl_held: false,
            right_ctrl_held: false,
            left_alt_held: false,
            right_alt_held: false,
            extended_e0_prefix: false,
            extended_f0_break: false,
            last_prefix_e0_before: false,
            last_prefix_f0_before: false,
            last_selected_set: ActiveScancodeSet::Unknown,
            last_set1_byte: None,
            last_set2_byte: None,
            last_fallback_byte: None,
            last_emitted_from_fallback: false,
        }
    }

    fn push(&mut self, byte: u8, trace_id: u64, modifiers: u8) {
        let next = (self.write_pos + 1) % self.buffer.len();
        if next != self.read_pos {
            self.buffer[self.write_pos] = byte;
            self.modifiers[self.write_pos] = modifiers;
            self.trace_ids[self.write_pos] = trace_id;
            self.write_pos = next;
        }
    }

    /// Decode a PS/2 scancode and enqueue any resulting character.
    /// This runs in deferred input processing context (non-IRQ).
    pub fn handle_scancode(&mut self, scancode: u8, trace_id: u64, status: u8, path: &str) {
        LAST_SCANCODE.store(scancode, Ordering::Relaxed);
        self.last_prefix_e0_before = self.extended_e0_prefix;
        self.last_prefix_f0_before = self.extended_f0_break;
        self.last_selected_set = ActiveScancodeSet::Unknown;
        self.last_set1_byte = None;
        self.last_set2_byte = None;
        self.last_fallback_byte = None;
        self.last_emitted_from_fallback = false;
        if KBD_TRACE_IO_ENABLED {
            crate::serial_println!(
                "[INPUT_STAGE_HW] id={} source=ps2 path={} scancode=0x{:02X} status=0x{:02X}",
                trace_id,
                path,
                scancode,
                status,
            );
        }
        let fallback_navigation = self.observe_extended_navigation(scancode);
        self.last_fallback_byte = fallback_navigation.map(|(byte, _)| byte);
        let decoded_set1 = decode_scancode_set1(&mut self.inner_set1, scancode);
        let decoded_set2 = decode_scancode_set2(&mut self.inner_set2, scancode);
        self.last_set1_byte = decoded_set1.as_ref().and_then(decoded_input_to_byte);
        self.last_set2_byte = decoded_set2.as_ref().and_then(decoded_input_to_byte);

        let selected = self.select_decoded_key(decoded_set1, decoded_set2);
        if selected.mode_changed {
            self.active_scancode_set = selected.active_set;
        }

        let mut emitted = false;
        if let Some(input) = selected.input {
            self.last_selected_set = input.origin;
            self.update_modifier_state(input.event.clone());
            if let Some(decoded) = input.decoded {
                if let Some(byte) = key_to_byte(decoded) {
                    LAST_BYTE.store(byte, Ordering::Relaxed);
                    TRANSLATED_BYTE_COUNT.fetch_add(1, Ordering::Relaxed);
                    crate::hal::input::trace_stage_decode(
                        trace_id,
                        crate::hal::input::INPUT_SOURCE_PS2,
                        scancode,
                        true,
                        Some(byte),
                    );
                    trace_decoded_input(scancode, &decoded, Some(byte), "translated");
                    self.push(byte, trace_id, self.modifier_flags());
                    crate::hal::input::trace_stage_queue_push(
                        trace_id,
                        crate::hal::input::INPUT_SOURCE_PS2,
                        byte,
                        true,
                        self.buffered_len(),
                    );
                    emitted = true;
                } else {
                    crate::hal::input::trace_stage_decode(
                        trace_id,
                        crate::hal::input::INPUT_SOURCE_PS2,
                        scancode,
                        true,
                        None,
                    );
                    trace_decoded_input(scancode, &decoded, None, "discarded");
                }
            }
        }

        if !emitted {
            if let Some((byte, released)) = fallback_navigation {
                crate::hal::input::trace_stage_decode(
                    trace_id,
                    crate::hal::input::INPUT_SOURCE_PS2,
                    scancode,
                    !released,
                    if released { None } else { Some(byte) },
                );
                if !released {
                    self.last_emitted_from_fallback = true;
                    LAST_BYTE.store(byte, Ordering::Relaxed);
                    TRANSLATED_BYTE_COUNT.fetch_add(1, Ordering::Relaxed);
                    self.push(byte, trace_id, self.modifier_flags());
                    crate::hal::input::trace_stage_queue_push(
                        trace_id,
                        crate::hal::input::INPUT_SOURCE_PS2,
                        byte,
                        true,
                        self.buffered_len(),
                    );
                }
            }
        }
        trace_ps2_decode_step(scancode, status, self);
    }

    /// Read one character from the keyboard input buffer.
    pub fn read_keypress(&mut self) -> Option<BufferedKeypress> {
        if self.read_pos == self.write_pos {
            return None;
        }

        let byte = self.buffer[self.read_pos];
        let modifiers = self.modifiers[self.read_pos];
        let trace_id = self.trace_ids[self.read_pos];
        self.read_pos = (self.read_pos + 1) % self.buffer.len();
        crate::hal::input::trace_stage_queue_pop(
            trace_id,
            crate::hal::input::INPUT_SOURCE_PS2,
            byte,
            byte,
            self.buffered_len(),
        );
        Some(BufferedKeypress {
            byte,
            trace_id,
            shift: modifiers & MODIFIER_SHIFT != 0,
            ctrl: modifiers & MODIFIER_CTRL != 0,
            alt: modifiers & MODIFIER_ALT != 0,
        })
    }

    #[must_use]
    pub fn buffered_len(&self) -> usize {
        if self.write_pos >= self.read_pos {
            self.write_pos - self.read_pos
        } else {
            self.buffer.len() - self.read_pos + self.write_pos
        }
    }

    pub fn set_layout(&mut self, layout: KeyboardLayout) {
        self.layout = layout;
        // Keep decoder state stable across layout updates.
        // Resetting decoders during pre-auth preferences can desynchronize
        // real hardware input right before username/password entry.
    }

    pub fn configure_scancode_mode(&mut self, translation_enabled: bool) {
        self.translation_enabled = translation_enabled;
        self.active_scancode_set = if translation_enabled {
            ActiveScancodeSet::Set1
        } else {
            ActiveScancodeSet::Unknown
        };
        // When i8042 translation is active, lock decoding to set1.
        // Dynamic switching can misclassify translated Enter (0x1C) as set2 'a' (0x61).
        self.allow_dynamic_switch = !translation_enabled;
        self.set1_only_streak = 0;
        self.set2_only_streak = 0;
    }

    fn reset_decoder_state_for_recovery(&mut self) {
        self.inner_set1 = PcKeyboard::new(
            ScancodeSet1::new(),
            layouts::Us104Key,
            HandleControl::Ignore,
        );
        self.inner_set2 = PcKeyboard::new(
            ScancodeSet2::new(),
            layouts::Us104Key,
            HandleControl::Ignore,
        );
        self.active_scancode_set = if self.translation_enabled {
            ActiveScancodeSet::Set1
        } else {
            ActiveScancodeSet::Unknown
        };
        self.allow_dynamic_switch = !self.translation_enabled;
        self.set1_only_streak = 0;
        self.set2_only_streak = 0;
        self.left_shift_held = false;
        self.right_shift_held = false;
        self.left_ctrl_held = false;
        self.right_ctrl_held = false;
        self.left_alt_held = false;
        self.right_alt_held = false;
        self.extended_e0_prefix = false;
        self.extended_f0_break = false;
        self.last_prefix_e0_before = false;
        self.last_prefix_f0_before = false;
        self.last_selected_set = ActiveScancodeSet::Unknown;
        self.last_set1_byte = None;
        self.last_set2_byte = None;
        self.last_fallback_byte = None;
        self.last_emitted_from_fallback = false;
    }

    #[must_use]
    pub fn layout(&self) -> KeyboardLayout {
        self.layout
    }

    fn modifier_flags(&self) -> u8 {
        let mut flags = 0u8;
        if self.left_shift_held || self.right_shift_held {
            flags |= MODIFIER_SHIFT;
        }
        if self.left_ctrl_held || self.right_ctrl_held {
            flags |= MODIFIER_CTRL;
        }
        if self.left_alt_held || self.right_alt_held {
            flags |= MODIFIER_ALT;
        }
        flags
    }

    fn update_modifier_state(&mut self, event: PcKeyEvent) {
        let pressed = matches!(event.state, KeyState::Down | KeyState::SingleShot);
        match event.code {
            KeyCode::LShift => self.left_shift_held = pressed,
            KeyCode::RShift => self.right_shift_held = pressed,
            KeyCode::LControl => self.left_ctrl_held = pressed,
            KeyCode::RControl | KeyCode::RControl2 => self.right_ctrl_held = pressed,
            KeyCode::LAlt => self.left_alt_held = pressed,
            KeyCode::RAltGr | KeyCode::RAlt2 => self.right_alt_held = pressed,
            _ => {}
        }
    }

    fn observe_extended_navigation(&mut self, scancode: u8) -> Option<(u8, bool)> {
        if scancode == 0xE0 {
            self.extended_e0_prefix = true;
            self.extended_f0_break = false;
            return None;
        }

        if !self.extended_e0_prefix {
            return None;
        }

        if scancode == 0xF0 {
            self.extended_f0_break = true;
            return None;
        }

        let released = self.extended_f0_break || (scancode & 0x80) != 0;
        let normalized = if (scancode & 0x80) != 0 {
            scancode & 0x7F
        } else {
            scancode
        };
        self.extended_e0_prefix = false;
        self.extended_f0_break = false;

        let byte = match normalized {
            0x48 | 0x75 => crate::hal::input::KEY_ARROW_UP,
            0x50 | 0x72 => crate::hal::input::KEY_ARROW_DOWN,
            0x4B | 0x6B => crate::hal::input::KEY_ARROW_LEFT,
            0x4D | 0x74 => crate::hal::input::KEY_ARROW_RIGHT,
            0x1C | 0x5A => b'\n',
            _ => return None,
        };
        Some((byte, released))
    }

    fn select_decoded_key(
        &mut self,
        set1: Option<DecodedInput>,
        set2: Option<DecodedInput>,
    ) -> SelectedDecoded {
        if !self.allow_dynamic_switch {
            self.set1_only_streak = 0;
            self.set2_only_streak = 0;
            return match self.active_scancode_set {
                ActiveScancodeSet::Set1 => SelectedDecoded {
                    input: set1,
                    active_set: ActiveScancodeSet::Set1,
                    mode_changed: false,
                },
                ActiveScancodeSet::Set2 => SelectedDecoded {
                    input: set2,
                    active_set: ActiveScancodeSet::Set2,
                    mode_changed: false,
                },
                ActiveScancodeSet::Unknown => SelectedDecoded {
                    input: set1.or(set2),
                    active_set: ActiveScancodeSet::Unknown,
                    mode_changed: false,
                },
            };
        }

        const SWITCH_STREAK: u8 = 3;

        match self.active_scancode_set {
            ActiveScancodeSet::Set1 => match (set1, set2) {
                (Some(input), _) => {
                    self.set1_only_streak = 0;
                    self.set2_only_streak = 0;
                    SelectedDecoded {
                        input: Some(input),
                        active_set: ActiveScancodeSet::Set1,
                        mode_changed: false,
                    }
                }
                (None, Some(input)) => {
                    if input.probe_key {
                        let cold_start = TRANSLATED_BYTE_COUNT.load(Ordering::Relaxed) == 0;
                        self.set2_only_streak = self.set2_only_streak.saturating_add(1);
                        self.set1_only_streak = 0;
                        if cold_start || self.set2_only_streak >= SWITCH_STREAK {
                            self.set2_only_streak = 0;
                            SelectedDecoded {
                                input: Some(input),
                                active_set: ActiveScancodeSet::Set2,
                                mode_changed: true,
                            }
                        } else {
                            SelectedDecoded {
                                input: None,
                                active_set: ActiveScancodeSet::Set1,
                                mode_changed: false,
                            }
                        }
                    } else {
                        self.set1_only_streak = 0;
                        self.set2_only_streak = 0;
                        SelectedDecoded {
                            input: None,
                            active_set: ActiveScancodeSet::Set1,
                            mode_changed: false,
                        }
                    }
                }
                (None, None) => {
                    self.set1_only_streak = 0;
                    self.set2_only_streak = 0;
                    SelectedDecoded {
                        input: None,
                        active_set: ActiveScancodeSet::Set1,
                        mode_changed: false,
                    }
                }
            },
            ActiveScancodeSet::Set2 => match (set1, set2) {
                (_, Some(input)) => {
                    self.set1_only_streak = 0;
                    self.set2_only_streak = 0;
                    SelectedDecoded {
                        input: Some(input),
                        active_set: ActiveScancodeSet::Set2,
                        mode_changed: false,
                    }
                }
                (Some(input), None) => {
                    if input.probe_key {
                        let cold_start = TRANSLATED_BYTE_COUNT.load(Ordering::Relaxed) == 0;
                        self.set1_only_streak = self.set1_only_streak.saturating_add(1);
                        self.set2_only_streak = 0;
                        if cold_start || self.set1_only_streak >= SWITCH_STREAK {
                            self.set1_only_streak = 0;
                            SelectedDecoded {
                                input: Some(input),
                                active_set: ActiveScancodeSet::Set1,
                                mode_changed: true,
                            }
                        } else {
                            SelectedDecoded {
                                input: None,
                                active_set: ActiveScancodeSet::Set2,
                                mode_changed: false,
                            }
                        }
                    } else {
                        self.set1_only_streak = 0;
                        self.set2_only_streak = 0;
                        SelectedDecoded {
                            input: None,
                            active_set: ActiveScancodeSet::Set2,
                            mode_changed: false,
                        }
                    }
                }
                (None, None) => {
                    self.set1_only_streak = 0;
                    self.set2_only_streak = 0;
                    SelectedDecoded {
                        input: None,
                        active_set: ActiveScancodeSet::Set2,
                        mode_changed: false,
                    }
                }
            },
            ActiveScancodeSet::Unknown => match (set1, set2) {
                (Some(input), None) => {
                    self.set1_only_streak = 0;
                    self.set2_only_streak = 0;
                    let probe_key = input.probe_key;
                    SelectedDecoded {
                        input: Some(input),
                        active_set: if probe_key {
                            ActiveScancodeSet::Set1
                        } else {
                            ActiveScancodeSet::Unknown
                        },
                        mode_changed: probe_key,
                    }
                }
                (None, Some(input)) => {
                    self.set1_only_streak = 0;
                    self.set2_only_streak = 0;
                    let probe_key = input.probe_key;
                    SelectedDecoded {
                        input: Some(input),
                        active_set: if probe_key {
                            ActiveScancodeSet::Set2
                        } else {
                            ActiveScancodeSet::Unknown
                        },
                        mode_changed: probe_key,
                    }
                }
                (Some(input1), Some(input2)) => {
                    self.set1_only_streak = 0;
                    self.set2_only_streak = 0;
                    if input1.probe_key && !input2.probe_key {
                        SelectedDecoded {
                            input: Some(input1),
                            active_set: ActiveScancodeSet::Set1,
                            mode_changed: true,
                        }
                    } else if input2.probe_key && !input1.probe_key {
                        SelectedDecoded {
                            input: Some(input2),
                            active_set: ActiveScancodeSet::Set2,
                            mode_changed: true,
                        }
                    } else {
                        SelectedDecoded {
                            input: Some(input1),
                            active_set: ActiveScancodeSet::Unknown,
                            mode_changed: false,
                        }
                    }
                }
                (None, None) => {
                    self.set1_only_streak = 0;
                    self.set2_only_streak = 0;
                    SelectedDecoded {
                        input: None,
                        active_set: ActiveScancodeSet::Unknown,
                        mode_changed: false,
                    }
                }
            },
        }
    }
}

/// Initialize the keyboard driver state.
pub fn init() {
    interrupts::without_interrupts(|| {
        *KEYBOARD.lock() = Keyboard::new();
    });
    reset_debug_counters();
    FORCED_POLLING_MODE.store(false, Ordering::Relaxed);
    NO_INPUT_DETECTED_LOGGED.store(false, Ordering::Relaxed);
    I8042_INIT_FAILED.store(false, Ordering::Relaxed);
    INPUT_INIT_MILLIS.store(uptime_millis(), Ordering::Relaxed);

    // Detect absent i8042 using repeated samples to reduce false negatives
    // on real hardware where one transient 0xFF read can occur during early boot.
    if likely_i8042_absent() {
        crate::serial_println!(
            "[kbd] i8042 controller not detected (repeated status/data=0xFF) — skipping PS/2 init"
        );
        I8042_PRESENT.store(false, Ordering::Relaxed);
        I8042_INIT_FAILED.store(true, Ordering::Relaxed);
        FORCED_POLLING_MODE.store(true, Ordering::Relaxed);
        return;
    }
    I8042_PRESENT.store(true, Ordering::Relaxed);

    let report = init_controller();
    let scan_enabled = matches!(report.scan_ack, Some(KEYBOARD_ACK));
    let self_test_passed = matches!(report.self_test, Some(CONTROLLER_SELF_TEST_OK));
    let translation_enabled = report
        .config_after
        .or(report.config_before)
        .map(|config| config & CONFIG_TRANSLATE_FIRST_PORT != 0)
        .unwrap_or(false);
    interrupts::without_interrupts(|| {
        KEYBOARD
            .lock()
            .configure_scancode_mode(translation_enabled);
    });

    if !self_test_passed && report.self_test.is_some() {
        crate::serial_println!(
            "[kbd] WARNING: i8042 self-test returned 0x{:02X} (expected 0x55) — controller may be absent or broken",
            report.self_test.unwrap_or(0)
        );
        I8042_INIT_FAILED.store(true, Ordering::Relaxed);
    }

    if !scan_enabled {
        crate::serial_println!(
            "[kbd] WARNING: keyboard scanning not enabled — input may require USB HID fallback"
        );
        I8042_INIT_FAILED.store(true, Ordering::Relaxed);
    }

    crate::serial_println!(
        "[kbd] i8042 init: status_before=0x{:02X} status_after=0x{:02X} self_test={} config_before={} config_after={} scan_ack={} scan_enabled={} translate={} present={}",
        report.status_before,
        report.status_after,
        format_byte(report.self_test),
        format_byte(report.config_before),
        format_byte(report.config_after),
        format_byte(report.scan_ack),
        if scan_enabled { "yes" } else { "no" },
        if translation_enabled { "on" } else { "off" },
        if self_test_passed { "yes" } else { "maybe" },
    );
    log_pic_masks("after-keyboard-init");
}

pub fn ensure_controller_ready(context: &str) {
    if !I8042_PRESENT.load(Ordering::Relaxed) {
        crate::serial_println!("[kbd] controller rearm ({}) skipped: i8042 not present", context);
        return;
    }
    let report = init_controller();
    let translation_enabled = report
        .config_after
        .or(report.config_before)
        .map(|config| config & CONFIG_TRANSLATE_FIRST_PORT != 0)
        .unwrap_or(false);
    interrupts::without_interrupts(|| {
        KEYBOARD
            .lock()
            .configure_scancode_mode(translation_enabled);
    });
    crate::serial_println!(
        "[kbd] controller rearm ({}) status_before=0x{:02X} status_after=0x{:02X} self_test={} config_before={} config_after={} scan_ack={} scan_enabled={} translate={}",
        context,
        report.status_before,
        report.status_after,
        format_byte(report.self_test),
        format_byte(report.config_before),
        format_byte(report.config_after),
        format_byte(report.scan_ack),
        if matches!(report.scan_ack, Some(KEYBOARD_ACK)) {
            "yes"
        } else {
            "no"
        },
        if translation_enabled { "on" } else { "off" },
    );
    log_pic_masks(context);
}

pub fn recover_decode_state(context: &str) {
    interrupts::without_interrupts(|| {
        KEYBOARD.lock().reset_decoder_state_for_recovery();
    });
    if KBD_TRACE_IO_ENABLED {
        crate::serial_println!("[kbd] decoder state recovered ({})", context);
    }
}

/// Called from IRQ1. Keep this path minimal: read hardware data and enqueue raw.
pub fn handle_irq1(status: u8) {
    IRQ_TRIGGER_COUNT.fetch_add(1, Ordering::Relaxed);
    LAST_STATUS.store(status, Ordering::Relaxed);

    if status & STATUS_OUTPUT_FULL == 0 {
        return;
    }
    if status & STATUS_AUX_DATA != 0 {
        // Drain AUX byte to avoid blocking the controller output buffer.
        // Some laptops can leave AUX data pending while keyboard input is waiting.
        let _ = port::inb(PS2_DATA_PORT);
        return;
    }

    let scancode = port::inb(PS2_DATA_PORT);
    LAST_SCANCODE.store(scancode, Ordering::Relaxed);
    IRQ_SCANCODE_COUNT.fetch_add(1, Ordering::Relaxed);
    push_raw_irq_scancode(scancode, status);
}

#[inline]
fn push_raw_irq_scancode(scancode: u8, status: u8) {
    let head = RAW_IRQ_HEAD.load(Ordering::Relaxed);
    let next = (head + 1) % RAW_IRQ_RING_CAPACITY;
    let tail = RAW_IRQ_TAIL.load(Ordering::Acquire);
    if next == tail {
        RAW_IRQ_DROPPED.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let packed = (u16::from(status) << 8) | u16::from(scancode);
    RAW_IRQ_RING[head].store(packed, Ordering::Relaxed);
    RAW_IRQ_HEAD.store(next, Ordering::Release);
}

#[inline]
fn pop_raw_irq_scancode() -> Option<(u8, u8)> {
    let tail = RAW_IRQ_TAIL.load(Ordering::Relaxed);
    let head = RAW_IRQ_HEAD.load(Ordering::Acquire);
    if tail == head {
        return None;
    }
    let packed = RAW_IRQ_RING[tail].load(Ordering::Relaxed);
    RAW_IRQ_TAIL.store((tail + 1) % RAW_IRQ_RING_CAPACITY, Ordering::Release);
    let scancode = (packed & 0x00FF) as u8;
    let status = (packed >> 8) as u8;
    Some((scancode, status))
}

#[must_use]
fn raw_irq_pending_len() -> usize {
    let head = RAW_IRQ_HEAD.load(Ordering::Acquire);
    let tail = RAW_IRQ_TAIL.load(Ordering::Relaxed);
    if head >= tail {
        head - tail
    } else {
        RAW_IRQ_RING_CAPACITY - tail + head
    }
}

/// Process raw IRQ1 scancodes in non-IRQ context.
#[must_use]
pub fn process_pending_irq_events() -> bool {
    let mut processed = false;
    while let Some((scancode, status)) = pop_raw_irq_scancode() {
        let trace_id = crate::hal::input::next_trace_id();
        let should_trace = !FIRST_IRQ_SCANCODE_LOGGED.swap(true, Ordering::Relaxed);
        KEYBOARD
            .lock()
            .handle_scancode(scancode, trace_id, status, "irq-deferred");
        if should_trace && KBD_TRACE_IO_ENABLED {
            crate::serial_println!("[kbd] first scancode via irq-deferred: 0x{:02X}", scancode);
        }
        processed = true;
    }
    processed
}

/// Drain any keyboard bytes that reached the i8042 output buffer without an IRQ.
#[must_use]
pub fn poll_hardware() -> bool {
    let mut drained = process_pending_irq_events();
    if !I8042_PRESENT.load(Ordering::Relaxed) {
        return drained;
    }
    let mut first_scancode = None;
    let mut read_budget_exhausted = false;
    let mut aux_drained = 0usize;
    interrupts::without_interrupts(|| {
        let mut keyboard = KEYBOARD.lock();
        let mut reads = 0usize;
        loop {
            if reads >= 256 {
                read_budget_exhausted = true;
                break;
            }
            let status = port::inb(PS2_STATUS_PORT);
            LAST_STATUS.store(status, Ordering::Relaxed);
            if status & STATUS_OUTPUT_FULL == 0 {
                break;
            }
            if status & STATUS_AUX_DATA != 0 {
                // Drain AUX data instead of bailing out; otherwise output buffer can
                // remain permanently blocked and keyboard bytes never get serviced.
                let _ = read_data_port("polling-aux", status);
                reads = reads.saturating_add(1);
                aux_drained = aux_drained.saturating_add(1);
                continue;
            }

            let scancode = read_data_port("polling", status);
            reads = reads.saturating_add(1);
            let trace_id = crate::hal::input::next_trace_id();
            if first_scancode.is_none() {
                first_scancode = Some(scancode);
            }
            let poll_n = POLLED_SCANCODE_COUNT.fetch_add(1, Ordering::Relaxed);
            if KBD_TRACE_IO_ENABLED && (poll_n < 8 || poll_n & 0xFF == 0) {
                crate::serial_println!(
                    "[kbd] polling read scancode=0x{:02X} status=0x{:02X} n={}",
                    scancode,
                    status,
                    poll_n
                );
            }
            keyboard.handle_scancode(scancode, trace_id, status, "poll");
            drained = true;
        }
    });

    if let Some(scancode) = first_scancode {
        if !FIRST_POLLED_SCANCODE_LOGGED.swap(true, Ordering::Relaxed) && KBD_TRACE_IO_ENABLED {
            crate::serial_println!("[kbd] first scancode via polling: 0x{:02X}", scancode);
        }
    }
    if read_budget_exhausted && KBD_TRACE_IO_ENABLED {
        crate::serial_println!(
            "[kbd] polling window capped at 256 reads (status=0x{:02X} aux_drained={})",
            LAST_STATUS.load(Ordering::Relaxed),
            aux_drained
        );
    } else if aux_drained != 0 && KBD_TRACE_IO_ENABLED {
        crate::serial_println!("[kbd] drained {} AUX bytes while polling keyboard", aux_drained);
    }

    drained
}

/// Read one buffered character for the shell.
pub fn read_char() -> Option<u8> {
    let _ = process_pending_irq_events();
    let byte = interrupts::without_interrupts(|| KEYBOARD.lock().read_keypress().map(|keypress| keypress.byte));
    if let Some(byte) = byte {
        CONSUMED_BYTE_COUNT.fetch_add(1, Ordering::Relaxed);
        LAST_BYTE.store(byte, Ordering::Relaxed);
    }
    byte
}

pub fn read_keypress() -> Option<BufferedKeypress> {
    let _ = process_pending_irq_events();
    let keypress = interrupts::without_interrupts(|| KEYBOARD.lock().read_keypress());
    if let Some(keypress) = keypress {
        CONSUMED_BYTE_COUNT.fetch_add(1, Ordering::Relaxed);
        LAST_BYTE.store(keypress.byte, Ordering::Relaxed);
        return Some(keypress);
    }
    None
}

#[must_use]
pub fn has_buffered_input() -> bool {
    let _ = process_pending_irq_events();
    interrupts::without_interrupts(|| KEYBOARD.lock().buffered_len() != 0)
}

pub fn set_layout(layout: KeyboardLayout) {
    interrupts::without_interrupts(|| {
        KEYBOARD.lock().set_layout(layout);
    });
}

#[must_use]
pub fn current_layout() -> KeyboardLayout {
    interrupts::without_interrupts(|| KEYBOARD.lock().layout())
}

#[must_use]
pub fn debug_snapshot() -> KeyboardDebugSnapshot {
    let trace = interrupts::without_interrupts(|| {
        let keyboard = KEYBOARD.lock();
        (
            keyboard.translation_enabled,
            keyboard.allow_dynamic_switch,
            keyboard.active_scancode_set.as_u8(),
            keyboard.last_selected_set.as_u8(),
            keyboard.last_prefix_e0_before,
            keyboard.last_prefix_f0_before,
            keyboard.last_set1_byte,
            keyboard.last_set2_byte,
            keyboard.last_fallback_byte,
            keyboard.last_emitted_from_fallback,
        )
    });
    KeyboardDebugSnapshot {
        irq_triggers: IRQ_TRIGGER_COUNT.load(Ordering::Relaxed),
        irq_scancodes: IRQ_SCANCODE_COUNT.load(Ordering::Relaxed),
        irq_raw_pending: raw_irq_pending_len(),
        irq_raw_dropped: RAW_IRQ_DROPPED.load(Ordering::Relaxed),
        polled_scancodes: POLLED_SCANCODE_COUNT.load(Ordering::Relaxed),
        translated_bytes: TRANSLATED_BYTE_COUNT.load(Ordering::Relaxed),
        consumed_bytes: CONSUMED_BYTE_COUNT.load(Ordering::Relaxed),
        last_scancode: LAST_SCANCODE.load(Ordering::Relaxed),
        last_byte: LAST_BYTE.load(Ordering::Relaxed),
        last_status: LAST_STATUS.load(Ordering::Relaxed),
        last_self_test: LAST_SELF_TEST.load(Ordering::Relaxed),
        forced_polling: FORCED_POLLING_MODE.load(Ordering::Relaxed),
        i8042_present: I8042_PRESENT.load(Ordering::Relaxed),
        i8042_init_failed: I8042_INIT_FAILED.load(Ordering::Relaxed),
        ps2_translation_enabled: trace.0,
        ps2_dynamic_switch: trace.1,
        ps2_active_set: trace.2,
        ps2_selected_set: trace.3,
        ps2_prefix_e0_before: trace.4,
        ps2_prefix_f0_before: trace.5,
        ps2_set1_byte: trace.6,
        ps2_set2_byte: trace.7,
        ps2_fallback_byte: trace.8,
        ps2_emitted_from_fallback: trace.9,
    }
}

#[must_use]
pub fn maybe_force_polling_mode(context: &str) -> bool {
    if FORCED_POLLING_MODE.load(Ordering::Relaxed) {
        return true;
    }

    let any_input_seen = IRQ_SCANCODE_COUNT.load(Ordering::Relaxed) != 0
        || POLLED_SCANCODE_COUNT.load(Ordering::Relaxed) != 0;
    if any_input_seen {
        return false;
    }

    let elapsed = uptime_millis().saturating_sub(INPUT_INIT_MILLIS.load(Ordering::Relaxed));
    if elapsed < NO_INPUT_FALLBACK_MS {
        return false;
    }

    FORCED_POLLING_MODE.store(true, Ordering::Relaxed);
    if !NO_INPUT_DETECTED_LOGGED.swap(true, Ordering::Relaxed) {
        crate::serial_println!(
            "[kbd] no input detected after {} ms; switching to forced polling mode status=0x{:02X} context={}",
            elapsed,
            port::inb(PS2_STATUS_PORT),
            context
        );
    }
    ensure_controller_ready("forced-polling-fallback");
    true
}

#[must_use]
pub fn polling_mode_forced() -> bool {
    FORCED_POLLING_MODE.load(Ordering::Relaxed)
}

#[must_use]
pub fn i8042_present() -> bool {
    I8042_PRESENT.load(Ordering::Relaxed)
}

#[must_use]
pub fn poll_for_input_window(window_ms: u64) -> bool {
    let start_ms = uptime_millis();
    loop {
        if poll_hardware() {
            return true;
        }

        if uptime_millis().saturating_sub(start_ms) >= window_ms {
            return false;
        }
        core::hint::spin_loop();
    }
}

fn key_to_byte(key: DecodedKey) -> Option<u8> {
    key_to_byte_ref(&key)
}

fn key_to_byte_ref(key: &DecodedKey) -> Option<u8> {
    match key {
        DecodedKey::Unicode(character) if character.is_ascii() => Some(*character as u8),
        DecodedKey::RawKey(KeyCode::Return) | DecodedKey::RawKey(KeyCode::NumpadEnter) => {
            Some(b'\n')
        }
        DecodedKey::RawKey(KeyCode::Backspace) => Some(0x08),
        DecodedKey::RawKey(KeyCode::Tab) => Some(b'\t'),
        DecodedKey::RawKey(KeyCode::Escape) => Some(0x1b),
        DecodedKey::RawKey(KeyCode::ArrowUp) => Some(crate::hal::input::KEY_ARROW_UP),
        DecodedKey::RawKey(KeyCode::ArrowDown) => Some(crate::hal::input::KEY_ARROW_DOWN),
        DecodedKey::RawKey(KeyCode::ArrowLeft) => Some(crate::hal::input::KEY_ARROW_LEFT),
        DecodedKey::RawKey(KeyCode::ArrowRight) => Some(crate::hal::input::KEY_ARROW_RIGHT),
        _ => None,
    }
}

fn decoded_input_to_byte(input: &DecodedInput) -> Option<u8> {
    input.decoded.as_ref().and_then(key_to_byte_ref)
}

fn scancode_probe_key(key: &DecodedKey) -> bool {
    match key {
        DecodedKey::Unicode(_) => true,
        DecodedKey::RawKey(
            KeyCode::Return
            | KeyCode::NumpadEnter
            | KeyCode::Backspace
            | KeyCode::Tab
            | KeyCode::Escape
            | KeyCode::ArrowUp
            | KeyCode::ArrowDown
            | KeyCode::ArrowLeft
            | KeyCode::ArrowRight,
        ) => true,
        DecodedKey::RawKey(
            KeyCode::LShift
            | KeyCode::RShift
            | KeyCode::LControl
            | KeyCode::RControl
            | KeyCode::LAlt
            | KeyCode::RAltGr
            | KeyCode::LWin
            | KeyCode::RWin
            | KeyCode::CapsLock
            | KeyCode::NumpadLock
            | KeyCode::ScrollLock
            | KeyCode::RControl2
            | KeyCode::RAlt2,
        ) => false,
        DecodedKey::RawKey(_) => true,
    }
}

fn decode_scancode_set1(
    decoder: &mut PcKeyboard<layouts::Us104Key, ScancodeSet1>,
    scancode: u8,
) -> Option<DecodedInput> {
    let event = decoder.add_byte(scancode).ok().flatten()?;
    let decoded = decoder.process_keyevent(event.clone());
    let probe_key = decoded.as_ref().is_some_and(scancode_probe_key);
    Some(DecodedInput {
        event,
        decoded,
        probe_key,
        origin: ActiveScancodeSet::Set1,
    })
}

fn decode_scancode_set2(
    decoder: &mut PcKeyboard<layouts::Us104Key, ScancodeSet2>,
    scancode: u8,
) -> Option<DecodedInput> {
    let event = decoder.add_byte(scancode).ok().flatten()?;
    let decoded = decoder.process_keyevent(event.clone());
    let probe_key = decoded.as_ref().is_some_and(scancode_probe_key);
    Some(DecodedInput {
        event,
        decoded,
        probe_key,
        origin: ActiveScancodeSet::Set2,
    })
}

#[derive(Clone)]
struct DecodedInput {
    event: PcKeyEvent,
    decoded: Option<DecodedKey>,
    probe_key: bool,
    origin: ActiveScancodeSet,
}

struct SelectedDecoded {
    input: Option<DecodedInput>,
    active_set: ActiveScancodeSet,
    mode_changed: bool,
}

fn reset_debug_counters() {
    INPUT_TRACE_BUDGET.store(48, Ordering::Relaxed);
    IRQ_TRIGGER_COUNT.store(0, Ordering::Relaxed);
    IRQ_SCANCODE_COUNT.store(0, Ordering::Relaxed);
    POLLED_SCANCODE_COUNT.store(0, Ordering::Relaxed);
    TRANSLATED_BYTE_COUNT.store(0, Ordering::Relaxed);
    CONSUMED_BYTE_COUNT.store(0, Ordering::Relaxed);
    LAST_SCANCODE.store(0, Ordering::Relaxed);
    LAST_BYTE.store(0, Ordering::Relaxed);
    LAST_STATUS.store(0, Ordering::Relaxed);
    LAST_SELF_TEST.store(0, Ordering::Relaxed);
    FIRST_IRQ_SCANCODE_LOGGED.store(false, Ordering::Relaxed);
    FIRST_POLLED_SCANCODE_LOGGED.store(false, Ordering::Relaxed);
    RAW_IRQ_HEAD.store(0, Ordering::Relaxed);
    RAW_IRQ_TAIL.store(0, Ordering::Relaxed);
    RAW_IRQ_DROPPED.store(0, Ordering::Relaxed);
}

fn trace_decoded_input(scancode: u8, decoded: &DecodedKey, byte: Option<u8>, stage: &str) {
    if !KBD_TRACE_IO_ENABLED {
        let _ = (scancode, decoded, byte, stage);
        return;
    }
    let remaining = INPUT_TRACE_BUDGET.load(Ordering::Relaxed);
    if remaining == 0 {
        return;
    }
    if INPUT_TRACE_BUDGET
        .compare_exchange(remaining, remaining - 1, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
    {
        match byte {
            Some(byte) => crate::serial_println!(
                "[kbd] {} scancode=0x{:02X} decoded={:?} byte=0x{:02X} '{}'",
                stage,
                scancode,
                decoded,
                byte,
                if byte.is_ascii_graphic() || byte == b' ' {
                    byte as char
                } else {
                    '.'
                }
            ),
            None => crate::serial_println!(
                "[kbd] {} scancode=0x{:02X} decoded={:?}",
                stage,
                scancode,
                decoded
            ),
        }
    }
}

fn trace_ps2_decode_step(scancode: u8, status: u8, keyboard: &Keyboard) {
    if !PS2_DECODE_TRACE_ENABLED {
        let _ = (scancode, status, keyboard);
        return;
    }
    let remaining = INPUT_TRACE_BUDGET.load(Ordering::Relaxed);
    if remaining == 0 {
        return;
    }
    if INPUT_TRACE_BUDGET
        .compare_exchange(remaining, remaining - 1, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
    {
        crate::serial_println!(
            "[PS2 TRACE] raw=0x{:02X} status=0x{:02X} pre_e0={} pre_f0={} active=set{} selected=set{} s1={} s2={} fb={} out=0x{:02X} from_fb={} dyn={} xlat={}",
            scancode,
            status,
            if keyboard.last_prefix_e0_before { "1" } else { "0" },
            if keyboard.last_prefix_f0_before { "1" } else { "0" },
            keyboard.active_scancode_set.as_u8(),
            keyboard.last_selected_set.as_u8(),
            format_byte(keyboard.last_set1_byte),
            format_byte(keyboard.last_set2_byte),
            format_byte(keyboard.last_fallback_byte),
            LAST_BYTE.load(Ordering::Relaxed),
            if keyboard.last_emitted_from_fallback {
                "1"
            } else {
                "0"
            },
            if keyboard.allow_dynamic_switch { "1" } else { "0" },
            if keyboard.translation_enabled { "1" } else { "0" }
        );
    }
}

fn init_controller() -> KeyboardInitReport {
    let mut report = KeyboardInitReport {
        status_before: sample_status("before-init"),
        status_after: 0,
        config_before: None,
        config_after: None,
        self_test: None,
        scan_ack: None,
    };

    drain_output_buffer(32);
    report.self_test = run_controller_self_test();

    if !wait_input_ready() {
        report.status_after = sample_status("init-abort");
        return report;
    }
    port::outb(PS2_COMMAND_PORT, CMD_ENABLE_FIRST_PORT);

    let Some(config_before) = read_controller_config() else {
        return report;
    };
    report.config_before = Some(config_before);

    let config_after = (config_before | CONFIG_FIRST_PORT_IRQ | CONFIG_TRANSLATE_FIRST_PORT)
        & !CONFIG_FIRST_PORT_CLOCK_DISABLED;
    if write_controller_config(config_after) {
        report.config_after = Some(config_after);
    }

    report.scan_ack = send_keyboard_command(CMD_ENABLE_SCANNING);
    drain_output_buffer(32);
    report.status_after = sample_status("after-init");
    report
}

fn run_controller_self_test() -> Option<u8> {
    if !wait_input_ready() {
        crate::serial_println!("[kbd] controller self-test skipped: input buffer busy");
        return None;
    }

    port::outb(PS2_COMMAND_PORT, CMD_CONTROLLER_SELF_TEST);
    let response = read_keyboard_data("self-test");
    if let Some(value) = response {
        LAST_SELF_TEST.store(value, Ordering::Relaxed);
    }
    crate::serial_println!(
        "[kbd] controller self-test response={} ok={}",
        format_byte(response),
        if matches!(response, Some(CONTROLLER_SELF_TEST_OK)) {
            "yes"
        } else {
            "no"
        }
    );
    response
}

fn read_controller_config() -> Option<u8> {
    if !wait_input_ready() {
        return None;
    }
    port::outb(PS2_COMMAND_PORT, CMD_READ_CONFIG);
    read_keyboard_data("read-config")
}

fn write_controller_config(config: u8) -> bool {
    if !wait_input_ready() {
        return false;
    }
    port::outb(PS2_COMMAND_PORT, CMD_WRITE_CONFIG);
    if !wait_input_ready() {
        return false;
    }
    port::outb(PS2_DATA_PORT, config);
    true
}

fn send_keyboard_command(command: u8) -> Option<u8> {
    if !wait_input_ready() {
        return None;
    }
    port::outb(PS2_DATA_PORT, command);
    read_keyboard_data("keyboard-cmd")
}

fn read_keyboard_data(context: &str) -> Option<u8> {
    // Shorter timeout: 50k iterations (~5-10ms) prevents hanging on absent i8042
    for _ in 0..50_000 {
        let status = port::inb(PS2_STATUS_PORT);
        LAST_STATUS.store(status, Ordering::Relaxed);
        if status & STATUS_OUTPUT_FULL == 0 {
            core::hint::spin_loop();
            continue;
        }

        let byte = read_data_port(context, status);
        if status & STATUS_AUX_DATA == 0 {
            return Some(byte);
        }
        crate::serial_println!(
            "[kbd] {} ignoring aux byte=0x{:02X} status=0x{:02X}",
            context,
            byte,
            status
        );
    }
    None
}

fn drain_output_buffer(max_reads: usize) {
    for _ in 0..max_reads {
        let status = port::inb(PS2_STATUS_PORT);
        LAST_STATUS.store(status, Ordering::Relaxed);
        if status & STATUS_OUTPUT_FULL == 0 {
            break;
        }
        let byte = read_data_port("drain", status);
        crate::serial_println!(
            "[kbd] drain byte=0x{:02X} status=0x{:02X}",
            byte,
            status
        );
    }
}

fn wait_input_ready() -> bool {
    for _ in 0..50_000 {
        let status = port::inb(PS2_STATUS_PORT);
        LAST_STATUS.store(status, Ordering::Relaxed);
        if status & STATUS_INPUT_FULL == 0 {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

fn sample_status(context: &str) -> u8 {
    let status = port::inb(PS2_STATUS_PORT);
    LAST_STATUS.store(status, Ordering::Relaxed);
    crate::serial_println!("[kbd] status {} 0x64=0x{:02X}", context, status);
    status
}

fn read_data_port(context: &str, status: u8) -> u8 {
    let byte = port::inb(PS2_DATA_PORT);
    LAST_STATUS.store(status, Ordering::Relaxed);
    if KBD_TRACE_PORT_IO_ENABLED {
        // Rate-limit hot-path logging — only log first 16 reads and then every 256th.
        let count = READ_DATA_LOG_COUNT.fetch_add(1, Ordering::Relaxed);
        if count < 16 || count & 0xFF == 0 {
            crate::serial_println!(
                "[kbd] {} read 0x60=0x{:02X} status=0x{:02X} n={}",
                context,
                byte,
                status,
                count
            );
        }
    }
    byte
}

static READ_DATA_LOG_COUNT: AtomicU32 = AtomicU32::new(0);

fn log_pic_masks(context: &str) {
    let masks = pic::read_masks();
    crate::serial_println!(
        "[kbd] PIC masks ({}) primary=0x{:02X} secondary=0x{:02X} irq1_unmasked={}",
        context,
        masks[0],
        masks[1],
        if masks[0] & (1 << 1) == 0 { "yes" } else { "no" }
    );
}

fn uptime_millis() -> u64 {
    crate::arch::x86_64::pit::elapsed_millis(crate::arch::x86_64::interrupts::tick_count())
}

fn likely_i8042_absent() -> bool {
    let mut ff_samples = 0u8;
    for _ in 0..4 {
        let status = port::inb(PS2_STATUS_PORT);
        let data = port::inb(PS2_DATA_PORT);
        if status == 0xFF && data == 0xFF {
            ff_samples = ff_samples.saturating_add(1);
        }
    }
    ff_samples == 4
}

fn format_byte(value: Option<u8>) -> OptionalHexByte {
    OptionalHexByte(value)
}

impl fmt::Display for OptionalHexByte {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(value) => write!(formatter, "0x{value:02X}"),
            None => formatter.write_str("none"),
        }
    }
}
