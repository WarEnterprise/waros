use alloc::{format, string::String};
use core::sync::atomic::{AtomicU32, Ordering};

use crate::auth::session;
use crate::display::{console, console::Colors, font};
use crate::hal;
use crate::shell::commands::execute_command;
use crate::{kprint, kprint_colored, kprintln};
use spin::{Lazy, Mutex};

pub mod commands;
pub mod history;

const INPUT_LIMIT: usize = 256;
const SHELL_IDLE_READ_TIMEOUT_TICKS: u64 = 1;
const SHELL_ACTIVE_READ_TIMEOUT_TICKS: u64 = 1;
const SHELL_BURST_DRAIN_LIMIT: usize = 64;
const SHELL_TIMEOUT_PANEL_UPDATE_INTERVAL: u32 = 128;
const SHELL_TIMEOUT_PRIME_INTERVAL: u32 = 8;
const SHELL_TRACE_ENABLED: bool = false;
const SHELL_DEBUG_PANEL_ENABLED: bool = true;
static SHELL_TRACE_BUDGET: AtomicU32 = AtomicU32::new(256);
static SHELL_DEBUG_STATE: Lazy<Mutex<ShellDebugState>> =
    Lazy::new(|| Mutex::new(ShellDebugState::default()));

#[derive(Clone, Default)]
struct ShellDebugState {
    current_loop_state: String,
    timeout_count: u32,
    last_recv: String,
    last_recv_tick: u64,
    last_reentry: String,
    last_reentry_tick: u64,
    last_append: String,
    last_append_tick: u64,
    last_echo: String,
    last_echo_tick: u64,
    last_submit: String,
    last_submit_tick: u64,
    last_reprompt_tick: u64,
    last_empty_tick: u64,
    last_empty_reason: String,
    last_prime: String,
    last_prime_tick: u64,
    len: usize,
    mode: String,
    warm: String,
    cursor: String,
    owner: String,
    clobber: String,
}

fn prompt() {
    let Some(user) = session::current_user() else {
        kprint!("login> ");
        return;
    };

    let path = session::current_prompt_path();
    kprint_colored!(Colors::DIM, "[");
    if session::is_recovery_session() {
        kprint_colored!(Colors::YELLOW, "WarOS recovery");
    } else {
        kprint_colored!(Colors::CYAN, "WarOS");
    }
    kprint_colored!(Colors::DIM, " ");
    if user.role == crate::auth::UserRole::Admin {
        kprint_colored!(Colors::RED, "{}", user.username);
    } else {
        kprint_colored!(Colors::GREEN, "{}", user.username);
    }
    kprint_colored!(Colors::DIM, "@waros ");
    kprint_colored!(Colors::BLUE, "{}", path);
    kprint_colored!(Colors::DIM, "]");
    if user.role == crate::auth::UserRole::Admin {
        kprint_colored!(Colors::RED, "# ");
    } else {
        kprint!("$ ");
    }
}

/// Re-render the shell prompt.
pub fn reprompt() {
    prompt();
}

