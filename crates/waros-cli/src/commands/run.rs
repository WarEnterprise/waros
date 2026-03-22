use std::path::Path;

use waros_quantum::parse_qasm;

use crate::utils::{build_simulator, read_utf8, CliResult};

pub fn execute(file: &Path, shots: u32, noise: &str, seed: Option<u64>) -> CliResult {
    let source = read_utf8(file)?;
    let circuit = parse_qasm(&source)?;
    let simulator = build_simulator(noise, seed)?;
    let result = simulator.run(&circuit, shots)?;
    result.print_histogram();
    Ok(())
}
