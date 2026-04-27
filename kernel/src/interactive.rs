use alloc::format;
use alloc::string::String;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use crate::boot::trace::{self, BootStage, BreadcrumbTag};
use crate::serial_println;

const SERVICE_LOG_INTERVAL_TICKS: u64 = 25;
const WAIT_LOG_INTERVAL_TICKS: u64 = 100;
const USB_TOPOLOGY_SERVICE_INTERVAL_TICKS: u64 = 10;
const SHELL_TASK_SERVICE_INTERVAL_TICKS: u64 = 25;
const SHELL_NET_SERVICE_INTERVAL_TICKS: u64 = 50;
const SHELL_USB_TOPOLOGY_SERVICE_INTERVAL_TICKS: u64 = 50;
const SHELL_PROMPT_WARM_TASK_SERVICE_INTERVAL_TICKS: u64 = 100;
const SHELL_PROMPT_WARM_NET_SERVICE_INTERVAL_TICKS: u64 = 200;
const SHELL_PROMPT_WARM_USB_TOPOLOGY_SERVICE_INTERVAL_TICKS: u64 = 200;
const SHELL_EMPTY_PROMPT_TASK_SERVICE_INTERVAL_TICKS: u64 = 200;
const SHELL_EMPTY_PROMPT_NET_SERVICE_INTERVAL_TICKS: u64 = 400;
const SHELL_EMPTY_PROMPT_USB_TOPOLOGY_SERVICE_INTERVAL_TICKS: u64 = 400;
const SHELL_IDLE_TASK_SERVICE_INTERVAL_TICKS: u64 = 200;
const SHELL_IDLE_NET_SERVICE_INTERVAL_TICKS: u64 = 400;
const SHELL_IDLE_USB_TOPOLOGY_SERVICE_INTERVAL_TICKS: u64 = 400;
const SHELL_RECENT_INPUT_GRACE_TICKS: u64 = 100;
const SHELL_FAST_MODE_GRACE_TICKS: u64 = 64;
const SHELL_EDITED_EMPTY_FAST_GRACE_TICKS: u64 = 256;
const SHELL_REPROMPT_FAST_GRACE_TICKS: u64 = 600;
const SHELL_EDITED_EMPTY_WARM_GRACE_TICKS: u64 = 1200;
const SHELL_PROMPT_WARM_GRACE_TICKS: u64 = 1200;
const SHELL_EMPTY_PROMPT_ARMED_GRACE_TICKS: u64 = 4800;
const PROGRESS_SERIAL_ENABLED: bool = false;
const INTERACTIVE_IO_TRACE_ENABLED: bool = false;
const INTERACTIVE_RUNTIME_TRACE_ENABLED: bool = false;

static CURRENT_STAGE: AtomicU32 = AtomicU32::new(InteractiveStage::None as u32);
static ENTER_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static LAST_SERVICE_LOG_TICK: AtomicU64 = AtomicU64::new(0);
static LAST_WAIT_LOG_TICK: AtomicU64 = AtomicU64::new(0);
static LAST_TASK_TICK: AtomicU64 = AtomicU64::new(u64::MAX);
static LAST_NET_POLL_TICK: AtomicU64 = AtomicU64::new(u64::MAX);
static LAST_USB_TOPOLOGY_SERVICE_TICK: AtomicU64 = AtomicU64::new(0);
static LAST_INPUT_ACTIVITY_TICK: AtomicU64 = AtomicU64::new(0);
static SHELL_FAST_MODE_UNTIL_TICK: AtomicU64 = AtomicU64::new(0);
static SHELL_LINE_ACTIVE: AtomicBool = AtomicBool::new(false);
static SHELL_EDITED_EMPTY_WARM_UNTIL_TICK: AtomicU64 = AtomicU64::new(0);
static SHELL_PROMPT_WARM_UNTIL_TICK: AtomicU64 = AtomicU64::new(0);
static SHELL_LAST_REPROMPT_TICK: AtomicU64 = AtomicU64::new(0);
static SHELL_EMPTY_PROMPT_ARMED_UNTIL_TICK: AtomicU64 = AtomicU64::new(0);
static SHELL_LAST_EMPTY_PROMPT_TICK: AtomicU64 = AtomicU64::new(0);
static SHELL_LAST_EMPTY_REASON: AtomicU32 = AtomicU32::new(ShellEmptyReason::None as u32);
static SHELL_LAST_IDLE_REARM_TICK: AtomicU64 = AtomicU64::new(0);
static SHELL_IDLE_REARM_COUNT: AtomicU32 = AtomicU32::new(0);

