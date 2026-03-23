mod commands;
mod utils;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use commands::{bench, ibm, qstat, repl, run, show};
use utils::CliResult;

#[derive(Parser)]
#[command(name = "waros", about = "WarOS Quantum Computing Toolkit")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Execute a quantum circuit from a QASM file.
    Run {
        /// Path to the QASM file.
        file: PathBuf,
        /// Number of shots.
        #[arg(short, long, default_value_t = 1000)]
        shots: u32,
        /// Noise profile: ideal, ibm, ionq, or custom:s,t,r.
        #[arg(short, long, default_value = "ideal")]
        noise: String,
        /// Random seed for reproducibility.
        #[arg(long)]
        seed: Option<u64>,
    },
    /// Show simulated quantum system status.
    Qstat,
    /// Display a circuit diagram from a QASM file.
    Show {
        /// Path to the QASM file.
        file: PathBuf,
    },
    /// Run lightweight performance probes.
    Bench {
        /// Number of qubits to benchmark.
        #[arg(short, long, default_value_t = 15)]
        qubits: usize,
    },
    /// Start an interactive quantum REPL.
    Repl {
        /// Number of qubits in the scratch circuit.
        #[arg(short, long, default_value_t = 5)]
        qubits: usize,
    },
    /// Work with IBM Quantum Runtime hardware backends.
    Ibm {
        #[command(subcommand)]
        command: IbmCommand,
    },
}

#[derive(Subcommand)]
enum IbmCommand {
    /// Save IBM Quantum credentials for future commands.
    Login {
        /// IBM Quantum Platform API key.
        #[arg(long)]
        token: Option<String>,
        /// IBM Quantum service instance CRN.
        #[arg(long)]
        instance_crn: Option<String>,
    },
    /// List available IBM backends.
    Backends,
    /// Run a QASM circuit on IBM hardware and wait for the result.
    Run {
        /// Path to the QASM file.
        file: PathBuf,
        /// IBM backend name, such as ibm_brisbane.
        #[arg(long, default_value = "ibm_brisbane")]
        backend: String,
        /// Number of shots.
        #[arg(short, long, default_value_t = 1000)]
        shots: u32,
        /// Maximum time to wait for the queued job.
        #[arg(long, default_value_t = 600)]
        timeout_secs: u64,
    },
    /// Check the status of a submitted IBM job.
    Status {
        /// IBM Quantum job ID.
        job_id: String,
    },
    /// Fetch the final result for a submitted IBM job.
    Result {
        /// IBM Quantum job ID.
        job_id: String,
        /// Maximum time to wait for the queued job.
        #[arg(long, default_value_t = 600)]
        timeout_secs: u64,
    },
}

fn main() -> CliResult {
    let cli = Cli::parse();
    match cli.command {
        Command::Run {
            file,
            shots,
            noise,
            seed,
        } => run::execute(&file, shots, &noise, seed),
        Command::Qstat => qstat::execute(),
        Command::Show { file } => show::execute(&file),
        Command::Bench { qubits } => bench::execute(qubits),
        Command::Repl { qubits } => repl::execute(qubits),
        Command::Ibm { command } => match command {
            IbmCommand::Login {
                token,
                instance_crn,
            } => ibm::login(token, instance_crn),
            IbmCommand::Backends => ibm::backends(),
            IbmCommand::Run {
                file,
                backend,
                shots,
                timeout_secs,
            } => ibm::run(&file, &backend, shots, timeout_secs),
            IbmCommand::Status { job_id } => ibm::status(&job_id),
            IbmCommand::Result {
                job_id,
                timeout_secs,
            } => ibm::result(&job_id, timeout_secs),
        },
    }
}
