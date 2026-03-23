use alloc::format;
use alloc::string::{String, ToString};

use core::f64::consts::PI;

use spin::Mutex;

use crate::display::console::Colors;
use crate::fs;
use crate::quantum::circuits::run_builtin;
use crate::quantum::display::{display_probabilities, display_results, display_state};
use crate::quantum::gates::{
    cnot, cz, hadamard, pauli_x, pauli_y, pauli_z, rx, ry, rz, s_gate, swap, t_gate,
};
use crate::quantum::session::QuantumSession;
use crate::quantum::simulator::{apply_1q, apply_2q, apply_toffoli, measure_all, Xorshift64};
use crate::quantum::state::{QuantumState, MAX_KERNEL_QUBITS};
use crate::{kprint_colored, kprintln};

pub mod circuits;
pub mod display;
pub mod gates;
pub mod session;
pub mod simulator;
pub mod state;

static QUANTUM_STATE: Mutex<Option<QuantumSession>> = Mutex::new(None);

/// Dispatch a shell quantum command.
pub fn handle_quantum_command(command: &str, args: &[&str]) -> Result<(), &'static str> {
    match command {
        "qalloc" => cmd_qalloc(args),
        "qfree" => {
            *QUANTUM_STATE.lock() = None;
            kprintln!("Quantum register freed.");
            Ok(())
        }
        "qreset" => {
            let mut guard = QUANTUM_STATE.lock();
            let Some(state) = guard.as_mut() else {
                kprintln!("No quantum register allocated. Use 'qalloc <n>' first.");
                return Ok(());
            };
            state.reset();
            kprintln!("Quantum register reset to |0...0>.");
            Ok(())
        }
        "qrun" => cmd_qrun(args),
        "qstate" => {
            let guard = QUANTUM_STATE.lock();
            let Some(session) = guard.as_ref() else {
                kprintln!("No quantum register allocated. Use 'qalloc <n>' first.");
                return Ok(());
            };
            display_state(&session.state);
            Ok(())
        }
        "qprobs" => {
            let guard = QUANTUM_STATE.lock();
            let Some(session) = guard.as_ref() else {
                kprintln!("No quantum register allocated. Use 'qalloc <n>' first.");
                return Ok(());
            };
            display_probabilities(&session.state);
            Ok(())
        }
        "qmeasure" => cmd_qmeasure(args),
        "qcircuit" => {
            let name = args.first().copied().unwrap_or("");
            run_builtin(name)
        }
        "qsave" | "qexport" => cmd_qsave(args),
        "qresult" => cmd_qresult(args),
        "qinfo" => {
            show_status();
            Ok(())
        }
        _ => {
            kprintln!(
                "Unknown quantum command '{}'. Type 'help quantum' for the command list.",
                command
            );
            Ok(())
        }
    }
}

/// Print detailed help for the in-kernel simulator.
pub fn show_help() {
    kprint_colored!(Colors::CYAN, "Quantum Computing Commands\n");
    kprintln!("  qalloc <n>          Allocate n-qubit register (max 18)");
    kprintln!("  qfree               Free the current quantum register");
    kprintln!("  qreset              Reset all qubits to |0>");
    kprintln!("  qrun <gate> <args>  Apply a quantum gate");
    kprintln!("  qstate              Show current state vector");
    kprintln!("  qprobs              Show probability distribution");
    kprintln!("  qmeasure [shots]    Measure current register (default: 100)");
    kprintln!("  qcircuit <name>     Run a built-in circuit demo");
    kprintln!("  qsave <name>        Save current circuit as QASM");
    kprintln!("  qexport <name>      Export current circuit as QASM");
    kprintln!("  qresult <name>      Save last measurement results");
    kprintln!("  qinfo               Quantum subsystem information");
    kprintln!();
    kprint_colored!(Colors::PURPLE, "Available gates for qrun\n");
    kprintln!("  h <q>               Hadamard");
    kprintln!("  x <q>               Pauli-X (NOT)");
    kprintln!("  y <q>               Pauli-Y");
    kprintln!("  z <q>               Pauli-Z");
    kprintln!("  s <q>               S gate");
    kprintln!("  t <q>               T gate");
    kprintln!("  cx <c> <t>          CNOT (controlled-X)");
    kprintln!("  cz <q0> <q1>        Controlled-Z");
    kprintln!("  swap <q0> <q1>      SWAP");
    kprintln!("  rx <q> <angle>      Rotation-X (radians or pi expressions)");
    kprintln!("  ry <q> <angle>      Rotation-Y (radians or pi expressions)");
    kprintln!("  rz <q> <angle>      Rotation-Z (radians or pi expressions)");
    kprintln!("  ccx <c0> <c1> <t>   Toffoli");
    kprintln!();
    kprint_colored!(Colors::PURPLE, "Built-in circuits for qcircuit\n");
    kprintln!("  bell                Bell state (2 qubits)");
    kprintln!("  ghz3                GHZ state (3 qubits)");
    kprintln!("  grover              Grover search 2-bit");
    kprintln!("  teleport            Quantum teleportation");
    kprintln!("  qft4                Quantum Fourier Transform (4 qubits)");
    kprintln!("  deutsch             Deutsch algorithm");
    kprintln!("  bernstein           Bernstein-Vazirani (3-bit secret)");
    kprintln!("  superdense          Superdense coding");
    kprintln!("  shor                Shor factoring demo (N = 15)");
    kprintln!("  vqe                 VQE hydrogen energy demo");
    kprintln!("  qaoa                QAOA triangle MaxCut demo");
}