/// Monotonic progress counter — incremented at every execution checkpoint
/// in the boot-to-interactive path. If this counter stops advancing, the
/// CPU has deadlocked or crashed at the last reported checkpoint.
static PROGRESS_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Log a progress checkpoint. The counter is monotonically increasing and
/// lock-free (atomic only). If serial output stops, the last printed
/// checkpoint number identifies the exact freeze point.
pub fn progress(label: &str) {
    let n = PROGRESS_COUNTER.fetch_add(1, Ordering::Relaxed);
    if !PROGRESS_SERIAL_ENABLED {
        let _ = label;
        return;
    }
    let ticks = crate::arch::x86_64::interrupts::tick_count();
    serial_println!(
        "[PROGRESS] n={} ticks={} IF={} label={}",
        n,
        ticks,
        if x86_64::instructions::interrupts::are_enabled() { "on" } else { "off" },
        label
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum InteractiveStage {
    None = 0,
    AuthPreferences,
    AuthReadLineEcho,
    AuthReadLineHidden,
    Shell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ShellEmptyReason {
    None = 0,
    StartupPrompt = 1,
    Reprompt = 2,
    Backspace = 3,
    EscClear = 4,
    HistoryEmpty = 5,
    Other = 6,
}

impl ShellEmptyReason {
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::StartupPrompt => "startup",
            Self::Reprompt => "reprompt",
            Self::Backspace => "backspace",
            Self::EscClear => "esc-clear",
            Self::HistoryEmpty => "history-empty",
            Self::Other => "other",
        }
    }

    fn is_edited(self) -> bool {
        matches!(self, Self::Backspace | Self::EscClear | Self::HistoryEmpty | Self::Other)
    }
}

pub fn enter(stage: InteractiveStage, reason: &str) {
    CURRENT_STAGE.store(stage as u32, Ordering::Relaxed);
    match stage {
        InteractiveStage::AuthPreferences
        | InteractiveStage::AuthReadLineEcho
        | InteractiveStage::AuthReadLineHidden => {
            crate::display::console::claim_screen_owner(
                crate::display::console::ScreenOwner::Auth,
                reason,
            );
        }
        InteractiveStage::Shell => {
            SHELL_FAST_MODE_UNTIL_TICK.store(0, Ordering::Relaxed);
            SHELL_LINE_ACTIVE.store(false, Ordering::Relaxed);
            SHELL_EDITED_EMPTY_WARM_UNTIL_TICK.store(0, Ordering::Relaxed);
            SHELL_PROMPT_WARM_UNTIL_TICK.store(0, Ordering::Relaxed);
            SHELL_LAST_REPROMPT_TICK.store(0, Ordering::Relaxed);
            SHELL_EMPTY_PROMPT_ARMED_UNTIL_TICK.store(0, Ordering::Relaxed);
            SHELL_LAST_EMPTY_PROMPT_TICK.store(0, Ordering::Relaxed);
            SHELL_LAST_EMPTY_REASON.store(ShellEmptyReason::None as u32, Ordering::Relaxed);
            SHELL_LAST_IDLE_REARM_TICK.store(0, Ordering::Relaxed);
            SHELL_IDLE_REARM_COUNT.store(0, Ordering::Relaxed);
            crate::display::console::claim_screen_owner(
                crate::display::console::ScreenOwner::Shell,
                reason,
            );
        }
        InteractiveStage::None => {}
    }
    let sequence = ENTER_SEQUENCE.fetch_add(1, Ordering::Relaxed).saturating_add(1);
    if INTERACTIVE_RUNTIME_TRACE_ENABLED {
        let ticks = crate::arch::x86_64::interrupts::tick_count();
        serial_println!(
            "[interactive] enter seq={} stage={} reason={} ticks={} IF={}",
            sequence,
            stage.label(),
            reason,
            ticks,
            interrupt_state()
        );
    } else {
        let _ = (sequence, reason);
    }
    if matches!(stage, InteractiveStage::AuthPreferences) {
        trace::set_stage(BootStage::Interactive, BreadcrumbTag::InteractiveReady);
    } else if matches!(stage, InteractiveStage::Shell) {
        trace::set_stage(BootStage::Shell, BreadcrumbTag::ShellReady);
    }
}

pub fn render(stage: InteractiveStage, reason: &str) {
    if !INTERACTIVE_RUNTIME_TRACE_ENABLED {
        let _ = (stage, reason);
        return;
    }
    let diag = crate::hal::input::diagnostic_snapshot();
    serial_println!(
        "[interactive] render stage={} reason={} ticks={} queue={} stall={} last_src={} last_key=0x{:02X}",
        stage.label(),
        reason,
        diag.ticks,
        diag.key_queue_len,
        diag.stall_count,
        source_label(diag.last_source),
        diag.last_keycode
    );
}

pub fn service(stage: InteractiveStage, reason: &str) {
    crate::heartbeat_log_if_due("interactive-service");
    let tick = crate::arch::x86_64::interrupts::tick_count();
    // Poll the full input path first so raw hardware-ready input is visible
    // before we decide whether background work may run.
    let mut pending_input = crate::hal::input::input_poll();
    let recent_input_grace = if matches!(stage, InteractiveStage::Shell) {
        SHELL_RECENT_INPUT_GRACE_TICKS
    } else {
        2
    };
    let recent_input =
        tick.saturating_sub(LAST_INPUT_ACTIVITY_TICK.load(Ordering::Relaxed)) < recent_input_grace;
    let shell_fast_mode = matches!(stage, InteractiveStage::Shell) && shell_fast_mode_active(tick);
    let shell_edited_empty_warm =
        matches!(stage, InteractiveStage::Shell) && shell_edited_empty_warm_active(tick);
    let shell_prompt_warm =
        matches!(stage, InteractiveStage::Shell) && shell_prompt_warm_active(tick);
    let shell_empty_prompt_armed =
        matches!(stage, InteractiveStage::Shell) && shell_empty_prompt_armed_active(tick);
    let shell_line_active =
        matches!(stage, InteractiveStage::Shell) && SHELL_LINE_ACTIVE.load(Ordering::Relaxed);
    let shell_empty_prompt_cold = matches!(stage, InteractiveStage::Shell)
        && !shell_line_active
        && !shell_fast_mode
        && !shell_prompt_warm
        && !shell_edited_empty_warm
        && !shell_empty_prompt_armed;
    if matches!(stage, InteractiveStage::Shell)
        && !pending_input
        && !recent_input
        && !shell_fast_mode
        && !shell_line_active
        && (shell_prompt_warm
            || shell_edited_empty_warm
            || shell_empty_prompt_armed
            || shell_empty_prompt_cold)
    {
        note_shell_idle_rearm(tick);
        pending_input = crate::hal::input::input_poll() || crate::hal::input::has_pending_input();
    }
    let shell_hot_path_busy = matches!(stage, InteractiveStage::Shell)
        && (pending_input
            || recent_input
            || shell_fast_mode
            || shell_line_active
            || shell_prompt_warm
            || shell_edited_empty_warm);
    let last = LAST_TASK_TICK.load(Ordering::Relaxed);
    let task_interval = if matches!(stage, InteractiveStage::Shell) {
        if shell_line_active || shell_fast_mode {
            SHELL_TASK_SERVICE_INTERVAL_TICKS
        } else if shell_prompt_warm || shell_edited_empty_warm {
            SHELL_PROMPT_WARM_TASK_SERVICE_INTERVAL_TICKS
        } else if shell_empty_prompt_armed {
            SHELL_EMPTY_PROMPT_TASK_SERVICE_INTERVAL_TICKS
        } else {
            SHELL_IDLE_TASK_SERVICE_INTERVAL_TICKS
        }
    } else {
        2
    };
    let run_task_tick = !shell_hot_path_busy
        && crate::task::has_tasks()
        && !pending_input
        && tick.saturating_sub(last) >= task_interval;
    if run_task_tick {
        LAST_TASK_TICK.store(tick, Ordering::Relaxed);
        crate::task::tick();
    }
    if matches!(stage, InteractiveStage::Shell) {
        let last_net_poll = LAST_NET_POLL_TICK.load(Ordering::Relaxed);
        let net_interval = if shell_line_active || shell_fast_mode {
            SHELL_NET_SERVICE_INTERVAL_TICKS
        } else if shell_prompt_warm || shell_edited_empty_warm {
            SHELL_PROMPT_WARM_NET_SERVICE_INTERVAL_TICKS
        } else if shell_empty_prompt_armed {
            SHELL_EMPTY_PROMPT_NET_SERVICE_INTERVAL_TICKS
        } else {
            SHELL_IDLE_NET_SERVICE_INTERVAL_TICKS
        };
        if !shell_hot_path_busy
            && (last_net_poll == u64::MAX || tick.saturating_sub(last_net_poll) >= net_interval)
        {
            LAST_NET_POLL_TICK.store(tick, Ordering::Relaxed);
            let _ = crate::net::poll();
        }
        if !shell_hot_path_busy {
            let last_usb = LAST_USB_TOPOLOGY_SERVICE_TICK.load(Ordering::Relaxed);
            let usb_interval = if shell_line_active || shell_fast_mode {
                SHELL_USB_TOPOLOGY_SERVICE_INTERVAL_TICKS
            } else if shell_prompt_warm || shell_edited_empty_warm {
                SHELL_PROMPT_WARM_USB_TOPOLOGY_SERVICE_INTERVAL_TICKS
            } else if shell_empty_prompt_armed {
                SHELL_EMPTY_PROMPT_USB_TOPOLOGY_SERVICE_INTERVAL_TICKS
            } else {
                SHELL_IDLE_USB_TOPOLOGY_SERVICE_INTERVAL_TICKS
            };
            if tick.saturating_sub(last_usb) >= usb_interval {
                LAST_USB_TOPOLOGY_SERVICE_TICK.store(tick, Ordering::Relaxed);
                let _ = crate::hal::usb::service_pending_topology();
            }
        }
    }
    crate::display::console::refresh_input_cursor();
    maybe_log_service(stage, reason);
}

pub fn idle_wait(stage: InteractiveStage, reason: &str) {
    maybe_log_wait(stage, reason);
    if matches!(stage, InteractiveStage::Shell) {
        let tick = crate::arch::x86_64::interrupts::tick_count();
        if shell_prompt_warm_active(tick) || shell_edited_empty_warm_active(tick) {
            crate::hal::input::wait_for_shell_activity_prompt_warm();
        } else if shell_empty_prompt_armed_active(tick) {
            crate::hal::input::wait_for_shell_activity_empty_prompt();
        } else if shell_fast_mode_active(tick) {
            crate::hal::input::wait_for_shell_activity_fast();
        } else {
            crate::hal::input::wait_for_shell_activity();
        }
    } else {
        crate::hal::input::wait_for_activity();
    }
}

pub fn poll_keypress(
    stage: InteractiveStage,
    consumer: &str,
) -> Option<crate::hal::input::InputKeypress> {
    service(stage, consumer);
    dequeue_keypress(stage, consumer)
}

pub fn try_read_ready_keypress(
    stage: InteractiveStage,
    consumer: &str,
) -> Option<crate::hal::input::InputKeypress> {
    if matches!(stage, InteractiveStage::Shell) {
        let _ = crate::hal::input::input_poll();
    }
    dequeue_keypress(stage, consumer)
}

pub fn try_read_ready_input(stage: InteractiveStage, consumer: &str) -> Option<u8> {
    try_read_ready_keypress(stage, consumer).map(|keypress| keypress.byte)
}

fn dequeue_keypress(
    stage: InteractiveStage,
    consumer: &str,
) -> Option<crate::hal::input::InputKeypress> {
    let keypress = crate::hal::input::try_read_keypress()?;
    let tick = crate::arch::x86_64::interrupts::tick_count();
    LAST_INPUT_ACTIVITY_TICK.store(tick, Ordering::Relaxed);
    if matches!(stage, InteractiveStage::Shell) {
        SHELL_FAST_MODE_UNTIL_TICK.store(
            tick.saturating_add(SHELL_FAST_MODE_GRACE_TICKS),
            Ordering::Relaxed,
        );
    }
    crate::hal::input::trace_stage_ui(
        keypress.trace_id,
        keypress.source,
        stage.label(),
        consumer,
        keypress.byte,
    );
    note_input(stage, consumer, keypress.byte, "dequeue");
    Some(keypress)
}

#[derive(Clone, Copy)]
pub struct ShellInputModeSnapshot {
    pub line_active: bool,
    pub fast_mode: bool,
    pub edited_empty_warm: bool,
    pub prompt_warm: bool,
    pub empty_prompt_armed: bool,
    pub since_input_ticks: u64,
    pub since_reprompt_ticks: u64,
    pub since_empty_prompt_ticks: u64,
    pub last_empty_prompt_tick: u64,
    pub last_empty_reason: ShellEmptyReason,
    pub idle_rearm_count: u32,
    pub since_idle_rearm_ticks: u64,
}

pub fn note_shell_input_activity(line_active: bool) {
    let tick = crate::arch::x86_64::interrupts::tick_count();
    LAST_INPUT_ACTIVITY_TICK.store(tick, Ordering::Relaxed);
    SHELL_LINE_ACTIVE.store(line_active, Ordering::Relaxed);
    if line_active {
        SHELL_EDITED_EMPTY_WARM_UNTIL_TICK.store(0, Ordering::Relaxed);
        SHELL_PROMPT_WARM_UNTIL_TICK.store(0, Ordering::Relaxed);
        SHELL_EMPTY_PROMPT_ARMED_UNTIL_TICK.store(0, Ordering::Relaxed);
    }
    SHELL_FAST_MODE_UNTIL_TICK.store(
        tick.saturating_add(SHELL_FAST_MODE_GRACE_TICKS),
        Ordering::Relaxed,
    );
}

pub fn note_shell_reprompt() {
    let tick = crate::arch::x86_64::interrupts::tick_count();
    arm_shell_empty_prompt(tick, ShellEmptyReason::Reprompt);
    SHELL_FAST_MODE_UNTIL_TICK.store(
        tick.saturating_add(SHELL_REPROMPT_FAST_GRACE_TICKS),
        Ordering::Relaxed,
    );
    SHELL_EDITED_EMPTY_WARM_UNTIL_TICK.store(0, Ordering::Relaxed);
    SHELL_PROMPT_WARM_UNTIL_TICK.store(
        tick.saturating_add(SHELL_PROMPT_WARM_GRACE_TICKS),
        Ordering::Relaxed,
    );
    SHELL_LAST_REPROMPT_TICK.store(tick, Ordering::Relaxed);
}

pub fn note_shell_startup_prompt() {
    let tick = crate::arch::x86_64::interrupts::tick_count();
    arm_shell_empty_prompt(tick, ShellEmptyReason::StartupPrompt);
    SHELL_FAST_MODE_UNTIL_TICK.store(
        tick.saturating_add(SHELL_REPROMPT_FAST_GRACE_TICKS),
        Ordering::Relaxed,
    );
    SHELL_EDITED_EMPTY_WARM_UNTIL_TICK.store(0, Ordering::Relaxed);
    SHELL_PROMPT_WARM_UNTIL_TICK.store(
        tick.saturating_add(SHELL_PROMPT_WARM_GRACE_TICKS),
        Ordering::Relaxed,
    );
    SHELL_LAST_REPROMPT_TICK.store(tick, Ordering::Relaxed);
}

pub fn note_shell_empty_prompt_armed(reason: ShellEmptyReason) {
    let tick = crate::arch::x86_64::interrupts::tick_count();
    arm_shell_empty_prompt(tick, reason);
    SHELL_PROMPT_WARM_UNTIL_TICK.store(0, Ordering::Relaxed);
    if reason.is_edited() {
        SHELL_FAST_MODE_UNTIL_TICK.store(
            tick.saturating_add(SHELL_EDITED_EMPTY_FAST_GRACE_TICKS),
            Ordering::Relaxed,
        );
        SHELL_EDITED_EMPTY_WARM_UNTIL_TICK.store(
            tick.saturating_add(SHELL_EDITED_EMPTY_WARM_GRACE_TICKS),
            Ordering::Relaxed,
        );
    } else {
        SHELL_EDITED_EMPTY_WARM_UNTIL_TICK.store(0, Ordering::Relaxed);
    }
}

pub fn shell_input_mode_snapshot() -> ShellInputModeSnapshot {
    let tick = crate::arch::x86_64::interrupts::tick_count();
    let since_input_ticks = tick.saturating_sub(LAST_INPUT_ACTIVITY_TICK.load(Ordering::Relaxed));
    let last_idle_rearm_tick = SHELL_LAST_IDLE_REARM_TICK.load(Ordering::Relaxed);
    let last_reprompt_tick = SHELL_LAST_REPROMPT_TICK.load(Ordering::Relaxed);
    let last_empty_prompt_tick = SHELL_LAST_EMPTY_PROMPT_TICK.load(Ordering::Relaxed);
    ShellInputModeSnapshot {
        line_active: SHELL_LINE_ACTIVE.load(Ordering::Relaxed),
        fast_mode: shell_fast_mode_active(tick),
        edited_empty_warm: shell_edited_empty_warm_active(tick),
        prompt_warm: shell_prompt_warm_active(tick),
        empty_prompt_armed: shell_empty_prompt_armed_active(tick),
        since_input_ticks,
        since_reprompt_ticks: tick.saturating_sub(last_reprompt_tick),
        since_empty_prompt_ticks: tick.saturating_sub(last_empty_prompt_tick),
        last_empty_prompt_tick,
        last_empty_reason: shell_empty_reason_from_u32(
            SHELL_LAST_EMPTY_REASON.load(Ordering::Relaxed),
        ),
        idle_rearm_count: SHELL_IDLE_REARM_COUNT.load(Ordering::Relaxed),
        since_idle_rearm_ticks: tick.saturating_sub(last_idle_rearm_tick),
    }
}

pub fn poll_input(stage: InteractiveStage, consumer: &str) -> Option<u8> {
    poll_keypress(stage, consumer).map(|keypress| keypress.byte)
}

pub fn read_keypress(
    stage: InteractiveStage,
    consumer: &str,
    timeout_ticks: Option<u64>,
) -> Option<crate::hal::input::InputKeypress> {
    let start_tick = crate::arch::x86_64::interrupts::tick_count();
    loop {
        if let Some(keypress) = poll_keypress(stage, consumer) {
            return Some(keypress);
        }

        let elapsed = crate::arch::x86_64::interrupts::tick_count().saturating_sub(start_tick);
        if timeout_ticks.is_some_and(|limit| elapsed >= limit) {
            note_timeout(stage, consumer, elapsed);
            return None;
        }

        idle_wait(stage, consumer);
    }
}

pub fn read_input(
    stage: InteractiveStage,
    consumer: &str,
    timeout_ticks: Option<u64>,
) -> Option<u8> {
    let start_tick = crate::arch::x86_64::interrupts::tick_count();
    let timeout_label = timeout_ticks.map_or(String::from("blocking"), |ticks| format!("{ticks}"));
    if INTERACTIVE_IO_TRACE_ENABLED {
        serial_println!(
            "[interactive] read stage={} consumer={} timeout_ticks={}",
            stage.label(),
            consumer,
            timeout_label
        );
        serial_println!(
            "[INTERACTIVE] entering loop stage={} consumer={}",
            stage.label(),
            consumer
        );
    }

    loop {
        if INTERACTIVE_IO_TRACE_ENABLED {
            serial_println!(
                "[INTERACTIVE] tick stage={} consumer={}",
                stage.label(),
                consumer
            );
        }
        if let Some(byte) = poll_input(stage, consumer) {
            if INTERACTIVE_IO_TRACE_ENABLED {
                serial_println!(
                    "[INTERACTIVE] processing input stage={} consumer={} byte=0x{:02X}",
                    stage.label(),
                    consumer,
                    byte
                );
            }
            return Some(byte);
        }

        let elapsed = crate::arch::x86_64::interrupts::tick_count().saturating_sub(start_tick);
        if timeout_ticks.is_some_and(|limit| elapsed >= limit) {
            note_timeout(stage, consumer, elapsed);
            return None;
        }

        if INTERACTIVE_IO_TRACE_ENABLED {
            serial_println!(
                "[INTERACTIVE] waiting input stage={} consumer={} elapsed_ticks={}",
                stage.label(),
                consumer,
                elapsed
            );
        }
        idle_wait(stage, consumer);
    }
}

pub fn note_input(stage: InteractiveStage, consumer: &str, byte: u8, outcome: &str) {
    if !INTERACTIVE_RUNTIME_TRACE_ENABLED {
        let _ = (stage, consumer, byte, outcome);
        return;
    }
    let diag = crate::hal::input::diagnostic_snapshot();
    serial_println!(
        "[interactive] input stage={} consumer={} outcome={} byte=0x{:02X} '{}' ticks={} queue={} stall={} src={} last_key=0x{:02X}",
        stage.label(),
        consumer,
        outcome,
        byte,
        printable(byte),
        diag.ticks,
        diag.key_queue_len,
        diag.stall_count,
        source_label(diag.last_source),
        diag.last_keycode
    );
}

pub fn note_timeout(stage: InteractiveStage, consumer: &str, elapsed_ticks: u64) {
    if !INTERACTIVE_RUNTIME_TRACE_ENABLED {
        let _ = (stage, consumer, elapsed_ticks);
        return;
    }
    serial_println!(
        "[interactive] timeout stage={} consumer={} elapsed_ticks={}",
        stage.label(),
        consumer,
        elapsed_ticks
    );
    log_pending_work(stage, "timeout");
}

pub fn log_pending_work(stage: InteractiveStage, reason: &str) {
    if !INTERACTIVE_RUNTIME_TRACE_ENABLED {
        let _ = (stage, reason);
        return;
    }
    let (tasks, processes, current_pid, ctx_switches) = x86_64::instructions::interrupts::without_interrupts(|| {
        (
            crate::task::snapshot(),
            crate::exec::snapshot(),
            crate::exec::current_pid(),
            crate::exec::context_switch_count(),
        )
    });
    let active_processes = processes
        .iter()
        .filter(|process| process.state != crate::exec::process::ProcessState::Zombie)
        .count();

    serial_println!(
        "[interactive] work stage={} reason={} current_pid={:?} ctx_switches={} tasks={} active_processes={}",
        stage.label(),
        reason,
        current_pid,
        ctx_switches,
        tasks.len(),
        active_processes
    );

    for task in tasks.iter().take(8) {
        serial_println!(
            "[interactive] task id={} state={:?} name={}",
            task.id,
            task.state,
            task.name
        );
    }

    for process in processes
        .iter()
        .filter(|process| process.state != crate::exec::process::ProcessState::Zombie)
        .take(8)
    {
        serial_println!(
            "[interactive] proc pid={} uid={} state={:?} prio={:?} image={:?} name={}",
            process.pid,
            process.uid,
            process.state,
            process.priority,
            process.image_kind,
            process.name
        );
    }
}

fn maybe_log_service(stage: InteractiveStage, reason: &str) {
    if !INTERACTIVE_RUNTIME_TRACE_ENABLED {
        let _ = (stage, reason);
        return;
    }
    let ticks = crate::arch::x86_64::interrupts::tick_count();
    let last = LAST_SERVICE_LOG_TICK.load(Ordering::Relaxed);
    if ticks.saturating_sub(last) < SERVICE_LOG_INTERVAL_TICKS {
        return;
    }
    LAST_SERVICE_LOG_TICK.store(ticks, Ordering::Relaxed);

    let diag = crate::hal::input::diagnostic_snapshot();
    let (current_pid, ctx_switches) = x86_64::instructions::interrupts::without_interrupts(|| {
        (crate::exec::current_pid(), crate::exec::context_switch_count())
    });
    serial_println!(
        "[interactive] step stage={} reason={} ticks={} IF={} current_pid={:?} ctx_switches={} pending_input={} queue={} stall={} src={} last_key=0x{:02X}",
        stage.label(),
        reason,
        ticks,
        interrupt_state(),
        current_pid,
        ctx_switches,
        crate::hal::input::has_pending_input(),
        diag.key_queue_len,
        diag.stall_count,
        source_label(diag.last_source),
        diag.last_keycode
    );
}

fn maybe_log_wait(stage: InteractiveStage, reason: &str) {
    if !INTERACTIVE_RUNTIME_TRACE_ENABLED {
        let _ = (stage, reason);
        return;
    }
    let ticks = crate::arch::x86_64::interrupts::tick_count();
    let last = LAST_WAIT_LOG_TICK.load(Ordering::Relaxed);
    if ticks.saturating_sub(last) < WAIT_LOG_INTERVAL_TICKS {
        return;
    }
    LAST_WAIT_LOG_TICK.store(ticks, Ordering::Relaxed);

    let current_pid = x86_64::instructions::interrupts::without_interrupts(crate::exec::current_pid);
    serial_println!(
        "[interactive] wait stage={} reason={} ticks={} tasks={} current_pid={:?}",
        stage.label(),
        reason,
        ticks,
        crate::task::has_tasks(),
        current_pid
    );
}

fn shell_fast_mode_active(tick: u64) -> bool {
    SHELL_LINE_ACTIVE.load(Ordering::Relaxed)
        || tick <= SHELL_FAST_MODE_UNTIL_TICK.load(Ordering::Relaxed)
}

fn shell_edited_empty_warm_active(tick: u64) -> bool {
    tick <= SHELL_EDITED_EMPTY_WARM_UNTIL_TICK.load(Ordering::Relaxed)
}

fn shell_prompt_warm_active(tick: u64) -> bool {
    tick <= SHELL_PROMPT_WARM_UNTIL_TICK.load(Ordering::Relaxed)
}

fn shell_empty_prompt_armed_active(tick: u64) -> bool {
    tick <= SHELL_EMPTY_PROMPT_ARMED_UNTIL_TICK.load(Ordering::Relaxed)
}

fn arm_shell_empty_prompt(tick: u64, reason: ShellEmptyReason) {
    LAST_INPUT_ACTIVITY_TICK.store(tick, Ordering::Relaxed);
    SHELL_LINE_ACTIVE.store(false, Ordering::Relaxed);
    SHELL_EMPTY_PROMPT_ARMED_UNTIL_TICK.store(
        tick.saturating_add(SHELL_EMPTY_PROMPT_ARMED_GRACE_TICKS),
        Ordering::Relaxed,
    );
    SHELL_LAST_EMPTY_PROMPT_TICK.store(tick, Ordering::Relaxed);
    SHELL_LAST_EMPTY_REASON.store(reason as u32, Ordering::Relaxed);
}

fn shell_empty_reason_from_u32(raw: u32) -> ShellEmptyReason {
    match raw {
        x if x == ShellEmptyReason::StartupPrompt as u32 => ShellEmptyReason::StartupPrompt,
        x if x == ShellEmptyReason::Reprompt as u32 => ShellEmptyReason::Reprompt,
        x if x == ShellEmptyReason::Backspace as u32 => ShellEmptyReason::Backspace,
        x if x == ShellEmptyReason::EscClear as u32 => ShellEmptyReason::EscClear,
        x if x == ShellEmptyReason::HistoryEmpty as u32 => ShellEmptyReason::HistoryEmpty,
        x if x == ShellEmptyReason::Other as u32 => ShellEmptyReason::Other,
        _ => ShellEmptyReason::None,
    }
}

fn note_shell_idle_rearm(tick: u64) {
    let last = SHELL_LAST_IDLE_REARM_TICK.load(Ordering::Relaxed);
    if last == tick {
        return;
    }
    SHELL_LAST_IDLE_REARM_TICK.store(tick, Ordering::Relaxed);
    SHELL_IDLE_REARM_COUNT.fetch_add(1, Ordering::Relaxed);
}

fn interrupt_state() -> &'static str {
    if x86_64::instructions::interrupts::are_enabled() {
        "on"
    } else {
        "off"
    }
}

