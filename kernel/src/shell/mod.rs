use alloc::string::{String, ToString};

use x86_64::instructions::hlt;

use crate::display::console::Colors;
use crate::drivers::keyboard;
use crate::shell::commands::execute_command;
use crate::task;
use crate::{kprint, kprint_colored, kprintln};

pub mod commands;
pub mod history;

const INPUT_LIMIT: usize = 256;

fn prompt() {
    let ip = crate::net::network_config()
        .map(|config| config.ip.to_string())
        .unwrap_or_else(|| "offline".to_string());

    kprint_colored!(Colors::DIM, "[");
    kprint_colored!(Colors::GREEN, "WarOS");
    kprint_colored!(Colors::DIM, " ");
    kprint_colored!(Colors::YELLOW, "{}", ip);
    kprint_colored!(Colors::DIM, " /]$ ");
}

/// Re-render the shell prompt.
pub fn reprompt() {
    prompt();
}

/// Run the minimal interactive WarShell loop forever.
pub fn run() -> ! {
    let mut input = String::new();
    let mut truncated = false;
    prompt();

    loop {
        task::tick();
        let _ = crate::net::poll();

        if let Some(byte) = keyboard::read_char() {
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
