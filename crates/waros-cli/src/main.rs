mod commands;
mod utils;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use commands::{bench, qstat, repl, run, show};
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
    }
}
