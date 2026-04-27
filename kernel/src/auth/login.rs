use alloc::string::String;
use core::sync::atomic::{AtomicBool, Ordering};

use spin::{Lazy, Mutex};

use crate::auth::{AuthError, UserAccount, UserRole, USER_DB};
use crate::display::console::{self, Colors};
use crate::hal;
use crate::hal::device::KeyboardLayout;
use crate::ui::{self, Language, TextId};
use crate::{kprint, kprint_colored, kprintln, KERNEL_VERSION};

const AUTH_INPUT_LIMIT: usize = 64;
const AUTH_READ_TIMEOUT_TICKS: u64 = 100;
const AUTH_TRACE_VERBOSE: bool = false;
const AUTH_HANDOFF_TRACE_ENABLED: bool = false;
const AUTH_SHOW_INPUT_DIAGNOSTIC: bool = false;
const ONBOARDING_CONFIRM_TRACE_ENABLED: bool = true;
static PRE_AUTH_INPUT_BOOTSTRAP_DONE: AtomicBool = AtomicBool::new(false);
static AUTH_INPUT_PATH_LOGGED: AtomicBool = AtomicBool::new(false);
static ONBOARDING_DEBUG_OVERLAY: Lazy<Mutex<OnboardingDebugOverlay>> =
    Lazy::new(|| Mutex::new(OnboardingDebugOverlay::default()));

