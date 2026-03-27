<p align="center">
  <img src="docs/assets/waros-logo.jpg" alt="WarOS logo" width="320">
</p>

# WarOS

[![CI](https://github.com/WarEnterprise/waros/actions/workflows/ci.yml/badge.svg)](https://github.com/WarEnterprise/waros/actions/workflows/ci.yml)
[![PyPI](https://img.shields.io/pypi/v/waros)](https://pypi.org/project/waros/)
[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)

WarOS is a research operating-system repository from War Enterprise.

Today the repository contains two concrete deliverables:

- a tested Rust/Python quantum and post-quantum cryptography SDK that runs on conventional hardware
- a bootable x86_64 `no_std` kernel prototype with WarFS, WarShell, a narrow WarExec userspace ABI, experimental networking, and WarShield Pass 1 hardening

`BLUEPRINT.md` is the architectural direction. It is not a claim that the full QRM/QHAL/QuantumIPC/Linux-compatibility design already ships.

## What WarOS Is Today

- `crates/waros-quantum`: implemented userspace statevector and MPS simulation, noise models, `OpenQASM` tooling, QEC helpers, and algorithm demos.
- `crates/waros-cli`: implemented CLI for local execution, circuit inspection, benchmarking, and IBM Quantum Runtime userspace flows.
- `crates/waros-python`: implemented Python bindings for the simulator, algorithms, QASM helpers, and crypto surfaces.
- `crates/waros-crypto`: implemented ML-KEM, ML-DSA, SLH-DSA, SHA-3/SHAKE, and simulated QRNG helpers.
- `kernel/`: bootable x86_64 kernel with framebuffer console, PS/2 shell, an 8 MiB kernel heap, WarFS RAM filesystem with virtio-blk persistence when present, DHCP/DNS/TCP/HTTP/TLS code paths, a kernel quantum simulator, and 12 CI-smoke-proven static ELF WarExec paths.
- `WarShield Pass 1`: integrated login/logout and file-mutation audit hooks, an outbound TCP firewall decision hook, ASLR on WarExec load paths, W^X enforcement on the WarExec loader path, and capability checks on selected shell and system operations.

## What WarOS Does Not Yet Claim

- no general Linux or POSIX compatibility layer, libc environment, `fork`, or dynamic-linking support
- no real QHAL, QRM, QuantumIPC, QuantumNet, or secure boot chain
- no release-grade kernel HTTPS trust model; kernel TLS encrypts traffic but does not validate server certificates
- no production package-signing chain; kernel package verification is still bootstrap digest-based
- no real quantum networking; the in-kernel BB84 path is a simulation, not a hardware-backed QKD link

The primary supported IBM Quantum hardware path remains userspace (`waros-quantum`, `waros-cli`, and `waros-python`). The kernel contains experimental HTTP/TLS/IBM client code, but it inherits the current kernel TLS limitation and should be treated as experimental.

## Repository Layout

- `kernel/`: standalone kernel crate, image tooling, and QEMU smoke path
- `crates/waros-quantum`: userspace quantum SDK
- `crates/waros-cli`: command-line tooling
- `crates/waros-python`: Python package bindings
- `crates/waros-crypto`: post-quantum cryptography helpers
- `docs/`: architecture audit, implementation matrix, current stage summary, and ABI notes

## Quick Start

Workspace:

```bash
git clone https://github.com/WarEnterprise/waros.git
cd waros
cargo test --workspace
cargo run -p waros-cli -- qstat
cargo run -p waros-cli -- run examples/qasm/bell.qasm --shots 128
```

Optional userspace IBM Runtime build:

```bash
cargo build -p waros-quantum --features ibm
```

Optional Python bindings:

```bash
cd crates/waros-python
maturin develop --release
python -c "import waros; print(waros.__version__)"
```

Kernel build on Windows:

```powershell
cd kernel
cargo +nightly build --release --target x86_64-unknown-none
.\tools\create_image.ps1
.\tools\run_qemu.ps1
```

Kernel build on Linux/macOS:

```bash
cd kernel
cargo +nightly build --release --target x86_64-unknown-none
./tools/create_image.sh
./tools/run_qemu.sh
```

Notes:

- `kernel/tools/create_image.*` produces BIOS and UEFI disk images under `kernel/target/`.
- `kernel/tools/run_qemu_pair.*` starts a two-node serial-link setup for `net send`, `net qsend`, and `ping` experiments.
- `help quantum`, `help fs`, and `security status` are the fastest ways to inspect the current kernel command surface.
- WarFS seeds `/readme.txt`, `/sysinfo.txt`, and the current WarExec smoke binaries at boot.

## Validation

Workspace validation:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -W clippy::all -W clippy::pedantic -A clippy::module_name_repetitions -A clippy::cast_possible_truncation
cargo doc --no-deps --workspace
```

Kernel validation:

```bash
cd kernel
cargo +nightly build --release --target x86_64-unknown-none
./tools/create_image.sh
sh ./tools/boot_smoke.sh
```

The current CI proves workspace build/test/clippy/doc generation, kernel build plus image creation, a headless BIOS kernel boot smoke, and Python binding tests.

## Documentation

- [BLUEPRINT.md](BLUEPRINT.md)
- [docs/POST_WARSHIELD_PASS1_STATUS.md](docs/POST_WARSHIELD_PASS1_STATUS.md)
- [docs/IMPLEMENTATION_STATUS_MATRIX.md](docs/IMPLEMENTATION_STATUS_MATRIX.md)
- [docs/ARCHITECTURE_AUDIT_MARCH_2026.md](docs/ARCHITECTURE_AUDIT_MARCH_2026.md)
- [docs/WAREXEC_MINIMAL_ABI.md](docs/WAREXEC_MINIMAL_ABI.md)
- [TRADEMARKS.md](TRADEMARKS.md)
- [CONTRIBUTING.md](CONTRIBUTING.md)

## License

Source code in this repository is licensed under Apache-2.0. The WarOS and War Enterprise names, logos, and brand assets are not granted for unrestricted reuse by the software license alone; see [TRADEMARKS.md](TRADEMARKS.md).