/// Run the interactive WarShell loop until the current session logs out.
pub fn run() {
    let mut input = String::new();
    let mut truncated = false;
    let mut history_index: Option<usize> = None;
    let mut history_draft = String::new();
    let mut timeout_count = 0u32;
    crate::interactive::enter(
        crate::interactive::InteractiveStage::Shell,
        if session::is_recovery_session() {
            "recovery-shell"
        } else {
            "session-shell"
        },
    );
    crate::display::console::claim_screen_owner(
        crate::display::console::ScreenOwner::Shell,
        "shell-run",
    );
    crate::display::console::reset_shell_clobber_debug();
    if !crate::display::console::rendering_enabled() {
        crate::display::console::set_rendering_enabled(true);
    }
    if !x86_64::instructions::interrupts::are_enabled() {
        crate::serial_println!(
            "[SHELL] interrupts were OFF at shell start; forcing enable"
        );
        x86_64::instructions::interrupts::enable();
    }
    trace_shell_event(format_args!(
        "[INPUT_COMPARE] auth_api=interactive::read_input(timeout) shell_api=interactive::read_input(timeout)"
    ));
    shell_debug_reset();
    shell_prime_input_path("shell-run-start", true);
    crate::serial_println!(
        "[interactive_handoff] stage=shell event=begin ticks={} IF={}",
        crate::arch::x86_64::interrupts::tick_count(),
        if x86_64::instructions::interrupts::are_enabled() {
            "on"
        } else {
            "off"
        }
    );
    crate::exec::ensure_shell_process();
    kprint_colored!(Colors::DIM, "\n");
    kprintln!("WarOS shell online. Type 'help' for available commands.");
    prompt();
    crate::display::console::set_input_cursor_enabled(true);
    crate::display::console::force_input_cursor_visible();
    crate::interactive::note_shell_startup_prompt();
    shell_prime_input_path("shell-startup-prompt-warm", false);
    shell_debug_update(|state| {
        state.current_loop_state = String::from("prompt");
    });

    loop {
        if !session::is_logged_in() {
            crate::display::console::set_input_cursor_enabled(false);
            return;
        }
        if !crate::display::console::screen_owner_is(crate::display::console::ScreenOwner::Shell)
        {
            trace_shell_event(format_args!(
                "[SHELL] owner-reclaim from {:?}",
                crate::display::console::current_screen_owner()
            ));
            crate::display::console::claim_screen_owner(
                crate::display::console::ScreenOwner::Shell,
                "shell-loop-owner-reclaim",
            );
            shell_debug_update(|state| {
                state.current_loop_state = String::from("owner-reclaim");
            });
        }

        crate::exec::ensure_shell_process();
        let input_mode = crate::interactive::shell_input_mode_snapshot();

        if let Some(byte) = crate::interactive::read_input(
            crate::interactive::InteractiveStage::Shell,
            "shell-read",
            Some(shell_read_timeout_ticks(
                !input.is_empty(),
                input_mode.prompt_warm,
                input_mode.empty_prompt_armed,
            )),
        ) {
            let reentered_from_idle = timeout_count != 0 && input.is_empty();
            let idle_timeouts = timeout_count;
            let reentry_source = if reentered_from_idle {
                Some(shell_reentry_origin_label())
            } else {
                None
            };
            timeout_count = 0;
            if !process_shell_byte(
                byte,
                &mut input,
                &mut truncated,
                &mut history_index,
                &mut history_draft,
            ) {
                return;
            }
            if let Some(origin) = reentry_source {
                shell_debug_note_reentry(byte, idle_timeouts, origin);
            }
            let mut burst_drained = 0usize;
            while burst_drained < SHELL_BURST_DRAIN_LIMIT {
                let Some(next_byte) = crate::interactive::try_read_ready_input(
                    crate::interactive::InteractiveStage::Shell,
                    "shell-burst",
                ) else {
                    break;
                };
                burst_drained = burst_drained.saturating_add(1);
                if !process_shell_byte(
                    next_byte,
                    &mut input,
                    &mut truncated,
                    &mut history_index,
                    &mut history_draft,
                ) {
                    return;
                }
            }
            if burst_drained != 0 {
                shell_debug_update(|state| {
                    state.current_loop_state = format!("burst+{}", burst_drained);
                });
            }
        } else {
            timeout_count = timeout_count.saturating_add(1);
            trace_shell_event(format_args!(
                "[SHELL] timeout count={} owner={:?}",
                timeout_count,
                crate::display::console::current_screen_owner()
            ));
            if input.is_empty() {
                if input_mode.prompt_warm {
                    shell_prime_input_path("shell-prompt-warm-rearm", false);
                } else if input_mode.empty_prompt_armed {
                    shell_prime_input_path("shell-empty-armed-rearm", false);
                } else {
                    shell_prime_input_path("shell-idle-rearm", false);
                }
            } else if timeout_count == 1 || timeout_count % SHELL_TIMEOUT_PRIME_INTERVAL == 0 {
                shell_prime_input_path("shell-read-timeout", false);
            }
            shell_debug_note_timeout(
                timeout_count,
                input.len(),
                timeout_count <= 2
                    || timeout_count % SHELL_TIMEOUT_PANEL_UPDATE_INTERVAL == 0,
            );
        }
    }
}