fn printable(byte: u8) -> char {
    if byte.is_ascii_graphic() || byte == b' ' {
        byte as char
    } else {
        '.'
    }
}

fn source_label(source: u8) -> &'static str {
    match source {
        1 => "ps2",
        2 => "usb",
        _ => "none",
    }
}

impl InteractiveStage {
    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::AuthPreferences => "auth-preferences",
            Self::AuthReadLineEcho => "auth-read-line-echo",
            Self::AuthReadLineHidden => "auth-read-line-hidden",
            Self::Shell => "shell",
        }
    }
}

/// Minimal interactive mode — bypasses ALL auth/exec/shell/task locks.
/// Proves that framebuffer + timer + raw input work on real hardware.
/// If THIS freezes, the bug is in the display/input/IRQ subsystem.
/// If this works but normal boot freezes, the bug is in auth/exec locks.
pub fn minimal_interactive_mode() -> ! {
    progress("minimal-mode-enter");
    serial_println!("[MINIMAL] === MINIMAL INTERACTIVE MODE ===");
    serial_println!("[MINIMAL] No auth, no exec, no shell. Pure hardware test.");

    // Ensure interrupts are on
    if !x86_64::instructions::interrupts::are_enabled() {
        x86_64::instructions::interrupts::enable();
    }
    progress("minimal-mode-interrupts-on");

    // Clear screen and draw a simple banner
    {
        use crate::display::console::CONSOLE;
        if let Some(ref mut console) = *CONSOLE.lock() {
            console.clear_screen();
        }
    }
    crate::kprintln!();
    crate::kprint_colored!(Colors::GREEN, "  WarOS Minimal Interactive Mode\n");
    crate::kprint_colored!(Colors::DIM, "  --------------------------------\n");
    crate::kprint_colored!(Colors::DIM, "  Bypassing auth/exec/shell.\n");
    crate::kprint_colored!(Colors::DIM, "  Type any key — it should echo below.\n");
    crate::kprint_colored!(Colors::DIM, "  Timer ticks should advance on serial.\n\n");
    crate::kprint_colored!(Colors::DIM, "  > ");

    progress("minimal-mode-banner-drawn");

    let mut tick_count = 0u64;
    let mut last_diag_tick = 0u64;

    loop {
        // 1. Poll USB HID
        crate::hal::usb::poll_runtime();

        // 2. Try read from unified input (USB + PS/2)
        if let Some(keypress) = crate::hal::input::try_read_keypress() {
            serial_println!(
                "[MINIMAL] KEY byte=0x{:02X} '{}' source={} keycode=0x{:02X}",
                keypress.byte,
                printable(keypress.byte),
                source_label(keypress.source),
                keypress.keycode
            );
            // Echo to screen
            let ch = printable(keypress.byte);
            if keypress.byte == b'\n' {
                crate::kprintln!();
                crate::kprint_colored!(Colors::DIM, "  > ");
            } else {
                crate::kprint!("{}", ch);
            }
        }

        // 3. Periodic diagnostic (every ~2 seconds = 200 ticks at 100Hz)
        let ticks = crate::arch::x86_64::interrupts::tick_count();
        if ticks.saturating_sub(last_diag_tick) >= 200 {
            last_diag_tick = ticks;
            tick_count += 1;
            let diag = crate::hal::input::diagnostic_snapshot();
            serial_println!(
                "[MINIMAL] heartbeat n={} ticks={} IF={} ps2={} usb_ctrl={} usb_kbd={} queue={} stall={}",
                tick_count,
                ticks,
                if x86_64::instructions::interrupts::are_enabled() { "on" } else { "off" },
                diag.ps2_present,
                diag.usb_controllers,
                diag.usb_hid_keyboards,
                diag.key_queue_len,
                diag.stall_count
            );
        }

        // 4. Brief idle to avoid 100% CPU spin
        let _ = crate::arch::x86_64::pit::wait_for_tick_advance(ticks, 1);
    }
}

use crate::display::console::Colors;
