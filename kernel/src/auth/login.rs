use alloc::string::String;

use x86_64::instructions::hlt;

use crate::auth::{AuthError, UserAccount, UserRole, USER_DB};
use crate::display::console::{self, Colors};
use crate::hal;
use crate::{kprint, kprint_colored, kprintln, KERNEL_VERSION};

pub fn first_boot_setup() -> UserAccount {
    loop {
        console::clear_screen();
        kprintln!();
        kprint_colored!(
            Colors::CYAN,
            "===================================================\n"
        );
        kprint_colored!(Colors::GREEN, "  WarOS v{} - First Time Setup\n", KERNEL_VERSION);
        kprintln!("  War Enterprise");
        kprintln!("===================================================\n");
        kprintln!("  Welcome to WarOS. Create your admin account.\n");

        kprint_colored!(Colors::DIM, "  Username: ");
        let username = read_line_echo();
        if username.is_empty() {
            continue;
        }

        kprint_colored!(Colors::DIM, "  Password: ");
        let password = read_line_hidden();
        kprintln!();
        kprint_colored!(Colors::DIM, "  Confirm:  ");
        let confirm = read_line_hidden();
        kprintln!();

        if password != confirm {
            kprint_colored!(Colors::RED, "[WarOS] ");
            kprintln!("Passwords do not match.");
            wait_ticks(150);
            continue;
        }

        let mut db = USER_DB.lock();
        match db.create_user(&username, &password, UserRole::Admin) {
            Ok(uid) => {
                db.save_to_fs();
                let user = db
                    .find_by_uid(uid)
                    .cloned()
                    .expect("newly created admin missing");
                drop(db);
                kprintln!();
                kprint_colored!(Colors::GREEN, "  Account created.");
                kprintln!(" You are now the system administrator.");
                kprintln!();
                wait_ticks(100);
                return user;
            }
            Err(error) => {
                drop(db);
                kprint_colored!(Colors::RED, "[WarOS] ");
                kprintln!("Failed to create account: {}.", error);
                wait_ticks(150);
            }
        }
    }
}

pub fn login_screen() -> UserAccount {
    let mut failed_attempts = 0u8;

    loop {
        console::clear_screen();
        kprintln!();
        kprint_colored!(
            Colors::CYAN,
            "===================================================\n"
        );
        kprint_colored!(Colors::GREEN, "  WarOS v{} - War Enterprise\n", KERNEL_VERSION);
        kprintln!("  Quantum-Classical Hybrid Operating System");
        kprintln!("===================================================\n");

        kprint_colored!(Colors::DIM, "  login: ");
        let username = read_line_echo();
        if username.is_empty() {
            continue;
        }

        kprint_colored!(Colors::DIM, "  password: ");
        let password = read_line_hidden();
        kprintln!();

        let mut db = USER_DB.lock();
        match db.authenticate(&username, &password) {
            Ok(user) => {
                let previous_login = db.record_login(user.uid).unwrap_or(0);
                db.save_to_fs();
                drop(db);

                crate::security::audit::log_event(
                    crate::security::audit::events::AuditEvent::LoginSuccess {
                        username: user.username.clone(),
                        uid: user.uid,
                    },
                );

                kprintln!();
                kprint_colored!(Colors::GREEN, "  Welcome back");
                kprintln!(", {}.", user.username);
                if previous_login != 0 {
                    let delta = crate::arch::x86_64::interrupts::tick_count()
                        .saturating_sub(previous_login);
                    kprint_colored!(Colors::DIM, "  Last login: ");
                    kprintln!("{} uptime ago", crate::fs::format_timestamp(delta));
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
                kprintln!();
                kprint_colored!(Colors::RED, "  [WarOS] ");
                match error {
                    AuthError::UserNotFound => kprintln!("User not found."),
                    AuthError::WrongPassword => kprintln!("Incorrect password."),
                    AuthError::AccountDisabled => kprintln!("Account disabled."),
                    _ => kprintln!("Authentication failed."),
                }

                if failed_attempts >= 3 {
                    kprint_colored!(Colors::RED, "  Too many failed attempts. ");
                    kprintln!("Waiting 30 seconds...");
                    wait_ticks(3_000);
                    failed_attempts = 0;
                } else {
                    wait_ticks(100);
                }
            }
        }
    }
}

pub fn read_line_echo() -> String {
    read_line(false)
}

pub fn read_line_hidden() -> String {
    read_line(true)
}

fn read_line(hidden: bool) -> String {
    let mut input = String::new();
    loop {
        if let Some(byte) = hal::input::read_char() {
            match byte {
                b'\n' => return input,
                0x08 => {
                    if !input.is_empty() {
                        input.pop();
                        console::backspace();
                    }
                }
                byte if byte.is_ascii_graphic() || byte == b' ' => {
                    input.push(byte as char);
                    if hidden {
                        kprint!("*");
                    } else {
                        kprint!("{}", byte as char);
                    }
                }
                _ => {}
            }
        } else {
            hal::usb::poll();
            hlt();
        }
    }
}

fn wait_ticks(ticks: u64) {
    let target = crate::arch::x86_64::interrupts::tick_count().saturating_add(ticks);
    while crate::arch::x86_64::interrupts::tick_count() < target {
        hlt();
    }
}