fn process_shell_byte(
    byte: u8,
    input: &mut String,
    truncated: &mut bool,
    history_index: &mut Option<usize>,
    history_draft: &mut String,
) -> bool {
    let was_empty = input.is_empty();
    let mut empty_transition_reason: Option<crate::interactive::ShellEmptyReason> = None;
    trace_shell_event(format_args!(
        "[SHELL] input byte=0x{:02X} '{}' len={} owner={:?}",
        byte,
        printable(byte),
        input.len(),
        crate::display::console::current_screen_owner()
    ));
    shell_debug_note_recv(byte, input.len());

    match byte {
        b'\n' | b'\r' => {
            crate::display::console::set_input_cursor_enabled(false);
            kprintln!();
            let submitted = shorten_for_panel(input);
            trace_shell_event(format_args!(
                "[SHELL] submit len={} cmd='{}'",
                input.len(),
                input
            ));
            self::history::push(input);
            execute_command(input);
            if *truncated {
                kprint_colored!(Colors::YELLOW, "[WARN]");
                kprintln!(" input truncated at {} characters.", INPUT_LIMIT);
            }
            input.clear();
            *truncated = false;
            *history_index = None;
            history_draft.clear();
            if !session::is_logged_in() {
                return false;
            }
            prompt();
            crate::display::console::set_input_cursor_enabled(true);
            crate::display::console::force_input_cursor_visible();
            crate::interactive::note_shell_reprompt();
            shell_prime_input_path("shell-reprompt-warm", false);
            shell_debug_note_submit(&submitted, 0, "reprompt");
            shell_debug_note_reprompt();
            trace_shell_event(format_args!("[SHELL] reprompt"));
        }
        0x08 => {
            if !input.is_empty() {
                input.pop();
                crate::display::console::backspace();
                crate::display::console::force_input_cursor_visible();
                shell_debug_note_append("backspace", input.len(), Some("backspace"), "line-edit");
                trace_shell_event(format_args!("[SHELL] backspace new_len={}", input.len()));
                if input.is_empty() {
                    empty_transition_reason = Some(crate::interactive::ShellEmptyReason::Backspace);
                }
            } else {
                shell_debug_note_append("backspace-empty", 0, None, "idle");
            }
            *truncated = false;
            *history_index = None;
        }
        0x1B => {
            replace_visible_input(input, "");
            *truncated = false;
            *history_index = None;
            history_draft.clear();
            crate::display::console::force_input_cursor_visible();
            shell_debug_note_append("clear-line", 0, Some("clear-line"), "line-edit");
            empty_transition_reason = Some(crate::interactive::ShellEmptyReason::EscClear);
        }
        hal::input::KEY_ARROW_UP => {
            let history = self::history::snapshot();
            if history.is_empty() {
                shell_debug_note_append("history-up-empty", input.len(), None, "idle");
                crate::interactive::note_shell_input_activity(!input.is_empty());
                return true;
            }

            let next_index = match *history_index {
                Some(index) if index > 0 => index - 1,
                Some(index) => index,
                None => {
                    *history_draft = input.clone();
                    history.len().saturating_sub(1)
                }
            };
            *history_index = Some(next_index);
            if let Some(entry) = history.get(next_index) {
                replace_visible_input(input, entry);
                *truncated = false;
                crate::display::console::force_input_cursor_visible();
                shell_debug_note_append("history-up", input.len(), Some("replace-line"), "line-edit");
                if input.is_empty() {
                    empty_transition_reason =
                        Some(crate::interactive::ShellEmptyReason::HistoryEmpty);
                }
            }
        }
        hal::input::KEY_ARROW_DOWN => {
            let history = self::history::snapshot();
            let Some(index) = *history_index else {
                shell_debug_note_append("history-down-empty", input.len(), None, "idle");
                crate::interactive::note_shell_input_activity(!input.is_empty());
                return true;
            };

            if index + 1 < history.len() {
                let next_index = index + 1;
                *history_index = Some(next_index);
                if let Some(entry) = history.get(next_index) {
                    replace_visible_input(input, entry);
                }
            } else {
                *history_index = None;
                replace_visible_input(input, history_draft);
            }
            *truncated = false;
            crate::display::console::force_input_cursor_visible();
            shell_debug_note_append("history-down", input.len(), Some("replace-line"), "line-edit");
            if input.is_empty() {
                empty_transition_reason = Some(crate::interactive::ShellEmptyReason::HistoryEmpty);
            }
        }
        byte if byte.is_ascii_graphic() || byte == b' ' => {
            if input.len() < INPUT_LIMIT {
                input.push(char::from(byte));
                kprint!("{}", char::from(byte));
                crate::display::console::force_input_cursor_visible();
                shell_debug_note_append("yes", input.len(), Some("yes"), "echo");
                trace_shell_event(format_args!(
                    "[SHELL] echo '{}' len={} buffer='{}'",
                    printable(byte),
                    input.len(),
                    input
                ));
            } else {
                *truncated = true;
                shell_debug_note_append("truncated", input.len(), None, "idle");
                trace_shell_event(format_args!(
                    "[SHELL] truncated input_limit={} dropped=0x{:02X}",
                    INPUT_LIMIT,
                    byte
                ));
            }
            *history_index = None;
        }
        _ => {
            shell_debug_note_append("filtered", input.len(), None, "idle");
        }
    }

    if input.is_empty() {
        if let Some(reason) = empty_transition_reason {
            crate::interactive::note_shell_empty_prompt_armed(reason);
        } else if !was_empty {
            crate::interactive::note_shell_empty_prompt_armed(
                crate::interactive::ShellEmptyReason::Other,
            );
        } else {
            crate::interactive::note_shell_input_activity(false);
        }
    } else {
        crate::interactive::note_shell_input_activity(true);
    }
    true
}

