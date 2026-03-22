# WarOS

WarOS is a hybrid quantum-classical operating system project from War Enterprise. The repository runs entirely on classical hardware today and provides a validated quantum SDK, realistic noise simulation, an `OpenQASM` toolchain, a CLI, and post-quantum cryptography primitives.

## Workspace

- `kernel/`
  Bare-metal `no_std` microkernel bootstrap for x86_64 using the `bootloader` crate, with a framebuffer console, serial debug output, GDT/IDT/PIC setup, a bitmap frame allocator, heap initialization, PS/2 keyboard input, a minimal `WarShell`, and a kernel-resident quantum simulator for interactive Bell/GHZ/Grover/teleportation demos.
- `crates/waros-quantum`
  State-vector simulator, circuit builder, QFT, Monte Carlo noise, `OpenQASM` 2.0 parser/serializer, examples, and benchmarks.
- `crates/waros-cli`
  Command-line interface for running QASM files, inspecting circuits, benchmarking, REPL usage, and simulated `qstat`.
- `crates/waros-crypto`
  ML-KEM, ML-DSA / SLH-DSA wrappers via `pqcrypto`, SHA-3 / SHAKE helpers, and a simulated QRNG backed by the quantum SDK.
- `crates/waros-python`
  PyO3 + maturin bindings exposing the quantum simulator, QASM utilities, noise models, and post-quantum cryptography to Python as `waros`.

## Quick Start

```bash
git clone https://github.com/warenterprise/waros.git
cd waros
cargo test --workspace
cargo run --example noise_simulation
cargo run --example pqc_demo
cargo run -p waros-cli -- qstat
cargo run -p waros-cli -- run examples/qasm/bell.qasm --shots 1000
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
- Set `WAROS_OVMF_PATH` on Windows or `OVMF_PATH` on Unix if the default OVMF firmware path does not exist.
- In the shell, `help quantum` lists the kernel quantum commands: `qalloc`, `qrun`, `qstate`, `qmeasure`, `qcircuit`, and `qinfo`.

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
- `OpenQASM` 2.0 parsing/serialization plus runnable QASM fixtures in [`examples/qasm`](examples/qasm).
- Post-quantum cryptography using maintained `pqcrypto` crates and SHA-3 / SHAKE.
- Python bindings via PyO3 and maturin with `Circuit`, `Simulator`, `NoiseModel`, `QuantumResult`, QASM helpers, and a `waros.crypto` submodule.
- Bootable x86_64 kernel bootstrap with framebuffer output, interrupt handling, memory initialization, PS/2 keyboard input, and a minimal interactive shell.
- Kernel-resident `no_std` quantum simulator with 15-qubit registers, shell-driven gate execution, state/probability inspection, histogram measurement, and built-in Bell/GHZ/Grover/teleport/QFT/Deutsch/Bernstein-Vazirani/superdense demos.

## Python SDK

```python
import waros
from waros import crypto

circuit = waros.Circuit(2)
circuit.h(0)
circuit.cnot(0, 1)
circuit.measure_all()

result = waros.Simulator(seed=42).run(circuit, shots=10_000)
print(result.counts)

public_key, secret_key = crypto.kem_keygen()
ciphertext, shared_secret_a = crypto.kem_encapsulate(public_key)
shared_secret_b = crypto.kem_decapsulate(secret_key, ciphertext)
assert shared_secret_a == shared_secret_b
```

## Validation

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -W clippy::all -W clippy::pedantic -A clippy::module_name_repetitions -A clippy::cast_possible_truncation
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
