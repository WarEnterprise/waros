use crate::display::console::Colors;
use crate::{kprint_colored, kprintln, KERNEL_VERSION};

const MAJOR_SEPARATOR: &str =
    "================================================================================";
const MINOR_SEPARATOR: &str =
    "--------------------------------------------------------------------------------";

/// Render the WarOS boot banner.
pub fn show_banner() {
    kprint_colored!(Colors::DIM, "{}\n\n", MAJOR_SEPARATOR);
    kprint_colored!(Colors::PURPLE, "     W A R O S   v{}\n", KERNEL_VERSION);
    kprintln!();
    kprint_colored!(Colors::FG, "     Quantum-Classical Hybrid Operating System\n");
    kprint_colored!(Colors::DIM, "     War Enterprise (c) 2026\n");
    kprint_colored!(Colors::CYAN, "     github.com/WarEnterprise/waros\n");
    kprintln!();
    kprint_colored!(Colors::DIM, "{}\n\n", MAJOR_SEPARATOR);
}

/// Render the standard dim separator between boot phases or command sections.
pub fn show_separator() {
    kprint_colored!(Colors::DIM, "{}\n", MINOR_SEPARATOR);
}