fn trace_shell_event(args: core::fmt::Arguments<'_>) {
    if !SHELL_TRACE_ENABLED {
        return;
    }
    let remaining = SHELL_TRACE_BUDGET.load(Ordering::Relaxed);
    if remaining == 0 {
        return;
    }
    if SHELL_TRACE_BUDGET
        .compare_exchange(remaining, remaining - 1, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
    {
        crate::serial_println!("{}", args);
    }
}

fn printable(byte: u8) -> char {
    if byte.is_ascii_graphic() || byte == b' ' {
        char::from(byte)
    } else {
        '.'
    }
}

fn replace_visible_input(current: &mut String, replacement: &str) {
    while !current.is_empty() {
        current.pop();
        crate::display::console::backspace();
    }

    for character in replacement.chars().take(INPUT_LIMIT) {
        current.push(character);
        kprint!("{}", character);
    }
}

fn shell_read_timeout_ticks(line_active: bool, prompt_warm: bool, empty_prompt_armed: bool) -> u64 {
    if line_active {
        SHELL_ACTIVE_READ_TIMEOUT_TICKS
    } else if prompt_warm || empty_prompt_armed {
        SHELL_ACTIVE_READ_TIMEOUT_TICKS
    } else {
        SHELL_IDLE_READ_TIMEOUT_TICKS
    }
}

fn shell_reentry_origin_label() -> &'static str {
    let input_mode = crate::interactive::shell_input_mode_snapshot();
    if input_mode.prompt_warm {
        "prompt-warm"
    } else if input_mode.edited_empty_warm {
        "edited-empty"
    } else if input_mode.empty_prompt_armed {
        "empty-armed"
    } else {
        "cold-idle"
    }
}

fn shell_debug_note_recv(byte: u8, len: usize) {
    let tick = crate::arch::x86_64::interrupts::tick_count();
    shell_debug_update(|state| {
        state.current_loop_state = String::from("recv");
        state.timeout_count = 0;
        state.last_recv = format!("'{}' (0x{:02X})", printable(byte), byte);
        state.last_recv_tick = tick;
        state.len = len;
    });
}

fn shell_debug_note_reentry(byte: u8, timeout_count: u32, origin: &str) {
    let tick = crate::arch::x86_64::interrupts::tick_count();
    shell_debug_update(|state| {
        state.last_reentry = format!("'{}' after {}#{}", printable(byte), origin, timeout_count);
        state.last_reentry_tick = tick;
    });
}

fn shell_debug_note_append(append: &str, len: usize, echo: Option<&str>, loop_state: &str) {
    let tick = crate::arch::x86_64::interrupts::tick_count();
    shell_debug_update(|state| {
        state.current_loop_state = String::from(loop_state);
        state.last_append = String::from(append);
        state.last_append_tick = tick;
        state.len = len;
        if let Some(result) = echo {
            state.last_echo = String::from(result);
            state.last_echo_tick = tick;
        }
    });
}

