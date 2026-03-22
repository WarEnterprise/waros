# WarOS — Quantum Computing & Post-Quantum Cryptography for Python

High-performance quantum circuit simulation and post-quantum cryptography, powered by Rust.

## Install

```bash
pip install waros
```

## Quick Start

```python
import waros

circuit = waros.Circuit(2)
circuit.h(0)
circuit.cnot(0, 1)
circuit.measure_all()

simulator = waros.Simulator(seed=42)
result = simulator.run(circuit, shots=10_000)
print(result)
```

## Realistic Noise

```python
noise = waros.NoiseModel.ibm()
simulator = waros.Simulator(seed=42, noise=noise)
result = simulator.run(circuit, shots=10_000)
result.histogram()
```

## Post-Quantum Cryptography

```python
from waros import crypto

pk, sk = crypto.kem_keygen()
ct, shared_secret_a = crypto.kem_encapsulate(pk)
shared_secret_b = crypto.kem_decapsulate(sk, ct)
assert shared_secret_a == shared_secret_b

pk, sk = crypto.sign_keygen()
signature = crypto.sign(sk, b"Hello WarOS")
assert crypto.verify(pk, b"Hello WarOS", signature)

digest = crypto.sha3_256(b"WarOS")
random_data = crypto.random_bytes(32)
```

## OpenQASM Support

```python
qasm_source = """
OPENQASM 2.0;
include "qelib1.inc";
qreg q[2];
creg c[2];
h q[0];
cx q[0], q[1];
measure q[0] -> c[0];
measure q[1] -> c[1];
"""

circuit = waros.parse_qasm(qasm_source)
result = waros.Simulator(seed=42).run(circuit, shots=1_000)
```

Part of the [WarOS](https://github.com/WarEnterprise/waros) hybrid quantum-classical operating system project.