macro_rules! auth_trace {
    ($($arg:tt)*) => {
        if AUTH_TRACE_VERBOSE {
            crate::serial_println!($($arg)*);
        }
    };
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OnboardingFocus {
    Language,
    Layout,
    Continue,
}

impl OnboardingFocus {
    fn next(self) -> Self {
        match self {
            Self::Language => Self::Layout,
            Self::Layout => Self::Continue,
            Self::Continue => Self::Language,
        }
    }

    fn previous(self) -> Self {
        match self {
            Self::Language => Self::Continue,
            Self::Layout => Self::Language,
            Self::Continue => Self::Layout,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Language => "Language",
            Self::Layout => "Keyboard",
            Self::Continue => "Continue",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CredentialFocus {
    Username,
    Password,
    Confirm,
}

enum LineReadOutcome {
    Submitted(String),
    OpenPreferences,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OnboardingAction {
    MovePrevious,
    MoveNext,
    FocusPrevious,
    FocusNext,
    Confirm,
    Back,
    Ignore,
}

enum PreferencesExit {
    Continue {
        handoff_status: Option<String>,
    },
    Back,
}

#[derive(Clone, Default)]
struct OnboardingDebugOverlay {
    key: String,
    ps2: String,
    validate: String,
    commit: String,
    exit: String,
    next: String,
}

fn onboarding_trace(args: core::fmt::Arguments<'_>) {
    if ONBOARDING_CONFIRM_TRACE_ENABLED {
        crate::serial_println!("{}", args);
    }
}

fn onboarding_overlay_reset() {
    if !ONBOARDING_CONFIRM_TRACE_ENABLED {
        return;
    }
    let mut overlay = ONBOARDING_DEBUG_OVERLAY.lock();
    *overlay = OnboardingDebugOverlay::default();
}

fn onboarding_overlay_update(
    update: impl FnOnce(&mut OnboardingDebugOverlay),
) {
    if !ONBOARDING_CONFIRM_TRACE_ENABLED {
        return;
    }
    update(&mut ONBOARDING_DEBUG_OVERLAY.lock());
}

fn onboarding_overlay_snapshot() -> OnboardingDebugOverlay {
    ONBOARDING_DEBUG_OVERLAY.lock().clone()
}

pub fn first_boot_setup() -> UserAccount {
    crate::serial_println!("[TRACE] F auth entry");
    trace_handoff("setup", "preferences begin");
    let mut status = match pre_auth_preferences(true) {
        PreferencesExit::Continue { handoff_status } => {
            onboarding_overlay_update(|overlay| {
                overlay.next = alloc::format!(
                    "next=first-boot-credentials status={}",
                    handoff_status.as_deref().unwrap_or("<none>")
                );
            });
            onboarding_trace(format_args!(
                "[LOGIN] handoff=first-boot-setup entered-next=credentials status={}",
                handoff_status.as_deref().unwrap_or("<none>")
            ));
            handoff_status.unwrap_or_default()
        }
        PreferencesExit::Back => {
            onboarding_overlay_update(|overlay| {
                overlay.next = String::from("next=first-boot-back unexpected");
            });
            onboarding_trace(format_args!(
                "[LOGIN] handoff=first-boot-setup exit=back unexpected=true"
            ));
            String::new()
        }
    };
    trace_handoff("setup", "preferences returned");
    trace_handoff("setup", "credentials begin");
    auth_trace!("[INTERACTIVE] entering loop stage=first-boot-credentials");

    loop {
        auth_trace!("[INTERACTIVE] tick stage=first-boot-credentials");
        render_credential_screen(
            ui::text(TextId::SetupTitle),
            ui::text(TextId::SetupSubtitle),
            CredentialFocus::Username,
            None,
            status.as_str(),
            true,
        );

        kprint_colored!(Colors::DIM, "  {}: ", ui::text(TextId::UsernameLabel));
        let username = String::from(read_line_echo().trim());
        if username.is_empty() {
            continue;
        }
        if !is_valid_username(&username) {
            status = String::from("Username must be 3-32 chars: [a-zA-Z0-9_.-].");
            continue;
        }

        render_credential_screen(
            ui::text(TextId::SetupTitle),
            ui::text(TextId::SetupSubtitle),
            CredentialFocus::Password,
            Some(username.as_str()),
            "",
            true,
        );
        kprint_colored!(Colors::DIM, "  {}: ", ui::text(TextId::PasswordLabel));
        let password = read_line_hidden();
        kprintln!();
        render_credential_screen(
            ui::text(TextId::SetupTitle),
            ui::text(TextId::SetupSubtitle),
            CredentialFocus::Confirm,
            Some(username.as_str()),
            "",
            true,
        );
        kprint_colored!(Colors::DIM, "  {}:  ", ui::text(TextId::ConfirmLabel));
        let confirm = read_line_hidden();
        kprintln!();

        if !is_strong_enough_password(&password) {
            status = String::from("Password must have at least 8 visible characters.");
            continue;
        }
        if password != confirm {
            status = String::from(ui::text(TextId::PasswordMismatch));
            continue;
        }

        let mut db = USER_DB.lock();
        match db.create_user(&username, &password, UserRole::Admin) {
            Ok(uid) => match db.try_save_to_fs() {
                Ok(()) => {
                    let user = db
                        .find_by_uid(uid)
                        .cloned()
                        .expect("newly created admin missing");
                    drop(db);
                    crate::auth::clear_first_boot_pending();
                    crate::serial_println!(
                            "[TRACE] auth: first-boot account persisted user={} uid={} first_boot_pending=cleared",
                            user.username,
                            user.uid
                        );
                    kprintln!();
                    kprint_colored!(Colors::GREEN, "  ");
                    kprintln!("{}", ui::text(TextId::AccountCreated));
                    kprint_colored!(Colors::DIM, "  ");
                    kprintln!("{}", ui::text(TextId::SessionPreparing));
                    wait_ticks(80);
                    crate::serial_println!(
                        "[TRACE] auth: first-boot setup returning user={} uid={}",
                        user.username,
                        user.uid
                    );
                    return user;
                }
                Err(error) => {
                    let _ = db.delete_user(uid);
                    drop(db);
                    status = alloc::format!(
                        "{}: {}.",
                        ui::text(TextId::AccountCreateFailed),
                        error
                    );
                }
            },
            Err(error) => {
                drop(db);
                status = alloc::format!(
                    "{}: {}.",
                    ui::text(TextId::AccountCreateFailed),
                    error
                );
            }
        }
    }
}

pub fn login_screen() -> UserAccount {
    trace_handoff("login", "credentials begin");
    auth_trace!("[INTERACTIVE] entering loop stage=login-credentials");
    let mut failed_attempts = 0u8;
    let mut status = String::new();

    loop {
        auth_trace!("[INTERACTIVE] tick stage=login-credentials");
        render_credential_screen(
            ui::text(TextId::LoginTitle),
            ui::text(TextId::LoginSubtitle),
            CredentialFocus::Username,
            None,
            status.as_str(),
            false,
        );

        kprint_colored!(Colors::DIM, "  {}: ", ui::text(TextId::UsernameLabel));
        let username = match read_line_echo_or_preferences() {
            LineReadOutcome::Submitted(value) => String::from(value.trim()),
            LineReadOutcome::OpenPreferences => {
                trace_handoff("login", "preferences begin");
                let handoff_status = match pre_auth_preferences(false) {
                    PreferencesExit::Continue { handoff_status } => {
                        onboarding_overlay_update(|overlay| {
                            overlay.next = alloc::format!(
                                "next=login-credentials status={}",
                                handoff_status.as_deref().unwrap_or("<none>")
                            );
                        });
                        onboarding_trace(format_args!(
                            "[LOGIN] handoff=login entered-next=login-credentials status={}",
                            handoff_status.as_deref().unwrap_or("<none>")
                        ));
                        handoff_status
                    }
                    PreferencesExit::Back => {
                        onboarding_overlay_update(|overlay| {
                            overlay.next = String::from("next=login-back unchanged");
                        });
                        onboarding_trace(format_args!(
                            "[LOGIN] handoff=login exit=back status=unchanged"
                        ));
                        status = String::from("Access preferences unchanged.");
                        continue;
                    }
                };
                trace_handoff("login", "preferences returned");
                status = handoff_status
                    .unwrap_or_else(|| String::from("Access preferences updated."));
                continue;
            }
        };
        if username.is_empty() {
            continue;
        }
        if !is_valid_username(&username) {
            status = String::from("Invalid username format.");
            continue;
        }

        render_credential_screen(
            ui::text(TextId::LoginTitle),
            ui::text(TextId::LoginSubtitle),
            CredentialFocus::Password,
            Some(username.as_str()),
            "",
            false,
        );
        kprint_colored!(Colors::DIM, "  {}: ", ui::text(TextId::PasswordLabel));
        let password = read_line_hidden();
        kprintln!();

        let mut db = USER_DB.lock();
        match db.authenticate(&username, &password) {
            Ok(user) => {
                let previous_login = db.record_login(user.uid).unwrap_or(0);
                if let Err(error) = db.try_save_to_fs() {
                    crate::serial_println!(
                        "[WARN] auth: failed to persist login metadata for {}: {}",
                        user.username,
                        error
                    );
                }
                drop(db);

                crate::security::audit::log_event(
                    crate::security::audit::events::AuditEvent::LoginSuccess {
                        username: user.username.clone(),
                        uid: user.uid,
                    },
                );

                kprintln!();
                kprint_colored!(Colors::GREEN, "  {} ", ui::text(TextId::WelcomeBack));
                kprintln!("{}.", user.username);
                if previous_login != 0 {
                    kprint_colored!(Colors::DIM, "  {}: ", ui::text(TextId::LastLogin));
                    kprintln!("{}", ui::text(TextId::LastLoginRecorded));
                }
                kprintln!();
                return user;
            }
            Err(error) => {
                drop(db);

                let reason = match error {
                    AuthError::UserNotFound => "user not found",
                    AuthError::WrongPassword => "wrong password",
                    AuthError::AccountDisabled => "account disabled",
                    _ => "authentication failed",
                };
                crate::security::audit::log_event(
                    crate::security::audit::events::AuditEvent::LoginFailed {
                        username: username.clone(),
                        reason: alloc::string::String::from(reason),
                    },
                );

                failed_attempts = failed_attempts.saturating_add(1);
                status = match error {
                    AuthError::UserNotFound => String::from(ui::text(TextId::UserNotFound)),
                    AuthError::WrongPassword => String::from(ui::text(TextId::WrongPassword)),
                    AuthError::AccountDisabled => String::from(ui::text(TextId::AccountDisabled)),
                    _ => String::from(ui::text(TextId::AuthFailed)),
                };

                if failed_attempts >= 3 {
                    status = String::from(ui::text(TextId::TooManyAttempts));
                    wait_ticks(3_000);
                    failed_attempts = 0;
                }
            }
        }
    }
}

pub fn read_line_echo() -> String {
    match read_line(false, false) {
        LineReadOutcome::Submitted(value) => value,
        LineReadOutcome::OpenPreferences => String::new(),
    }
}

pub fn read_line_hidden() -> String {
    match read_line(true, false) {
        LineReadOutcome::Submitted(value) => value,
        LineReadOutcome::OpenPreferences => String::new(),
    }
}

fn read_line_echo_or_preferences() -> LineReadOutcome {
    read_line(false, true)
}

fn read_line(hidden: bool, allow_preferences_shortcut: bool) -> LineReadOutcome {
    let mut input = String::new();
    let stage = if hidden {
        crate::interactive::InteractiveStage::AuthReadLineHidden
    } else {
        crate::interactive::InteractiveStage::AuthReadLineEcho
    };
    if !AUTH_INPUT_PATH_LOGGED.swap(true, Ordering::Relaxed) {
        crate::serial_println!(
            "[INPUT_COMPARE] auth_api=interactive::read_input(timeout) stage={}",
            stage.label()
        );
    }
    crate::interactive::enter(stage, if hidden { "password" } else { "text" });
    console::set_input_cursor_enabled(true);
    console::force_input_cursor_visible();
    auth_trace!(
        "[INTERACTIVE] entering loop stage={} consumer=auth-line",
        stage.label()
    );
    prime_input_path(if hidden {
        "auth-read-line-hidden-start"
    } else {
        "auth-read-line-echo-start"
    });
    let mut timeout_count = 0u32;
    loop {
        if let Some(byte) =
            crate::interactive::read_input(stage, "auth-line", Some(AUTH_READ_TIMEOUT_TICKS))
        {
            timeout_count = 0;
            auth_trace!(
                "[INTERACTIVE] processing input stage={} consumer=auth-line byte=0x{:02X}",
                stage.label(),
                byte
            );
            trace_ui_byte("auth-line", byte, "delivered");
            match byte {
                b'\t' | 0x1B if !hidden && input.is_empty() && allow_preferences_shortcut => {
                    console::set_input_cursor_enabled(false);
                    return LineReadOutcome::OpenPreferences;
                }
                b'\n' | b'\r' => {
                    if input.is_empty() {
                        trace_ui_byte("auth-line", byte, "ignored-empty-enter");
                        continue;
                    }
                    trace_ui_byte("auth-line", byte, "consumed-enter");
                    console::set_input_cursor_enabled(false);
                    return LineReadOutcome::Submitted(input);
                }
                0x08 => {
                    trace_ui_byte("auth-line", byte, "consumed-backspace");
                    if !input.is_empty() {
                        input.pop();
                        console::backspace();
                        console::force_input_cursor_visible();
                    }
                }
                byte if byte.is_ascii_graphic() || byte == b' ' => {
                    trace_ui_byte("auth-line", byte, "consumed-visible");
                    if input.len() < AUTH_INPUT_LIMIT {
                        input.push(byte as char);
                        if hidden {
                            kprint!("*");
                        } else {
                            kprint!("{}", byte as char);
                        }
                        console::force_input_cursor_visible();
                    }
                }
                _ => {
                    trace_ui_byte("auth-line", byte, "discarded");
                }
            }
        } else {
            auth_trace!(
                "[INTERACTIVE] waiting input stage={} consumer=auth-line timeout={}",
                stage.label(),
                AUTH_READ_TIMEOUT_TICKS
            );
            timeout_count = timeout_count.saturating_add(1);
            if timeout_count == 1 || timeout_count % 2 == 0 {
                prime_input_path("auth-line-timeout-recovery");
            }
            crate::interactive::idle_wait(stage, "auth-line-timeout");
        }
    }
}

fn wait_ticks(ticks: u64) {
    if ticks == 0 {
        return;
    }

    let mut remaining = ticks;
    let mut pit_fallback_logged = false;
    while remaining > 0 {
        let start_tick = crate::arch::x86_64::interrupts::tick_count();
        let advanced = if crate::arch::x86_64::pit::wait_for_tick_advance(start_tick, 1) {
            crate::arch::x86_64::interrupts::tick_count()
                .saturating_sub(start_tick)
                .max(1)
        } else {
            if !pit_fallback_logged {
                crate::serial_println!(
                    "[TRACE] auth wait: timer IRQ stalled during {}-tick delay; using PIT-only progress",
                    ticks
                );
                pit_fallback_logged = true;
            }
            1
        };
        remaining = remaining.saturating_sub(advanced);
        crate::interactive::service(
            crate::interactive::InteractiveStage::AuthPreferences,
            "wait-ticks",
        );
    }
}

pub fn render_session_ready(user: &UserAccount, first_boot: bool) {
    let active_owner = console::current_screen_owner();
    if matches!(
        active_owner,
        console::ScreenOwner::Shell | console::ScreenOwner::Gui
    ) {
        crate::serial_println!(
            "[SCREEN] auth-render blocked owner={:?} stage=session-ready",
            active_owner
        );
        return;
    }
    console::claim_screen_owner(console::ScreenOwner::Auth, "render-session-ready");
    let _ = console::clear_screen_for(console::ScreenOwner::Auth, "render-session-ready");
    kprintln!();
    kprint_colored!(
        Colors::CYAN,
        "============================================================\n"
    );
    kprint_colored!(
        Colors::GREEN,
        "  WarOS v{} - {}\n",
        KERNEL_VERSION,
        ui::text(TextId::SessionReadyTitle)
    );
    if first_boot {
        kprintln!("  {}", ui::text(TextId::SessionReadyFirstBoot));
    } else {
        kprintln!("  {}", ui::text(TextId::SessionReadyLogin));
    }
    kprint_colored!(
        Colors::DIM,
        "  {}: {}    {}: {}\n",
        ui::text(TextId::CurrentLanguage),
        ui::language().label(),
        ui::text(TextId::CurrentLayout),
        ui::layout_label(ui::keyboard_layout())
    );
    kprint_colored!(
        Colors::CYAN,
        "------------------------------------------------------------\n"
    );
    kprint_colored!(Colors::DIM, "  user: ");
    kprintln!("{}", user.username);
    kprint_colored!(Colors::DIM, "  ");
    kprintln!("{}", ui::text(TextId::SessionReadyHint));
    if let Some(note) = ui::localization_status_note() {
        kprint_colored!(Colors::YELLOW, "  note: ");
        kprintln!("{}", note);
    }
    kprintln!();
}

fn pre_auth_preferences(first_boot: bool) -> PreferencesExit {
    onboarding_overlay_reset();
    crate::interactive::enter(
        crate::interactive::InteractiveStage::AuthPreferences,
        if first_boot {
            "pre-auth-first-boot"
        } else {
            "pre-auth-login"
        },
    );
    let mut focus = OnboardingFocus::Language;
    let mut status = String::new();
    let mut dirty = false;
    let mut needs_render = true;

    loop {
        if needs_render {
            render_preferences_screen(first_boot, focus, status.as_str());
            if AUTH_SHOW_INPUT_DIAGNOSTIC {
                render_input_diagnostic();
            }
            needs_render = false;
        }
        prime_input_path("pre-auth-loop");

        match read_menu_key_timeout_interactive(AUTH_READ_TIMEOUT_TICKS) {
            Some(keypress) => {
                let action = onboarding_action(keypress);
                let ps2_snapshot = crate::drivers::keyboard::debug_snapshot();
                onboarding_overlay_update(|overlay| {
                    overlay.key = alloc::format!(
                        "key=0x{:02X} src={} action={} focus={}",
                        keypress.byte,
                        match keypress.source {
                            hal::input::INPUT_SOURCE_PS2 => "ps2",
                            hal::input::INPUT_SOURCE_USB => "usb",
                            _ => "none",
                        },
                        onboarding_action_label(action),
                        focus.label()
                    );
                    if keypress.source == hal::input::INPUT_SOURCE_PS2 {
                        overlay.ps2 = alloc::format!(
                            "raw=0x{:02X} pre_e0={} pre_f0={} active={} selected={} s1={} s2={} fb={} out=0x{:02X} dyn={} xlat={} from_fb={}",
                            ps2_snapshot.last_scancode,
                            if ps2_snapshot.ps2_prefix_e0_before { "1" } else { "0" },
                            if ps2_snapshot.ps2_prefix_f0_before { "1" } else { "0" },
                            ps2_set_label(ps2_snapshot.ps2_active_set),
                            ps2_set_label(ps2_snapshot.ps2_selected_set),
                            format_optional_byte(ps2_snapshot.ps2_set1_byte),
                            format_optional_byte(ps2_snapshot.ps2_set2_byte),
                            format_optional_byte(ps2_snapshot.ps2_fallback_byte),
                            ps2_snapshot.last_byte,
                            if ps2_snapshot.ps2_dynamic_switch {
                                "1"
                            } else {
                                "0"
                            },
                            if ps2_snapshot.ps2_translation_enabled {
                                "1"
                            } else {
                                "0"
                            },
                            if ps2_snapshot.ps2_emitted_from_fallback {
                                "1"
                            } else {
                                "0"
                            }
                        );
                    }
                });
                onboarding_trace(format_args!(
                    "[ONBOARDING] key=0x{:02X} source={} action={} focus={} lang={} layout={}",
                    keypress.byte,
                    match keypress.source {
                        hal::input::INPUT_SOURCE_PS2 => "ps2",
                        hal::input::INPUT_SOURCE_USB => "usb",
                        _ => "none",
                    },
                    onboarding_action_label(action),
                    focus.label(),
                    ui::language().label(),
                    ui::layout_label(ui::keyboard_layout()),
                ));
                match action {
                    OnboardingAction::MovePrevious => match focus {
                        OnboardingFocus::Language => {
                            let language = cycle_language(-1);
                            if ui::language() != language {
                                ui::set_language(language);
                                dirty = true;
                            }
                            status = alloc::format!("Language set to {}.", language.label());
                        }
                        OnboardingFocus::Layout => {
                            let layout = cycle_ready_layout(-1);
                            status = apply_layout_choice(layout, &mut dirty)
                                .unwrap_or_else(|error| alloc::format!("Layout change failed: {}", error));
                        }
                        OnboardingFocus::Continue => {
                            focus = OnboardingFocus::Layout;
                            status = String::from("Review the keyboard layout before continuing.");
                        }
                    },
                    OnboardingAction::MoveNext => match focus {
                        OnboardingFocus::Language => {
                            let language = cycle_language(1);
                            if ui::language() != language {
                                ui::set_language(language);
                                dirty = true;
                            }
                            status = alloc::format!("Language set to {}.", language.label());
                        }
                        OnboardingFocus::Layout => {
                            let layout = cycle_ready_layout(1);
                            status = apply_layout_choice(layout, &mut dirty)
                                .unwrap_or_else(|error| alloc::format!("Layout change failed: {}", error));
                        }
                        OnboardingFocus::Continue => {
                            focus = OnboardingFocus::Language;
                            status = String::from("Review the language before continuing.");
                        }
                    },
                    OnboardingAction::FocusPrevious => {
                        focus = focus.previous();
                        status = alloc::format!("Focus moved to {}.", focus.label());
                    }
                    OnboardingAction::FocusNext => {
                        focus = focus.next();
                        status = alloc::format!("Focus moved to {}.", focus.label());
                    }
                    OnboardingAction::Confirm => {
                        onboarding_trace(format_args!(
                            "[ONBOARDING] confirm focus={} continue_highlighted={} lang={} layout={} dirty={}",
                            focus.label(),
                            if matches!(focus, OnboardingFocus::Continue) {
                                "true"
                            } else {
                                "false"
                            },
                            ui::language().label(),
                            ui::layout_label(ui::keyboard_layout()),
                            if dirty { "true" } else { "false" }
                        ));
                        match commit_onboarding_preferences(dirty) {
                            Ok(handoff_status) => {
                                onboarding_overlay_update(|overlay| {
                                    overlay.exit = alloc::format!(
                                        "exit=Continue status={}",
                                        handoff_status.as_deref().unwrap_or("<none>")
                                    );
                                });
                                onboarding_trace(format_args!(
                                    "[ONBOARDING] exit=Continue status={}",
                                    handoff_status.as_deref().unwrap_or("<none>")
                                ));
                                return PreferencesExit::Continue { handoff_status };
                            }
                            Err(error) => {
                                onboarding_overlay_update(|overlay| {
                                    overlay.exit = alloc::format!("exit=Stay reason={error}");
                                });
                                onboarding_trace(format_args!(
                                    "[ONBOARDING] exit=Stay reason={}",
                                    error
                                ));
                                status = error;
                                if !matches!(focus, OnboardingFocus::Continue) {
                                    focus = OnboardingFocus::Continue;
                                }
                            }
                        }
                    }
                    OnboardingAction::Back => {
                        if first_boot {
                            status = String::from(
                                "First boot requires language and keyboard selection before continuing.",
                            );
                        } else {
                            return PreferencesExit::Back;
                        }
                    }
                    OnboardingAction::Ignore => {}
                }
                needs_render = true;
            }
            None => {
                pre_auth_input_bootstrap_once();
                prime_input_path("pre-auth-timeout");
                const WAITING_STATUS: &str = "Waiting for keyboard input...";
                if status.is_empty() || status.as_str() == WAITING_STATUS {
                    status.clear();
                    status.push_str(WAITING_STATUS);
                    needs_render = true;
                }
            }
        }
    }
}

fn commit_onboarding_preferences(dirty: bool) -> Result<Option<String>, String> {
    onboarding_overlay_update(|overlay| {
        overlay.commit = alloc::format!(
            "commit begin dirty={} lang={} layout={}",
            if dirty { "true" } else { "false" },
            ui::language().label(),
            ui::layout_label(ui::keyboard_layout())
        );
    });
    onboarding_trace(format_args!(
        "[ONBOARDING] commit begin dirty={} lang={} layout={}",
        if dirty { "true" } else { "false" },
        ui::language().label(),
        ui::layout_label(ui::keyboard_layout())
    ));
    validate_onboarding_selection()?;
    onboarding_overlay_update(|overlay| {
        overlay.validate = String::from("validate=ok");
    });
    onboarding_trace(format_args!("[ONBOARDING] validate=ok"));

    if !dirty {
        onboarding_overlay_update(|overlay| {
            overlay.commit = String::from("commit=ok persist=skipped");
        });
        onboarding_trace(format_args!(
            "[ONBOARDING] commit=ok persist=skipped reason=no-changes"
        ));
        return Ok(None);
    }

    match ui::save_preferences() {
        Ok(()) => {
            onboarding_overlay_update(|overlay| {
                overlay.commit = String::from("commit=ok persist=ok");
            });
            onboarding_trace(format_args!("[ONBOARDING] commit=ok persist=ok"));
            Ok(None)
        }
        Err(error) => {
            onboarding_overlay_update(|overlay| {
                overlay.commit = alloc::format!("commit=ok persist=warn {error}");
            });
            onboarding_trace(format_args!(
                "[ONBOARDING] commit=ok persist=warn error={}",
                error
            ));
            Ok(Some(alloc::format!(
                "Preferences applied for this session only; failed to save defaults: {}",
                error
            )))
        }
    }
}

fn validate_onboarding_selection() -> Result<(), String> {
    let current_layout = ui::keyboard_layout();
    let layout_ready = ui::keyboard_layout_choices()
        .iter()
        .any(|&(_, layout, ready)| layout == current_layout && ready);
    if !layout_ready {
        onboarding_overlay_update(|overlay| {
            overlay.validate = alloc::format!(
                "validate=fail layout={} not-ready",
                ui::layout_label(current_layout)
            );
        });
        onboarding_trace(format_args!(
            "[ONBOARDING] validate=fail lang={} layout={} reason=layout-not-ready",
            ui::language().label(),
            ui::layout_label(current_layout)
        ));
        return Err(alloc::format!(
            "Selected keyboard layout {} is staged. Choose a ready layout before continuing.",
            ui::layout_label(current_layout)
        ));
    }

    Ok(())
}

fn onboarding_action_label(action: OnboardingAction) -> &'static str {
    match action {
        OnboardingAction::MovePrevious => "MovePrevious",
        OnboardingAction::MoveNext => "MoveNext",
        OnboardingAction::FocusPrevious => "FocusPrevious",
        OnboardingAction::FocusNext => "FocusNext",
        OnboardingAction::Confirm => "Confirm",
        OnboardingAction::Back => "Back",
        OnboardingAction::Ignore => "Ignore",
    }
}

fn ps2_set_label(set: u8) -> &'static str {
    match set {
        1 => "set1",
        2 => "set2",
        _ => "none",
    }
}

fn format_optional_byte(value: Option<u8>) -> String {
    match value {
        Some(byte) => alloc::format!("0x{:02X}", byte),
        None => String::from("none"),
    }
}

fn pre_auth_input_bootstrap_once() {
    if PRE_AUTH_INPUT_BOOTSTRAP_DONE
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
        .is_err()
    {
        return;
    }

    verify_interrupt_state("pre-auth-bootstrap");
    let snapshot = hal::input::diagnostic_snapshot();

    // One-shot keyboard controller rearm only at auth entry.
    // This avoids repeated reprogramming in hot input loops while still
    // recovering machines where scanning is not active after boot.
    if snapshot.ps2_present && snapshot.ps2_irq_count == 0 && snapshot.ps2_polled_count == 0 {
        crate::drivers::keyboard::ensure_controller_ready("pre-auth-bootstrap");
    }

    // Only run invasive USB reprobe when PS/2 is not a healthy primary path.
    // This prevents pre-auth from repeatedly coupling input responsiveness to
    // xHCI topology churn on machines where the internal keyboard is PS/2.
    let usb_keyboard_primary = !snapshot.ps2_present || snapshot.ps2_init_failed;
    if usb_keyboard_primary && (snapshot.usb_controllers == 0 || snapshot.usb_hid_keyboards == 0)
    {
        let probed = crate::hal::usb::probe_controllers();
        crate::serial_println!(
            "[input] pre-auth-bootstrap: usb probe controllers={}",
            probed
        );
    }

    if snapshot.usb_hid_keyboards != 0 && snapshot.usb_hid_armed == 0 {
        crate::serial_println!("[input] pre-auth-bootstrap: usb keyboard present but not armed; polling");
    }

    for _ in 0..3 {
        let _ = hal::input::input_poll();
    }
}

fn cycle_language(step: isize) -> Language {
    let languages = ui::supported_languages();
    let current = ui::language();
    let current_index = languages
        .iter()
        .position(|(language, _, _)| *language == current)
        .unwrap_or(0);
    let next_index = step_wrapped(current_index, languages.len(), step);
    languages[next_index].0
}

fn cycle_ready_layout(step: isize) -> KeyboardLayout {
    let current = ui::keyboard_layout();
    let mut ready_count = 0usize;
    let mut current_ready_index = 0usize;
    let mut current_is_ready = false;

    for &(_, layout, ready) in ui::keyboard_layout_choices() {
        if !ready {
            continue;
        }
        if layout == current {
            current_ready_index = ready_count;
            current_is_ready = true;
        }
        ready_count = ready_count.saturating_add(1);
    }

    if ready_count == 0 {
        return current;
    }

    let next_ready_index = step_wrapped(
        if current_is_ready { current_ready_index } else { 0 },
        ready_count,
        step,
    );

    let mut ready_index = 0usize;
    for &(_, layout, ready) in ui::keyboard_layout_choices() {
        if !ready {
            continue;
        }
        if ready_index == next_ready_index {
            return layout;
        }
        ready_index = ready_index.saturating_add(1);
    }

    current
}

fn step_wrapped(current_index: usize, len: usize, step: isize) -> usize {
    if len == 0 {
        return 0;
    }

    let len = len as isize;
    let current = current_index as isize;
    (current + step).rem_euclid(len) as usize
}

fn apply_layout_choice(layout: KeyboardLayout, dirty: &mut bool) -> Result<String, &'static str> {
    let changed = ui::keyboard_layout() != layout;
    if changed {
        ui::set_keyboard_layout(layout)?;
    }
    if changed {
        *dirty = true;
    }

    let message = match layout {
        KeyboardLayout::UsQwerty => String::from("Keyboard layout set to en-US."),
        KeyboardLayout::BrazilAbnt2 => String::from("Keyboard layout set to pt-BR."),
        _ => alloc::format!("Keyboard layout set to {}.", ui::layout_label(layout)),
    };
    Ok(message)
}

fn render_preferences_screen(first_boot: bool, focus: OnboardingFocus, status: &str) {
    if !console::screen_owner_is(console::ScreenOwner::Auth) {
        crate::serial_println!(
            "[SCREEN] auth-render blocked owner={:?} stage=preferences",
            console::current_screen_owner()
        );
        return;
    }
    let _ = console::clear_screen_for(console::ScreenOwner::Auth, "render-preferences");
    kprintln!();
    kprint_colored!(Colors::GREEN, "  WAR ENTERPRISE\n");
    kprint_colored!(
        Colors::CYAN,
        "  WarOS v{}  |  {}\n",
        KERNEL_VERSION,
        ui::text(TextId::PreferencesTitle)
    );
    kprint_colored!(
        Colors::CYAN,
        "  ===========================================================\n"
    );
    if first_boot {
        kprint_colored!(Colors::GREEN, "  {}\n", ui::text(TextId::SetupTitle));
    } else {
        kprint_colored!(Colors::GREEN, "  {}\n", ui::text(TextId::LoginTitle));
    }
    kprint_colored!(Colors::DIM, "  {}\n", ui::text(TextId::PreferencesSubtitle));
    kprintln!();
    kprint_colored!(Colors::DIM, "  SAFE INPUT ONBOARDING\n");
    kprint_colored!(Colors::DIM, "  Focus: ");
    for section in [
        OnboardingFocus::Language,
        OnboardingFocus::Layout,
        OnboardingFocus::Continue,
    ] {
        if section == focus {
            kprint_colored!(Colors::GREEN, "[{}] ", section.label());
        } else {
            kprint_colored!(Colors::DIM, "{} ", section.label());
        }
    }
    kprintln!();

    let current_language = ui::language();
    let current_layout = ui::keyboard_layout();
    kprint_colored!(Colors::CYAN, "  {}\n", ui::text(TextId::PreferencesLanguage));
    for &(language, label, ready) in ui::supported_languages() {
        let marker = if language == current_language { ">" } else { " " };
        let readiness = if ready { "ready" } else { "staged" };
        let selected = language == current_language;
        let highlighted = focus == OnboardingFocus::Language && selected;
        if highlighted {
            kprint_colored!(Colors::GREEN, "  {} {:<20} {}\n", marker, label, readiness);
        } else if selected {
            kprint_colored!(Colors::CYAN, "  {} {:<20} {}\n", marker, label, readiness);
        } else {
            kprintln!("  {} {:<20} {}", marker, label, readiness);
        }
    }
    kprintln!();

    kprint_colored!(Colors::CYAN, "  {}\n", ui::text(TextId::PreferencesKeyboard));
    for &(label, layout, ready) in ui::keyboard_layout_choices() {
        let marker = if layout == current_layout && ready { ">" } else { " " };
        let readiness = if ready {
            ui::text(TextId::PreferencesLayoutReady)
        } else {
            ui::text(TextId::PreferencesLayoutStaged)
        };
        let selected = layout == current_layout && ready;
        let highlighted = focus == OnboardingFocus::Layout && selected;
        if highlighted {
            kprint_colored!(Colors::GREEN, "  {} {:<24} {}\n", marker, label, readiness);
        } else if selected {
            kprint_colored!(Colors::CYAN, "  {} {:<24} {}\n", marker, label, readiness);
        } else if !ready {
            kprint_colored!(Colors::DIM, "  {} {:<24} {}\n", marker, label, readiness);
        } else {
            kprintln!("  {} {:<24} {}", marker, label, readiness);
        }
    }
    kprintln!();

    kprint_colored!(Colors::CYAN, "  CURRENT ACCESS PROFILE\n");
    kprint_colored!(
        Colors::DIM,
        "  {}: {}    {}: {}\n",
        ui::text(TextId::CurrentLanguage),
        current_language.label(),
        ui::text(TextId::CurrentLayout),
        ui::layout_label(current_layout)
    );
    if let Some(note) = ui::localization_status_note() {
        kprint_colored!(Colors::YELLOW, "  note: ");
        kprintln!("{}", note);
    }
    kprint_colored!(Colors::YELLOW, "  note: ");
    kprintln!("{}", ui::text(TextId::KeyboardSupportStaged));
    kprint_colored!(Colors::YELLOW, "  note: ");
    kprintln!("{}", ui::text(TextId::PreferencesTimezoneLater));
    kprintln!();
    kprint_colored!(Colors::DIM, "  navigation: ");
    kprintln!("Up/Down change selection. Left/Right or Tab move focus.");
    kprint_colored!(Colors::DIM, "  actions:    ");
    kprintln!("Space or Enter continues with the current selections. Esc goes back.");
    kprint_colored!(Colors::DIM, "  support:    ");
    kprintln!("Pre-layout onboarding uses only navigation-intent keys.");
    if !status.is_empty() {
        kprintln!();
        render_status_line(status);
    }
    render_onboarding_debug_overlay();
    kprintln!();
    if focus == OnboardingFocus::Continue {
        kprint_colored!(Colors::GREEN, "  > {}\n", ui::text(TextId::PreferencesContinue));
    } else {
        kprint_colored!(Colors::DIM, "    {}\n", ui::text(TextId::PreferencesContinue));
    }
}

fn render_credential_screen(
    title: &str,
    subtitle: &str,
    focus: CredentialFocus,
    username: Option<&str>,
    status: &str,
    first_boot: bool,
) {
    if !console::screen_owner_is(console::ScreenOwner::Auth) {
        crate::serial_println!(
            "[SCREEN] auth-render blocked owner={:?} stage=credentials",
            console::current_screen_owner()
        );
        return;
    }
    let _ = console::clear_screen_for(console::ScreenOwner::Auth, "render-credentials");
    kprintln!();
    kprint_colored!(Colors::GREEN, "  WAR ENTERPRISE\n");
    kprint_colored!(Colors::CYAN, "  WarOS v{}  |  {}\n", KERNEL_VERSION, title);
    kprint_colored!(
        Colors::CYAN,
        "  ===========================================================\n"
    );
    kprint_colored!(Colors::GREEN, "  {}\n", subtitle);
    kprint_colored!(
        Colors::DIM,
        "  {}: {}    {}: {}\n",
        ui::text(TextId::CurrentLanguage),
        ui::language().label(),
        ui::text(TextId::CurrentLayout),
        ui::layout_label(ui::keyboard_layout())
    );
    kprint_colored!(Colors::CYAN, "  -----------------------------------------------------------\n");
    kprint_colored!(Colors::DIM, "  ACCESS PANEL\n");
    kprint_colored!(
        if focus == CredentialFocus::Username {
            Colors::GREEN
        } else {
            Colors::DIM
        },
        "  {} {}\n",
        if focus == CredentialFocus::Username { ">" } else { " " },
        ui::text(TextId::UsernameLabel)
    );
    kprint_colored!(
        Colors::DIM,
        "    {}\n",
        username.unwrap_or("enter account identifier")
    );
    kprint_colored!(
        if focus == CredentialFocus::Password {
            Colors::GREEN
        } else {
            Colors::DIM
        },
        "  {} {}\n",
        if focus == CredentialFocus::Password { ">" } else { " " },
        ui::text(TextId::PasswordLabel)
    );
    kprint_colored!(Colors::DIM, "    protected entry\n");
    if first_boot {
        kprint_colored!(
            if focus == CredentialFocus::Confirm {
                Colors::GREEN
            } else {
                Colors::DIM
            },
            "  {} {}\n",
            if focus == CredentialFocus::Confirm { ">" } else { " " },
            ui::text(TextId::ConfirmLabel)
        );
        kprint_colored!(Colors::DIM, "    repeat password to commit setup\n");
    }
    kprint_colored!(Colors::CYAN, "  -----------------------------------------------------------\n");
    kprint_colored!(
        Colors::DIM,
        "  Input: Enter submits, Backspace edits, Space remains literal text.\n"
    );
    if !first_boot {
        kprint_colored!(
            Colors::DIM,
            "  Access preferences: press Tab or Esc on an empty username field.\n"
        );
    }
    if let Some(note) = ui::localization_status_note() {
        kprint_colored!(Colors::YELLOW, "  note: ");
        kprintln!("{}", note);
    }
    if !status.is_empty() {
        kprintln!();
        render_status_line(status);
    }
    render_onboarding_debug_overlay();
    kprintln!();
}

fn render_onboarding_debug_overlay() {
    if !ONBOARDING_CONFIRM_TRACE_ENABLED {
        return;
    }

    let overlay = onboarding_overlay_snapshot();
    if overlay.key.is_empty()
        && overlay.ps2.is_empty()
        && overlay.validate.is_empty()
        && overlay.commit.is_empty()
        && overlay.exit.is_empty()
        && overlay.next.is_empty()
    {
        return;
    }

    kprintln!();
    kprint_colored!(Colors::DIM, "  DEBUG CONFIRM TRACE\n");
    if !overlay.key.is_empty() {
        kprint_colored!(Colors::DIM, "  key:   {}\n", overlay.key);
    }
    if !overlay.ps2.is_empty() {
        kprint_colored!(Colors::DIM, "  ps2:   {}\n", overlay.ps2);
    }
    if !overlay.validate.is_empty() {
        kprint_colored!(Colors::DIM, "  check: {}\n", overlay.validate);
    }
    if !overlay.commit.is_empty() {
        kprint_colored!(Colors::DIM, "  save:  {}\n", overlay.commit);
    }
    if !overlay.exit.is_empty() {
        kprint_colored!(Colors::DIM, "  exit:  {}\n", overlay.exit);
    }
    if !overlay.next.is_empty() {
        kprint_colored!(Colors::DIM, "  next:  {}\n", overlay.next);
    }
}

fn read_menu_key_timeout_interactive(
    timeout_ticks: u64,
) -> Option<crate::hal::input::InputKeypress> {
    let keypress = crate::interactive::read_keypress(
        crate::interactive::InteractiveStage::AuthPreferences,
        "pre-auth-read",
        Some(timeout_ticks),
    )?;
    trace_ui_byte("pre-auth-read", keypress.byte, "delivered");
    Some(keypress)
}

/// Show live input hardware diagnostics at the bottom of the preferences screen.
fn render_input_diagnostic() {
    let diag = hal::input::diagnostic_snapshot();
    kprintln!();
    kprint_colored!(
        Colors::CYAN,
        "------------------------------------------------------------\n"
    );
    kprint_colored!(Colors::DIM, "  INPUT DIAGNOSTICS (live)\n");
    kprint_colored!(Colors::DIM, "  ticks={:<8} interrupts={}\n",
        diag.ticks,
        if diag.interrupts_enabled { "ON" } else { "OFF" }
    );
    kprint_colored!(Colors::DIM, "  ps2: present={} failed={} irq={} polled={} xlat={} status=0x{:02X} fpoll={}\n",
        if diag.ps2_present { "Y" } else { "N" },
        if diag.ps2_init_failed { "Y" } else { "N" },
        diag.ps2_irq_count,
        diag.ps2_polled_count,
        diag.ps2_translated,
        diag.ps2_last_status,
        if diag.ps2_forced_polling { "Y" } else { "N" }
    );
    kprint_colored!(
        Colors::DIM,
        "  ps2 decode: raw=0x{:02X} out=0x{:02X} active={} selected={} pre_e0={} pre_f0={} s1={} s2={} fb={} dyn={} xlat={} from_fb={}\n",
        diag.ps2_last_scancode,
        diag.ps2_last_byte,
        ps2_set_label(diag.ps2_active_set),
        ps2_set_label(diag.ps2_selected_set),
        if diag.ps2_prefix_e0_before { "1" } else { "0" },
        if diag.ps2_prefix_f0_before { "1" } else { "0" },
        format_optional_byte(diag.ps2_set1_byte),
        format_optional_byte(diag.ps2_set2_byte),
        format_optional_byte(diag.ps2_fallback_byte),
        if diag.ps2_dynamic_switch { "1" } else { "0" },
        if diag.ps2_translation_enabled { "1" } else { "0" },
        if diag.ps2_emitted_from_fallback { "1" } else { "0" }
    );
    kprint_colored!(Colors::DIM, "  usb: controllers={} hid_kbd={} armed={} key_queue={} dropped={}\n",
        diag.usb_controllers,
        diag.usb_hid_keyboards,
        diag.usb_hid_armed,
        diag.key_queue_len,
        diag.dropped_key_events
    );
    kprint_colored!(Colors::DIM, "  stall={} last_src={} last_key=0x{:02X}\n",
        diag.stall_count,
        match diag.last_source { 1 => "ps2", 2 => "usb", _ => "none" },
        diag.last_keycode
    );
    // Show visible proof of key delivery
    match diag.last_source {
        1 => kprint_colored!(Colors::GREEN, "  LAST INPUT: source=ps2 key=0x{:02X}\n", diag.last_keycode),
        2 => kprint_colored!(Colors::GREEN, "  LAST INPUT: source=usb key=0x{:02X}\n", diag.last_keycode),
        _ => kprint_colored!(Colors::YELLOW, "  LAST INPUT: none (waiting for keypress...)\n"),
    }
}

/// Log and verify that interrupts are enabled before entering the interactive screen.
fn verify_interrupt_state(context: &str) {
    let ticks = crate::arch::x86_64::interrupts::tick_count();
    let enabled = x86_64::instructions::interrupts::are_enabled();
    if AUTH_TRACE_VERBOSE {
        crate::serial_println!(
            "[input] verify_interrupt_state({}) ticks={} IF={}",
            context,
            ticks,
            if enabled { "on" } else { "off" }
        );
    }
    if !enabled {
        crate::serial_println!(
            "[input] WARNING: interrupts OFF at {}; forcing enable",
            context
        );
        x86_64::instructions::interrupts::enable();
    }
}

fn log_keyboard_snapshot(stage: &str) {
    let snapshot = crate::drivers::keyboard::debug_snapshot();
    crate::serial_println!(
        "[input] {} irq_triggers={} irq_scancodes={} polled_scancodes={} translated_bytes={} consumed_bytes={} last_scancode=0x{:02X} last_byte=0x{:02X} last_status=0x{:02X} last_self_test=0x{:02X} forced_polling={} active={} selected={} pre_e0={} pre_f0={} s1={} s2={} fb={} dyn={} xlat={}",
        stage,
        snapshot.irq_triggers,
        snapshot.irq_scancodes,
        snapshot.polled_scancodes,
        snapshot.translated_bytes,
        snapshot.consumed_bytes,
        snapshot.last_scancode,
        snapshot.last_byte,
        snapshot.last_status,
        snapshot.last_self_test,
        if snapshot.forced_polling { "yes" } else { "no" },
        ps2_set_label(snapshot.ps2_active_set),
        ps2_set_label(snapshot.ps2_selected_set),
        if snapshot.ps2_prefix_e0_before {
            "1"
        } else {
            "0"
        },
        if snapshot.ps2_prefix_f0_before {
            "1"
        } else {
            "0"
        },
        format_optional_byte(snapshot.ps2_set1_byte),
        format_optional_byte(snapshot.ps2_set2_byte),
        format_optional_byte(snapshot.ps2_fallback_byte),
        if snapshot.ps2_dynamic_switch { "1" } else { "0" },
        if snapshot.ps2_translation_enabled {
            "1"
        } else {
            "0"
        }
    );
}

fn trace_ui_byte(stage: &str, byte: u8, outcome: &str) {
    if !AUTH_TRACE_VERBOSE {
        let _ = (stage, byte, outcome);
        return;
    }
    crate::serial_println!(
        "[input] {} {} byte=0x{:02X} '{}'",
        stage,
        outcome,
        byte,
        if byte.is_ascii_graphic() || byte == b' ' {
            byte as char
        } else {
            '.'
        }
    );
}

fn trace_handoff(stage: &str, event: &str) {
    if !AUTH_HANDOFF_TRACE_ENABLED {
        let _ = (stage, event);
        return;
    }
    crate::serial_println!(
        "[interactive_handoff] stage={} event={} ticks={} IF={} render={} gui={}",
        stage,
        event,
        crate::arch::x86_64::interrupts::tick_count(),
        if x86_64::instructions::interrupts::are_enabled() {
            "on"
        } else {
            "off"
        },
        if console::rendering_enabled() {
            "on"
        } else {
            "off"
        },
        if crate::gui::is_active() { "on" } else { "off" }
    );
}

fn is_valid_username(username: &str) -> bool {
    if username.len() < 3 || username.len() > 32 {
        return false;
    }
    username
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

fn is_strong_enough_password(password: &str) -> bool {
    if password.len() < 8 {
        return false;
    }
    password.bytes().any(|byte| byte.is_ascii_graphic())
}

fn prime_input_path(context: &str) {
    verify_interrupt_state(context);
    // Keep handoff non-blocking on real hardware:
    // only service queued input sources here.
    let _ = hal::input::input_poll();
}

fn onboarding_action(keypress: crate::hal::input::InputKeypress) -> OnboardingAction {
    match keypress.byte {
        hal::input::KEY_ARROW_UP => OnboardingAction::MovePrevious,
        hal::input::KEY_ARROW_DOWN => OnboardingAction::MoveNext,
        hal::input::KEY_ARROW_LEFT => OnboardingAction::FocusPrevious,
        hal::input::KEY_ARROW_RIGHT => OnboardingAction::FocusNext,
        b'\t' if keypress.shift => OnboardingAction::FocusPrevious,
        b'\t' => OnboardingAction::FocusNext,
        b' ' | b'\n' | b'\r' => OnboardingAction::Confirm,
        0x1B => OnboardingAction::Back,
        _ => OnboardingAction::Ignore,
    }
}

fn render_status_line(status: &str) {
    let lower = status.to_ascii_lowercase();
    let color = if lower.contains("failed")
        || lower.contains("invalid")
        || lower.contains("must")
        || lower.contains("mismatch")
        || lower.contains("requires")
    {
        Colors::RED
    } else if lower.contains("waiting") || lower.contains("staged") {
        Colors::YELLOW
    } else {
        Colors::GREEN
    };
    kprint_colored!(color, "  status: ");
    kprintln!("{}", status);
}