/// Snapshot the current in-kernel register allocation.
#[must_use]
pub fn active_register() -> Option<(usize, usize)> {
    let guard = QUANTUM_STATE.lock();
    guard
        .as_ref()
        .map(|session| (session.state.num_qubits, session.state.bytes_used()))
}

fn cmd_qalloc(args: &[&str]) -> Result<(), &'static str> {
    let qubits = args
        .first()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);

    if !(1..=MAX_KERNEL_QUBITS).contains(&qubits) {
        kprintln!("Usage: qalloc <1-18>");
        kprintln!("  Max 18 qubits in kernel mode (~4 MiB state vector).");
        return Ok(());
    }

    let state = QuantumState::new(qubits)?;
    let amplitudes = state.dimension();
    let bytes = state.bytes_used();
    *QUANTUM_STATE.lock() = Some(QuantumSession::new(state));
    kprint_colored!(Colors::GREEN, "Allocated ");
    kprintln!(
        "{}-qubit register QR-0 ({} amplitudes, {} bytes)",
        qubits,
        amplitudes,
        bytes
    );
    Ok(())
}

fn cmd_qrun(args: &[&str]) -> Result<(), &'static str> {
    if args.is_empty() {
        kprintln!("Usage: qrun <gate> <qubit(s)> [params]");
        kprintln!("  Example: qrun h 0");
        kprintln!("  Example: qrun cx 0 1");
        kprintln!("  Example: qrun rz 0 pi/4");
        return Ok(());
    }

    let mut guard = QUANTUM_STATE.lock();
    let Some(session) = guard.as_mut() else {
        kprintln!("No quantum register allocated. Use 'qalloc <n>' first.");
        return Ok(());
    };
    let state = &mut session.state;

    let gate = args[0];
    match gate {
        "h" => {
            let qubit = parse_qubit(args, 1)?;
            apply_1q(state, qubit, &hadamard())?;
            session.record_operation(format!("h q[{qubit}];"));
            announce("Applied H to qubit", qubit);
        }
        "x" => {
            let qubit = parse_qubit(args, 1)?;
            apply_1q(state, qubit, &pauli_x())?;
            session.record_operation(format!("x q[{qubit}];"));
            announce("Applied X to qubit", qubit);
        }
        "y" => {
            let qubit = parse_qubit(args, 1)?;
            apply_1q(state, qubit, &pauli_y())?;
            session.record_operation(format!("y q[{qubit}];"));
            announce("Applied Y to qubit", qubit);
        }
        "z" => {
            let qubit = parse_qubit(args, 1)?;
            apply_1q(state, qubit, &pauli_z())?;
            session.record_operation(format!("z q[{qubit}];"));
            announce("Applied Z to qubit", qubit);
        }
        "s" => {
            let qubit = parse_qubit(args, 1)?;
            apply_1q(state, qubit, &s_gate())?;
            session.record_operation(format!("s q[{qubit}];"));
            announce("Applied S to qubit", qubit);
        }
        "t" => {
            let qubit = parse_qubit(args, 1)?;
            apply_1q(state, qubit, &t_gate())?;
            session.record_operation(format!("t q[{qubit}];"));
            announce("Applied T to qubit", qubit);
        }
        "rx" => {
            let qubit = parse_qubit(args, 1)?;
            let angle = parse_angle(args, 2)?;
            apply_1q(state, qubit, &rx(angle))?;
            session.record_operation(format!("rx({angle:.6}) q[{qubit}];"));
            kprint_colored!(Colors::GREEN, "Applied Rx to qubit ");
            kprintln!("{} (theta = {:.4})", qubit, angle);
        }
        "ry" => {
            let qubit = parse_qubit(args, 1)?;
            let angle = parse_angle(args, 2)?;
            apply_1q(state, qubit, &ry(angle))?;
            session.record_operation(format!("ry({angle:.6}) q[{qubit}];"));
            kprint_colored!(Colors::GREEN, "Applied Ry to qubit ");
            kprintln!("{} (theta = {:.4})", qubit, angle);
        }
        "rz" => {
            let qubit = parse_qubit(args, 1)?;
            let angle = parse_angle(args, 2)?;
            apply_1q(state, qubit, &rz(angle))?;
            session.record_operation(format!("rz({angle:.6}) q[{qubit}];"));
            kprint_colored!(Colors::GREEN, "Applied Rz to qubit ");
            kprintln!("{} (theta = {:.4})", qubit, angle);
        }
        "cx" | "cnot" => {
            let control = parse_qubit(args, 1)?;
            let target = parse_qubit(args, 2)?;
            apply_2q(state, control, target, &cnot())?;
            session.record_operation(format!("cx q[{control}], q[{target}];"));
            kprint_colored!(Colors::GREEN, "Applied CNOT: ");
            kprintln!("control = {}, target = {}", control, target);
        }
        "cz" => {
            let q0 = parse_qubit(args, 1)?;
            let q1 = parse_qubit(args, 2)?;
            apply_2q(state, q0, q1, &cz())?;
            session.record_operation(format!("cz q[{q0}], q[{q1}];"));
            kprint_colored!(Colors::GREEN, "Applied CZ: ");
            kprintln!("qubits = {}, {}", q0, q1);
        }
        "swap" => {
            let q0 = parse_qubit(args, 1)?;
            let q1 = parse_qubit(args, 2)?;
            apply_2q(state, q0, q1, &swap())?;
            session.record_operation(format!("swap q[{q0}], q[{q1}];"));
            kprint_colored!(Colors::GREEN, "Applied SWAP: ");
            kprintln!("qubits = {}, {}", q0, q1);
        }
        "ccx" | "toffoli" => {
            let control0 = parse_qubit(args, 1)?;
            let control1 = parse_qubit(args, 2)?;
            let target = parse_qubit(args, 3)?;
            apply_toffoli(state, control0, control1, target)?;
            session.record_operation(format!(
                "ccx q[{control0}], q[{control1}], q[{target}];"
            ));
            kprint_colored!(Colors::GREEN, "Applied Toffoli: ");
            kprintln!("controls = {}, {}, target = {}", control0, control1, target);
        }
        _ => {
            kprintln!(
                "Unknown gate '{}'. Type 'help quantum' for supported gates.",
                gate
            );
        }
    }

    Ok(())
}