fn shell_debug_note_submit(submit: &str, len: usize, loop_state: &str) {
    let tick = crate::arch::x86_64::interrupts::tick_count();
    shell_debug_update(|state| {
        state.current_loop_state = String::from(loop_state);
        state.timeout_count = 0;
        state.last_submit = String::from(submit);
        state.last_submit_tick = tick;
        state.last_append = String::from("submit");
        state.last_append_tick = tick;
        state.last_echo = String::from("submit");
        state.last_echo_tick = tick;
        state.len = len;
    });
}

fn shell_debug_note_reprompt() {
    let tick = crate::arch::x86_64::interrupts::tick_count();
    shell_debug_update(|state| {
        state.last_reprompt_tick = tick;
        state.current_loop_state = String::from("reprompt");
    });
}

fn shell_debug_note_timeout(timeout_count: u32, len: usize, render: bool) {
    shell_debug_update_with_render(render, |state| {
        state.current_loop_state = format!("timeout #{}", timeout_count);
        state.timeout_count = timeout_count;
        state.len = len;
    });
}

fn shell_debug_reset() {
    if !SHELL_DEBUG_PANEL_ENABLED {
        return;
    }
    {
        let mut state = SHELL_DEBUG_STATE.lock();
        *state = ShellDebugState {
            current_loop_state: String::from("startup"),
            timeout_count: 0,
            last_recv: String::from("-"),
            last_recv_tick: 0,
            last_reentry: String::from("-"),
            last_reentry_tick: 0,
            last_append: String::from("-"),
            last_append_tick: 0,
            last_echo: String::from("-"),
            last_echo_tick: 0,
            last_submit: String::from("-"),
            last_submit_tick: 0,
            last_reprompt_tick: 0,
            last_empty_tick: 0,
            last_empty_reason: String::from("none"),
            last_prime: String::from("-"),
            last_prime_tick: 0,
            len: 0,
            mode: String::from("-"),
            warm: String::from("-"),
            cursor: String::from("-"),
            owner: String::from("-"),
            clobber: String::from("no"),
        };
        refresh_shell_debug_runtime(&mut state);
    }
    render_shell_debug_panel();
}

fn shell_debug_update(update: impl FnOnce(&mut ShellDebugState)) {
    shell_debug_update_with_render(true, update);
}

fn shell_debug_update_with_render(render: bool, update: impl FnOnce(&mut ShellDebugState)) {
    if !SHELL_DEBUG_PANEL_ENABLED {
        return;
    }
    {
        let mut state = SHELL_DEBUG_STATE.lock();
        update(&mut state);
        refresh_shell_debug_runtime(&mut state);
    }
    if render {
        render_shell_debug_panel();
    }
}

fn refresh_shell_debug_runtime(state: &mut ShellDebugState) {
    let cursor = console::input_cursor_snapshot();
    state.cursor = if cursor.enabled && cursor.visible {
        format!("shown r{} c{}", cursor.row, cursor.col)
    } else if cursor.enabled {
        format!("armed r{} c{}", cursor.row, cursor.col)
    } else {
        format!("off r{} c{}", cursor.row, cursor.col)
    };
    let input_mode = crate::interactive::shell_input_mode_snapshot();
    state.mode = if input_mode.line_active {
        String::from("line")
    } else if input_mode.edited_empty_warm {
        String::from("empty-edit-warm")
    } else if input_mode.prompt_warm {
        String::from("prompt-warm")
    } else if input_mode.empty_prompt_armed {
        String::from("empty-armed")
    } else if input_mode.fast_mode {
        String::from("fast")
    } else {
        String::from("idle-cold")
    };
    state.warm = if input_mode.prompt_warm {
        format!(
            "prompt=yes fast={} reprompt={}",
            if input_mode.fast_mode { "yes" } else { "no" },
            input_mode.since_reprompt_ticks
        )
    } else if input_mode.edited_empty_warm {
        format!(
            "prompt=no edited=yes empty={}",
            input_mode.since_empty_prompt_ticks
        )
    } else if input_mode.empty_prompt_armed {
        format!("prompt=no armed=yes empty={}", input_mode.since_empty_prompt_ticks)
    } else {
        format!(
            "prompt=no armed=no idle={} rearm={}@{}",
            input_mode.since_input_ticks,
            input_mode.idle_rearm_count,
            input_mode.since_idle_rearm_ticks
        )
    };
    state.last_empty_tick = input_mode.last_empty_prompt_tick;
    state.last_empty_reason = String::from(input_mode.last_empty_reason.label());
    let owner = console::current_screen_owner();
    state.owner = String::from(owner.label());
    let clobber = console::shell_clobber_snapshot();
    state.clobber = if clobber.count == 0 {
        String::from("no")
    } else {
        format!(
            "{}:{} #{}",
            clobber.kind.label(),
            clobber.owner.label(),
            clobber.count
        )
    };
}

