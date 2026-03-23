# WarOS

[![CI](https://github.com/WarEnterprise/waros/actions/workflows/ci.yml/badge.svg)](https://github.com/WarEnterprise/waros/actions/workflows/ci.yml)
[![PyPI](https://img.shields.io/pypi/v/waros)](https://pypi.org/project/waros/)
[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)

WarOS is a hybrid quantum-classical operating system project from War Enterprise. The repository runs entirely on classical hardware today and provides a validated quantum SDK, realistic noise simulation, an `OpenQASM` toolchain, a CLI, and post-quantum cryptography primitives.

## Workspace

- `kernel/`
  Bare-metal `no_std` kernel bootstrap for x86_64 using the `bootloader` crate, with a framebuffer console, serial debug output, GDT/IDT/PIC setup, a bitmap frame allocator, a 4 MiB kernel heap, WarFS in-memory files, cooperative background tasks, COM2 serial networking primitives, WarShell commands, and a kernel-resident quantum simulator.
- `crates/waros-quantum`
  State-vector and MPS simulators, circuit builder, QFT, Monte Carlo noise, QEC helpers, Qiskit-compatible `OpenQASM` import support, an optional IBM Quantum Runtime hardware backend, advanced algorithms (QPE, Shor, VQE, QAOA, Simon, random walk), examples, and benchmarks.
- `crates/waros-cli`
  Command-line interface for running QASM files, inspecting circuits, benchmarking, REPL usage, simulated `qstat`, and IBM Quantum Runtime login/backends/run/status/result flows.
- `crates/waros-crypto`
  ML-KEM, ML-DSA / SLH-DSA wrappers via `pqcrypto`, SHA-3 / SHAKE helpers, and a simulated QRNG backed by the quantum SDK.
- `crates/waros-python`
  PyO3 + maturin bindings exposing the quantum simulator, IBM Quantum Runtime access, QASM utilities, a Qiskit-style compatibility layer, advanced algorithms, noise models, and post-quantum cryptography to Python as `waros`.

## Quick Start

```bash
git clone https://github.com/warenterprise/waros.git
cd waros
cargo test --workspace
cargo run --example noise_simulation
cargo run --example shor_demo
cargo run --example vqe_demo
cargo run --example qaoa_demo
cargo run --example pqc_demo
cargo run -p waros-cli -- qstat
cargo run -p waros-cli -- run examples/qasm/bell.qasm --shots 1000
cargo build -p waros-quantum --features ibm
cd crates/waros-python
maturin develop --release
python -c "import waros; print(waros.__version__)"
```

## Kernel Quick Start

The kernel is intentionally kept outside the Cargo workspace because it uses a custom `no_std`
target and nightly-only build settings.

```powershell
cd kernel
cargo +nightly build --release --target x86_64-unknown-none
.\tools\create_image.ps1
.\tools\run_qemu.ps1
```

On Linux/macOS:

```bash
cd kernel
cargo +nightly build --release --target x86_64-unknown-none
./tools/create_image.sh
./tools/run_qemu.sh
```

Notes:

- `kernel/tools/create_image.*` produces `kernel/target/waros.img` (UEFI) and `kernel/target/waros-bios.img`.
- `kernel/tools/run_qemu.*` expects `qemu-system-x86_64` in `PATH`.
- `kernel/tools/run_qemu_pair.*` prints a two-node COM2 serial-link setup for `net send`, `net qsend`, and `ping` testing.
- Set `WAROS_OVMF_PATH` on Windows or `OVMF_PATH` on Unix if the default OVMF firmware path does not exist.
- In the shell, `help quantum` lists the kernel quantum commands: `qalloc`, `qrun`, `qstate`, `qmeasure`, `qcircuit`, `qsave`, `qexport`, `qresult`, and `qinfo`.
- WarFS system files are created automatically at boot: `/readme.txt` and `/sysinfo.txt`.

## Example

```rust
use waros_quantum::{Circuit, NoiseModel, Simulator, WarosError};

fn main() -> Result<(), WarosError> {
    let mut circuit = Circuit::new(2)?;
    circuit.h(0)?;
    circuit.cnot(0, 1)?;
    circuit.measure_all()?;

    let simulator = Simulator::builder()
        .seed(42)
        .noise(NoiseModel::ibm_like())
        .build();

    let result = simulator.run(&circuit, 10_000)?;
    result.print_histogram();
    Ok(())
}
```

## Current Capabilities

- Validated gate library with unitarity regression tests and normalization assertions.
- Shot-based execution and state-vector inspection.
- Built-in QFT / inverse QFT, circuit composition, depth analysis, and ASCII diagrams.
- Monte Carlo depolarizing, damping, phase, and readout noise profiles.
- Advanced algorithm demos and APIs for Quantum Phase Estimation, Shor factoring, VQE chemistry, QAOA MaxCut, Simon's algorithm, and quantum random walks.
- State-vector layout selection (`AoS` or `SoA`) plus an MPS backend for low-entanglement larger-qubit workloads.
- `OpenQASM` 2.0 parsing/serialization plus runnable QASM fixtures in [`examples/qasm`](examples/qasm).
- Qiskit-oriented import support including `u1`/`u2`/`u3`, custom gates, conditionals, and a Python compatibility wrapper.
- Feature-gated IBM Quantum Runtime hardware backend for Rust, Python, and CLI userspace execution.
- Quantum error-correction helpers with repetition-code and Steane-code circuit builders.
- Post-quantum cryptography using maintained `pqcrypto` crates and SHA-3 / SHAKE.
- Python bindings via PyO3 and maturin with `Circuit`, `Simulator`, `NoiseModel`, `QuantumResult`, QASM helpers, a `waros.crypto` submodule, and a `waros.algorithms` submodule.
- Python `IBMBackend` bindings with saved-credential helpers for IBM Quantum Runtime jobs.
- Python convenience helpers for circuit stats, notebook HTML rendering, and one-line Bell/Grover/teleport demos via `waros.algorithms`.
- Bootable x86_64 kernel bootstrap with framebuffer output, interrupt handling, memory initialization, PS/2 keyboard input, and a minimal interactive shell.
- Kernel-resident `no_std` quantum simulator with 18-qubit registers, shell-driven gate execution, state/probability inspection, histogram measurement, and built-in Bell/GHZ/Grover/teleport/QFT/Deutsch/Bernstein-Vazirani/superdense/Shor/VQE/QAOA demos.
- Kernel WarFS commands (`ls`, `cat`, `write`, `rm`, `touch`, `stat`, `df`), task commands (`tasks`, `spawn`, `kill`), and serial networking commands (`net status`, `net send`, `net qsend`, `net listen`, `ping`).

## Python SDK

```python
import waros
from waros import crypto

circuit = waros.Circuit(2)
circuit.h(0)
circuit.cnot(0, 1)
circuit.measure_all()
print(circuit.stats())
print(circuit.draw())

result = waros.Simulator(seed=42).run(circuit, shots=10_000)
print(result.counts)

public_key, secret_key = crypto.kem_keygen()
ciphertext, shared_secret_a = crypto.kem_encapsulate(public_key)
shared_secret_b = crypto.kem_decapsulate(secret_key, ciphertext)
assert shared_secret_a == shared_secret_b

shor = waros.algorithms.shor_factor(15, seed=42)
vqe = waros.algorithms.vqe_hydrogen(seed=42)
qaoa = waros.algorithms.qaoa_maxcut("square", seed=42)
bell = waros.algorithms.run_bell_state(shots=1_000, seed=42)
```

## IBM Quantum Hardware

IBM Quantum integration is userspace-only. The kernel remains simulation-only and does not perform HTTPS requests.

Current IBM Quantum Runtime authentication requires both an API key and a service CRN:

```bash
export WAROS_IBM_TOKEN="your-ibm-api-key"
export WAROS_IBM_INSTANCE_CRN="your-service-crn"

cargo run -p waros-cli -- ibm backends
cargo run -p waros-cli -- ibm run examples/qasm/bell.qasm --backend ibm_brisbane --shots 1000
cargo run -p waros-quantum --example ibm_real_hardware --features ibm
```

```python
import waros

circuit = waros.Circuit(2)
circuit.h(0)
circuit.cnot(0, 1)
circuit.measure_all()

ibm = waros.IBMBackend(token="your-ibm-api-key", instance_crn="your-service-crn")
print(ibm.backends())
print(ibm.run(circuit, shots=1000, backend="ibm_brisbane"))
```

## Validation

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -W clippy::all -W clippy::pedantic -A clippy::module_name_repetitions -A clippy::cast_possible_truncation
cargo doc --no-deps --workspace
cargo build --release --workspace
```

Kernel validation:

```powershell
cd kernel
cargo +nightly build --release --target x86_64-unknown-none
.\tools\create_image.ps1
```

## Documentation

- [BLUEPRINT.md](BLUEPRINT.md)
- [CONTRIBUTING.md](CONTRIBUTING.md)

## License

Apache-2.0
