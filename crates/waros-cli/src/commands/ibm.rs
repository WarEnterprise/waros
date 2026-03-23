use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

use waros_quantum::backends::ibm::{auth, IBMBackend};
use waros_quantum::parse_qasm;

use crate::utils::{read_utf8, CliResult};

pub fn login(token: Option<String>, instance_crn: Option<String>) -> CliResult {
    let token = token.unwrap_or(prompt("IBM Quantum API key")?);
    let instance_crn = instance_crn.unwrap_or(prompt("IBM Quantum service CRN")?);
    auth::save_credentials(&token, &instance_crn)?;
    println!("Saved IBM Quantum credentials to ~/.waros.");
    Ok(())
}

pub fn backends() -> CliResult {
    let ibm = IBMBackend::from_env()?;
    for backend in ibm.list_backends()? {
        println!(
            "{}: {} qubits, status={}, queue={}",
            backend.name,
            backend.num_qubits,
            backend.status.message(),
            backend.queue_length
        );
    }
    Ok(())
}

pub fn run(file: &Path, backend: &str, shots: u32, timeout_secs: u64) -> CliResult {
    let source = read_utf8(file)?;
    let circuit = parse_qasm(&source)?;
    let ibm = IBMBackend::from_env()?;
    let result =
        ibm.run_with_timeout(backend, &circuit, shots, Duration::from_secs(timeout_secs))?;
    result.print_histogram();
    Ok(())
}

pub fn status(job_id: &str) -> CliResult {
    let ibm = IBMBackend::from_env()?;
    println!("{:?}", ibm.job_status_by_id(job_id)?);
    Ok(())
}

pub fn result(job_id: &str, timeout_secs: u64) -> CliResult {
    let ibm = IBMBackend::from_env()?;
    let result = ibm.wait_for_job_id(job_id, Duration::from_secs(timeout_secs))?;
    result.print_histogram();
    Ok(())
}

fn prompt(label: &str) -> CliResult<String> {
    print!("{label}: ");
    io::stdout().flush()?;
    let mut value = String::new();
    io::stdin().read_line(&mut value)?;
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(format!("{label} cannot be empty").into());
    }
    Ok(value)
}