fn cmd_qmeasure(args: &[&str]) -> Result<(), &'static str> {
    let shots = args
        .first()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(100)
        .clamp(1, 10_000);

    let guard = QUANTUM_STATE.lock();
    let Some(session) = guard.as_ref() else {
        kprintln!("No quantum register allocated. Use 'qalloc <n>' first.");
        return Ok(());
    };
    let state = &session.state;

    let mut rng = Xorshift64::new(
        crate::arch::x86_64::interrupts::tick_count()
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .wrapping_add(state.num_qubits as u64),
    );
    let results = measure_all(state, shots, &mut rng);
    display_results(&results, state.num_qubits, shots);
    drop(guard);
    if let Some(session) = QUANTUM_STATE.lock().as_mut() {
        session.record_measurement(&results, shots);
    }
    Ok(())
}

fn show_status() {
    kprint_colored!(Colors::PURPLE, "WarOS Quantum Subsystem\n");
    kprintln!("----------------------------------------");
    kprintln!("  Backend:     Kernel StateVector Simulator");
    kprintln!("  Max qubits:  18 (kernel heap limited)");
    kprintln!("  Gates:       H, X, Y, Z, S, T, Rx, Ry, Rz, CNOT, CZ, SWAP, CCX");
    kprintln!("  PRNG:        Xorshift64 (PIT-seeded)");
    kprintln!("  Demos:       bell, ghz3, grover, teleport, qft4, deutsch, bernstein, superdense, shor, vqe, qaoa");
    kprintln!("  Persistence: qsave, qexport, qresult");

    let guard = QUANTUM_STATE.lock();
    if let Some(session) = guard.as_ref() {
        kprintln!(
            "  Active reg:  {} qubits ({} amplitudes, {} bytes)",
            session.state.num_qubits,
            session.state.dimension(),
            session.state.bytes_used()
        );
        kprintln!("  Norm:        {:.6}", session.state.total_probability());
    } else {
        kprintln!("  Active reg:  none");
    }
}

