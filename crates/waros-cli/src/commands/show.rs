use std::path::Path;

use waros_quantum::parse_qasm;

use crate::utils::{read_utf8, CliResult};

pub fn execute(file: &Path) -> CliResult {
    let source = read_utf8(file)?;
    let circuit = parse_qasm(&source)?;
    println!("{}", circuit.to_ascii());
    Ok(())
}
