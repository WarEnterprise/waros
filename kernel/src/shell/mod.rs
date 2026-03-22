use alloc::string::String;

use x86_64::instructions::hlt;

use crate::drivers::keyboard;
use crate::shell::commands::execute_command;
use crate::{print, println};

pub mod commands;

/// Run the minimal interactive WarShell loop forever.
pub fn run() -> ! {
    let mut input = String::new();
    print!("waros> ");

    loop {
        if let Some(byte) = keyboard::read_char() {
            match byte {
                b'\n' => {
                    println!();
                    execute_command(&input);
                    input.clear();
                    print!("waros> ");
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
                        print!("{}", char::from(byte));
                    }
                }
                _ => {}
            }
        } else {
            hlt();
        }
    }
}
