# WarOS Examples

## Quantum Computing

| Example | Location | Description | Qubits |
|---------|----------|-------------|--------|
| `bell_state` | `crates/waros-quantum/examples/bell_state.rs` | Bell-state entanglement | 2 |
| `grover_2bit` | `crates/waros-quantum/examples/grover_2bit.rs` | Grover's quantum search | 2 |
| `quantum_teleportation` | `crates/waros-quantum/examples/quantum_teleportation.rs` | State teleportation | 3 |
| `noise_simulation` | `crates/waros-quantum/examples/noise_simulation.rs` | Ideal vs noisy hardware | 5 |
| `shor_demo` | `crates/waros-quantum/examples/shor_demo.rs` | Shor's factoring demo (`N = 15`) | 12 |
| `vqe_demo` | `crates/waros-quantum/examples/vqe_demo.rs` | `H2` molecule ground-state search | 2 |
| `qaoa_demo` | `crates/waros-quantum/examples/qaoa_demo.rs` | `MaxCut` graph optimization | 4 |

## Cryptography

| Example | Location | Description |
|---------|----------|-------------|
| `pqc_demo` | `crates/waros-crypto/examples/pqc_demo.rs` | Post-quantum crypto showcase |

## Running

```bash
cargo run --example bell_state
cargo run --example shor_demo
cargo run --example vqe_demo
cargo run --example qaoa_demo
cargo run --example pqc_demo -p waros-crypto
```