fn cmd_qsave(args: &[&str]) -> Result<(), &'static str> {
    let Some(raw_name) = args.first().copied() else {
        kprintln!("Usage: qsave <name>");
        return Ok(());
    };

    let guard = QUANTUM_STATE.lock();
    let Some(session) = guard.as_ref() else {
        kprintln!("No quantum register allocated. Use 'qalloc <n>' first.");
        return Ok(());
    };

    let filename = with_extension(raw_name, ".qasm");
    let qasm = session.qasm_source();
    let saved_path = fs::write_current(&filename, qasm.as_bytes()).map_err(map_fs_error)?;
    kprint_colored!(Colors::GREEN, "Saved circuit to ");
    kprintln!("'{}' ({} bytes)", saved_path, qasm.len());
    Ok(())
}

fn cmd_qresult(args: &[&str]) -> Result<(), &'static str> {
    let Some(raw_name) = args.first().copied() else {
        kprintln!("Usage: qresult <name>");
        return Ok(());
    };

    let guard = QUANTUM_STATE.lock();
    let Some(session) = guard.as_ref() else {
        kprintln!("No quantum register allocated. Use 'qalloc <n>' first.");
        return Ok(());
    };
    let Some(result_text) = session.last_result_text() else {
        kprintln!("No measurement results available. Run 'qmeasure' first.");
        return Ok(());
    };

    let filename = with_extension(raw_name, ".txt");
    let saved_path = fs::write_current(&filename, result_text.as_bytes()).map_err(map_fs_error)?;
    kprint_colored!(Colors::GREEN, "Saved measurement results to ");
    kprintln!("'{}' ({} bytes)", saved_path, result_text.len());
    Ok(())
}

fn parse_qubit(args: &[&str], index: usize) -> Result<usize, &'static str> {
    args.get(index)
        .and_then(|value| value.parse::<usize>().ok())
        .ok_or("Missing or invalid qubit index")
}

fn parse_angle(args: &[&str], index: usize) -> Result<f64, &'static str> {
    let token = args
        .get(index)
        .copied()
        .ok_or("Missing angle (use radians, e.g. 1.5708 or pi/2)")?;
    parse_angle_expression(token)
        .ok_or("Invalid angle expression (use radians, pi, pi/2, 2pi, etc.)")
}

fn parse_angle_expression(token: &str) -> Option<f64> {
    if let Ok(value) = token.parse::<f64>() {
        return Some(value);
    }

    let mut normalized = String::with_capacity(token.len());
    for character in token.chars() {
        if character != '*' && character != ' ' {
            normalized.push(character);
        }
    }

    if let Ok(value) = normalized.parse::<f64>() {
        return Some(value);
    }

    let value = normalized.as_str();
    let pi_index = value.find("pi")?;
    let (coefficient_text, suffix) = value.split_at(pi_index);
    let coefficient = match coefficient_text {
        "" | "+" => 1.0,
        "-" => -1.0,
        other => other.parse::<f64>().ok()?,
    };

    let suffix = &suffix[2..];
    let mut angle = coefficient * PI;
    if suffix.is_empty() {
        return Some(angle);
    }

    if let Some(denominator) = suffix.strip_prefix('/') {
        angle /= denominator.parse::<f64>().ok()?;
        return Some(angle);
    }

    if let Some(multiplier) = suffix
        .strip_prefix('x')
        .or_else(|| suffix.strip_prefix('X'))
    {
        angle *= multiplier.parse::<f64>().ok()?;
        return Some(angle);
    }

    None
}

fn announce(prefix: &str, qubit: usize) {
    kprint_colored!(Colors::GREEN, "{} ", prefix);
    kprintln!("{}", qubit);
}

fn with_extension(name: &str, extension: &str) -> String {
    let trimmed = name.trim().trim_start_matches('/');
    if trimmed.ends_with(extension) {
        trimmed.to_string()
    } else {
        format!("{trimmed}{extension}")
    }
}

fn map_fs_error(error: fs::FsError) -> &'static str {
    match error {
        fs::FsError::FileNotFound => "file not found",
        fs::FsError::FilesystemFull => "filesystem full",
        fs::FsError::FilenameTooLong => "filename too long",
        fs::FsError::FileTooLarge => "file too large",
        fs::FsError::ReadOnly => "file is read-only",
        fs::FsError::InvalidFilename => "invalid filename",
        fs::FsError::PermissionDenied => "permission denied",
    }
}