fn render_shell_debug_panel() {
    if !SHELL_DEBUG_PANEL_ENABLED || !console::screen_owner_is(console::ScreenOwner::Shell) {
        return;
    }
    let state = SHELL_DEBUG_STATE.lock().clone();
    let _ = console::with_raw_console(|console| render_shell_debug_panel_raw(console, &state));
}

fn render_shell_debug_panel_raw(console: &mut console::FramebufferConsole, state: &ShellDebugState) {
    const SCALE: usize = 1;
    const PANEL_WIDTH: usize = 40 * font::FONT_WIDTH + 16;
    const TITLE_HEIGHT: usize = font::FONT_HEIGHT_PIXELS + 8;
    const LINE_GAP: usize = font::FONT_HEIGHT_PIXELS + 2;
    const PANEL_PADDING: usize = 8;
    const PANEL_MARGIN: usize = 12;
    const PANEL_BG: u32 = 0x00101823;
    const PANEL_BORDER: u32 = 0x002A3A4A;
    const PANEL_TITLE: u32 = 0x0056D4DD;
    const PANEL_TEXT: u32 = 0x00E6EDF3;
    const PANEL_ACCENT: u32 = 0x003FB950;

    let now_tick = crate::arch::x86_64::interrupts::tick_count();
    let lines = [
        format!("state={}", state.current_loop_state),
        format!(
            "recv={} @{}",
            state.last_recv,
            format_tick(state.last_recv_tick)
        ),
        format!(
            "reentry={} @{}",
            state.last_reentry,
            format_tick(state.last_reentry_tick)
        ),
        format!(
            "append={} @{}",
            state.last_append,
            format_tick(state.last_append_tick)
        ),
        format!(
            "echo={} @{}",
            state.last_echo,
            format_tick(state.last_echo_tick)
        ),
        format!(
            "submit={} @{}",
            state.last_submit,
            format_tick(state.last_submit_tick)
        ),
        format!(
            "reprompt=@{} since={}",
            format_tick(state.last_reprompt_tick),
            age_since(now_tick, state.last_reprompt_tick)
        ),
        format!(
            "empty=@{} since={}",
            format_tick(state.last_empty_tick),
            age_since(now_tick, state.last_empty_tick)
        ),
        format!("empty_reason={}", state.last_empty_reason),
        format!(
            "prime={} @{} since={}",
            state.last_prime,
            format_tick(state.last_prime_tick),
            age_since(now_tick, state.last_prime_tick)
        ),
        format!(
            "len={} since_recv={}",
            state.len,
            age_since(now_tick, state.last_recv_tick)
        ),
        format!(
            "since_echo={} timeouts={}",
            age_since(now_tick, state.last_echo_tick),
            state.timeout_count
        ),
        format!("mode={}", state.mode),
        format!("warm={}", state.warm),
        format!("cursor={}", state.cursor),
        format!("owner={}", state.owner),
        format!("clobber={}", state.clobber),
    ];
    let panel_height =
        TITLE_HEIGHT + PANEL_PADDING + lines.len().saturating_mul(LINE_GAP) + PANEL_PADDING;
    let start_x = console
        .width_pixels()
        .saturating_sub(PANEL_WIDTH.saturating_add(PANEL_MARGIN));
    let start_y = PANEL_MARGIN;

    console.raw_begin_batch();
    fill_rect(console, start_x, start_y, PANEL_WIDTH, panel_height, PANEL_BG);
    fill_rect(console, start_x, start_y, PANEL_WIDTH, 2, PANEL_ACCENT);
    fill_rect(console, start_x, start_y, 2, panel_height, PANEL_BORDER);
    fill_rect(
        console,
        start_x.saturating_add(PANEL_WIDTH.saturating_sub(2)),
        start_y,
        2,
        panel_height,
        PANEL_BORDER,
    );
    fill_rect(
        console,
        start_x,
        start_y.saturating_add(panel_height.saturating_sub(2)),
        PANEL_WIDTH,
        2,
        PANEL_BORDER,
    );
    draw_text(
        console,
        start_x + PANEL_PADDING,
        start_y + 4,
        "SHELL DEBUG",
        SCALE,
        PANEL_TITLE,
    );

    for (index, line) in lines.iter().enumerate() {
        let y = start_y + TITLE_HEIGHT + index.saturating_mul(LINE_GAP);
        draw_text(
            console,
            start_x + PANEL_PADDING,
            y,
            line,
            SCALE,
            PANEL_TEXT,
        );
    }
    console.raw_end_batch();
}

