use alloc::string::String;

use x86_64::instructions::hlt;

use crate::display::console::Colors;
use crate::drivers::keyboard;
use crate::shell::commands::execute_command;
use crate::{kprint, kprint_colored, kprintln};

pub mod commands;
pub mod history;

fn prompt() {
    kprint_colored!(Colors::GREEN, "waros");
    kprint_colored!(Colors::DIM, "> ");
}

/// Run the minimal interactive WarShell loop forever.
pub fn run() -> ! {
    let mut input = String::new();
    prompt();

    loop {
        if let Some(byte) = keyboard::read_char() {
            match byte {
                b'\n' => {
                    kprintln!();
                    self::history::push(&input);
                    execute_command(&input);
                    input.clear();
                    prompt();
                }
                0x08 => {
                    if !input.is_empty() {
                        input.pop();
                        crate::display::console::backspace();
                    }
                }
                byte if byte.is_ascii_graphic() || byte == b' ' => {
                    if input.len() < 256 {
                        input.push(char::from(byte));
                        kprint!("{}", char::from(byte));
                    }
                }
                _ => {}
            }
        } else {
            hlt();
        }
    }
}
