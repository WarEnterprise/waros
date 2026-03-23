use std::thread;

use sysinfo::System;

use crate::utils::CliResult;

#[allow(clippy::unnecessary_wraps)]
pub fn execute() -> CliResult {
    let mut system = System::new();
    system.refresh_memory();
    let available_memory = system.available_memory();
    let amplitude_budget = available_memory / 16;
    let max_qubits = if amplitude_budget == 0 {
        0
    } else {
        usize::try_from(u64::BITS - 1 - amplitude_budget.leading_zeros()).unwrap_or(0)
    };
    let threads = thread::available_parallelism()
        .map(std::num::NonZeroUsize::get)
        .unwrap_or(1);
    let available_mib = available_memory / 1024 / 1024;

    println!("╔══════════════════════════════════════════════════╗");
    println!("║              WarOS Quantum - System Status      ║");
    println!("╚══════════════════════════════════════════════════╝");
    println!();
    println!("  Backend:     StateVector (classical simulation)");
    println!("  Max qubits:  {max_qubits} (estimated from {available_mib} MiB available RAM)");
    println!("  Threads:     {threads} (Rayon parallel for >=16 qubits)");
    println!("  Features:    QFT, noise simulation, QASM parser");
    println!("  Version:     waros-quantum {}", env!("CARGO_PKG_VERSION"));
    println!();
    println!("  Note: Running in simulation mode. Connect real QPU");
    println!("        hardware via QHAL drivers (coming soon).");
    Ok(())
}