fn fill_rect(
    console: &mut console::FramebufferConsole,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    color: u32,
) {
    let max_x = x.saturating_add(width).min(console.width_pixels());
    let max_y = y.saturating_add(height).min(console.height_pixels());
    for py in y..max_y {
        for px in x..max_x {
            console.write_pixel(px, py, color);
        }
    }
}

fn draw_text(
    console: &mut console::FramebufferConsole,
    mut x: usize,
    y: usize,
    text: &str,
    scale: usize,
    color: u32,
) {
    for byte in text.bytes().take(40) {
        draw_char(console, x, y, byte, scale, color);
        x = x.saturating_add(font::FONT_WIDTH.saturating_mul(scale));
    }
}

fn draw_char(
    console: &mut console::FramebufferConsole,
    x: usize,
    y: usize,
    byte: u8,
    scale: usize,
    color: u32,
) {
    let glyph = font::glyph(char::from(byte));
    for (row_index, row) in glyph.raster().iter().enumerate() {
        for (column_index, intensity) in row.iter().copied().enumerate() {
            if intensity == 0 {
                continue;
            }
            let pixel_x = x.saturating_add(column_index.saturating_mul(scale));
            let pixel_y = y.saturating_add(row_index.saturating_mul(scale));
            for dy in 0..scale {
                for dx in 0..scale {
                    console.write_pixel(
                        pixel_x.saturating_add(dx),
                        pixel_y.saturating_add(dy),
                        color,
                    );
                }
            }
        }
    }
}

fn shell_prime_input_path(context: &str, reflect_in_panel: bool) {
    let tick = crate::arch::x86_64::interrupts::tick_count();
    if !x86_64::instructions::interrupts::are_enabled() {
        trace_shell_event(format_args!(
            "[SHELL] prime context={} interrupts=off->on",
            context
        ));
        x86_64::instructions::interrupts::enable();
    }
    let _ = crate::hal::input::input_poll();
    trace_shell_event(format_args!(
        "[SHELL] prime context={} queue_ready={}",
        context,
        crate::hal::input::has_pending_input()
    ));
    shell_debug_update_with_render(reflect_in_panel, |state| {
        state.last_prime = String::from(context);
        state.last_prime_tick = tick;
        if reflect_in_panel {
            state.current_loop_state = format!("prime:{}", context);
        }
    });
}

fn age_since(now_tick: u64, tick: u64) -> String {
    if tick == 0 {
        String::from("-")
    } else {
        format!("{}", now_tick.saturating_sub(tick))
    }
}

fn format_tick(tick: u64) -> String {
    if tick == 0 {
        String::from("-")
    } else {
        format!("{}", tick)
    }
}

fn shorten_for_panel(text: &str) -> String {
    let mut shortened = String::new();
    for character in text.chars().take(18) {
        shortened.push(character);
    }
    if text.chars().count() > 18 {
        shortened.push_str("...");
    }
    if shortened.is_empty() {
        String::from("<empty>")
    } else {
        shortened
    }
}
