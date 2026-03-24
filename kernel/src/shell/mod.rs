use alloc::string::String;

use x86_64::instructions::hlt;

use crate::auth::session;
use crate::display::console::Colors;
use crate::hal;
use crate::shell::commands::execute_command;
use crate::task;
use crate::{kprint, kprint_colored, kprintln};

pub mod commands;
pub mod history;

const INPUT_LIMIT: usize = 256;

fn prompt() {
    let Some(user) = session::current_user() else {
        kprint!("login> ");
        return;
    };

    let path = session::current_prompt_path();
    kprint_colored!(Colors::DIM, "[");
    kprint_colored!(Colors::CYAN, "WarOS");
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
    prompt();

    loop {
        if !session::is_logged_in() {
            return;
        }

        task::tick();
        hal::usb::poll();
        let _ = crate::net::poll();

        if let Some(byte) = hal::input::read_char() {
            match byte {
                b'\n' => {
                    kprintln!();
                    self::history::push(&input);
                    execute_command(&input);
                    if truncated {
                        kprint_colored!(Colors::YELLOW, "[WARN]");
                        kprintln!(" input truncated at {} characters.", INPUT_LIMIT);
                    }
                    input.clear();
                    truncated = false;
                    if !session::is_logged_in() {
                        return;
                    }
                    prompt();
                }
                0x08 => {
                    if !input.is_empty() {
                        input.pop();
                        crate::display::console::backspace();
                    }
                    truncated = false;
                }
                byte if byte.is_ascii_graphic() || byte == b' ' => {
                    if input.len() < INPUT_LIMIT {
                        input.push(char::from(byte));
                        kprint!("{}", char::from(byte));
                    } else {
                        truncated = true;
                    }
                }
                _ => {}
            }
        } else if !task::has_tasks() {
            hlt();
        }
    }
}
