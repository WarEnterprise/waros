use std::thread;

use sysinfo::System;

use crate::utils::CliResult;

pub fn execute() -> CliResult {
    let mut system = System::new();
    system.refresh_memory();
    let available_memory = system.available_memory();
    let max_qubits = if available_memory < 16 {
        0
    } else {
        ((available_memory / 16) as f64).log2().floor() as usize
    };
    let threads = thread::available_parallelism()
        .map(|parallelism| parallelism.get())
        .unwrap_or(1);

    println!("╔══════════════════════════════════════════════════╗");
    println!("║              WarOS Quantum - System Status      ║");
    println!("╚══════════════════════════════════════════════════╝");
    println!();
    println!("  Backend:     StateVector (classical simulation)");
    println!(
        "  Max qubits:  {max_qubits} (estimated from {:.1} GiB available RAM)",
        available_memory as f64 / 1024.0 / 1024.0 / 1024.0
    );
    println!("  Threads:     {threads} (Rayon parallel for >=16 qubits)");
    println!("  Features:    QFT, noise simulation, QASM parser");
    println!("  Version:     waros-quantum {}", env!("CARGO_PKG_VERSION"));
    println!();
    println!("  Note: Running in simulation mode. Connect real QPU");
    println!("        hardware via QHAL drivers (coming soon).");
    Ok(())
}
