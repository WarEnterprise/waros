"""Tests for waros Python bindings.

Run with:
    maturin develop --release
    pytest tests/test_waros.py -v
"""

from pathlib import Path
import sys

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import waros


def test_ibm_backend_is_exported():
    assert hasattr(waros, "IBMBackend")


class TestCircuit:
    def test_create(self):
        circuit = waros.Circuit(2)
        assert circuit.num_qubits == 2
        assert circuit.gate_count == 0

    def test_gates(self):
        circuit = waros.Circuit(3)
        circuit.h(0)
        circuit.x(1)
        circuit.cnot(0, 2)
        assert circuit.gate_count == 3

    def test_all_single_gates(self):
        circuit = waros.Circuit(1)
        circuit.h(0)
        circuit.x(0)
        circuit.y(0)
        circuit.z(0)
        circuit.s(0)
        circuit.sdg(0)
        circuit.t(0)
        circuit.tdg(0)
        circuit.rx(0, 1.0)
        circuit.ry(0, 1.0)
        circuit.rz(0, 1.0)
        circuit.sx(0)
        assert circuit.gate_count == 12

    def test_two_qubit_gates(self):
        circuit = waros.Circuit(3)
        circuit.cnot(0, 1)
        circuit.cz(0, 1)
        circuit.swap(0, 1)
        circuit.toffoli(0, 1, 2)
        assert circuit.gate_count > 3

    def test_qft(self):
        circuit = waros.Circuit(4)
        circuit.qft([0, 1, 2, 3])
        assert circuit.gate_count > 0

    def test_measure(self):
        circuit = waros.Circuit(2)
        circuit.h(0)
        circuit.measure_all()
        assert circuit.num_classical_bits == 2

    def test_depth(self):
        circuit = waros.Circuit(2)
        circuit.h(0)
        circuit.cnot(0, 1)
        assert circuit.depth >= 2

    def test_draw(self):
        circuit = waros.Circuit(2)
        circuit.h(0)
        circuit.cnot(0, 1)
        assert len(circuit.draw()) > 0

    def test_repr(self):
        circuit = waros.Circuit(3)
        assert "3 qubits" in repr(circuit)

    def test_len(self):
        circuit = waros.Circuit(2)
        circuit.h(0)
        circuit.h(1)
        assert len(circuit) == 2

    def test_qubit_out_of_range(self):
        circuit = waros.Circuit(2)
        with pytest.raises(ValueError):
            circuit.h(5)

    def test_cnot_same_qubit(self):
        circuit = waros.Circuit(2)
        with pytest.raises(ValueError):
            circuit.cnot(0, 0)

    def test_max_qubits(self):
        with pytest.raises(ValueError):
            waros.Circuit(129)


class TestSimulator:
    def test_basic(self):
        circuit = waros.Circuit(1)
        circuit.x(0)
        circuit.measure(0)
        result = waros.Simulator(seed=42).run(circuit, shots=100)
        assert result["1"] == 100

    def test_bell_state(self):
        circuit = waros.Circuit(2)
        circuit.h(0)
        circuit.cnot(0, 1)
        circuit.measure_all()
        result = waros.Simulator(seed=42).run(circuit, shots=10000)
        assert abs(result.probability("00") - 0.5) < 0.05
        assert abs(result.probability("11") - 0.5) < 0.05
        assert result.probability("01") < 0.01
        assert result.probability("10") < 0.01

    def test_reproducible(self):
        circuit = waros.Circuit(2)
        circuit.h(0)
        circuit.cnot(0, 1)
        circuit.measure_all()
        result_a = waros.Simulator(seed=123).run(circuit, shots=1000)
        result_b = waros.Simulator(seed=123).run(circuit, shots=1000)
        assert result_a.counts == result_b.counts

    def test_statevector(self):
        circuit = waros.Circuit(1)
        circuit.h(0)
        statevector = waros.Simulator().statevector(circuit)
        assert len(statevector) == 2
        assert abs(statevector[0][0] - 0.7071) < 0.001
        assert abs(statevector[1][0] - 0.7071) < 0.001

    def test_probabilities(self):
        circuit = waros.Circuit(1)
        circuit.h(0)
        probabilities = waros.Simulator().probabilities(circuit)
        assert len(probabilities) == 2
        assert abs(probabilities[0] - 0.5) < 1e-10
        assert abs(probabilities[1] - 0.5) < 1e-10

    def test_result_counts(self):
        circuit = waros.Circuit(1)
        circuit.x(0)
        circuit.measure(0)
        result = waros.Simulator(seed=1).run(circuit, shots=500)
        assert result.total_shots == 500
        assert result.counts == {"1": 500}

    def test_most_probable(self):
        circuit = waros.Circuit(1)
        circuit.x(0)
        circuit.measure(0)
        result = waros.Simulator(seed=1).run(circuit, shots=100)
        state, count = result.most_probable()
        assert state == "1"
        assert count == 100

    def test_histogram_data(self):
        circuit = waros.Circuit(1)
        circuit.h(0)
        circuit.measure(0)
        result = waros.Simulator(seed=1).run(circuit, shots=1000)
        histogram = result.histogram_data()
        assert len(histogram) == 2
        assert all(len(entry) == 3 for entry in histogram)

    def test_result_len(self):
        circuit = waros.Circuit(2)
        circuit.h(0)
        circuit.cnot(0, 1)
        circuit.measure_all()
        result = waros.Simulator(seed=42).run(circuit, shots=1000)
        assert len(result) == 2

    def test_result_repr(self):
        circuit = waros.Circuit(1)
        circuit.h(0)
        circuit.measure(0)
        result = waros.Simulator(seed=1).run(circuit, shots=100)
        assert "100 shots" in repr(result)


class TestNoise:
    def test_ideal(self):
        noise = waros.NoiseModel.ideal()
        assert noise is not None

    def test_ibm(self):
        noise = waros.NoiseModel.ibm()
        circuit = waros.Circuit(2)
        circuit.h(0)
        circuit.cnot(0, 1)
        circuit.measure_all()
        result = waros.Simulator(seed=42, noise=noise).run(circuit, shots=10000)
        assert result.probability("01") > 0 or result.probability("10") > 0

    def test_ionq(self):
        noise = waros.NoiseModel.ionq()
        assert noise is not None

    def test_uniform(self):
        noise = waros.NoiseModel.uniform(0.1, 0.1, 0.1)
        assert noise is not None


class TestQasm:
    def test_parse(self):
        qasm = """OPENQASM 2.0;
include "qelib1.inc";
qreg q[2];
creg c[2];
h q[0];
cx q[0], q[1];
measure q[0] -> c[0];
measure q[1] -> c[1];
"""
        circuit = waros.parse_qasm(qasm)
        assert circuit.num_qubits == 2
        assert circuit.gate_count == 2

    def test_roundtrip(self):
        circuit = waros.Circuit(2)
        circuit.h(0)
        circuit.cnot(0, 1)
        circuit.measure_all()
        qasm = circuit.to_qasm()
        assert "OPENQASM" in qasm
        assert "h q[0]" in qasm

    def test_parse_error(self):
        with pytest.raises(ValueError):
            waros.parse_qasm("garbage input")

    def test_parse_and_run(self):
        qasm = """OPENQASM 2.0;
include "qelib1.inc";
qreg q[1];
creg c[1];
x q[0];
measure q[0] -> c[0];
"""
        circuit = waros.parse_qasm(qasm)
        result = waros.Simulator(seed=42).run(circuit, shots=100)
        assert result["1"] == 100


class TestCrypto:
    def test_kem_roundtrip(self):
        public_key, secret_key = waros.crypto.kem_keygen()
        ciphertext, shared_secret_a = waros.crypto.kem_encapsulate(public_key)
        shared_secret_b = waros.crypto.kem_decapsulate(secret_key, ciphertext)
        assert shared_secret_a == shared_secret_b

    def test_sign_verify(self):
        public_key, secret_key = waros.crypto.sign_keygen()
        message = b"Hello WarOS"
        signature = waros.crypto.sign(secret_key, message)
        assert waros.crypto.verify(public_key, message, signature)

    def test_sign_verify_wrong_message(self):
        public_key, secret_key = waros.crypto.sign_keygen()
        signature = waros.crypto.sign(secret_key, b"Hello")
        assert not waros.crypto.verify(public_key, b"World", signature)

    def test_sha3_256(self):
        digest = waros.crypto.sha3_256(b"waros")
        assert len(digest) == 32

    def test_sha3_512(self):
        digest = waros.crypto.sha3_512(b"waros")
        assert len(digest) == 64

    def test_shake256(self):
        digest = waros.crypto.shake256(b"waros", 64)
        assert len(digest) == 64

    def test_random_bytes(self):
        first = waros.crypto.random_bytes(32)
        second = waros.crypto.random_bytes(32)
        assert len(first) == 32
        assert first != second

    def test_version(self):
        assert hasattr(waros, "__version__")


class TestAlgorithms:
    def test_shor_factor(self):
        result = waros.algorithms.shor_factor(15, seed=42)
        assert sorted(result["factors"]) == [3, 5]
        assert result["period"] == 4
        assert result["attempts"] >= 1

    def test_vqe_hydrogen(self):
        result = waros.algorithms.vqe_hydrogen(max_iterations=10, shots=500, seed=42)
        assert result["energy"] < -1.0
        assert len(result["params"]) == 2
        assert result["iterations"] >= 1

    def test_qaoa_maxcut_from_edges(self):
        result = waros.algorithms.qaoa_maxcut(
            [(0, 1), (1, 2), (2, 3), (3, 0)],
            4,
            p=2,
            max_iterations=10,
            shots=500,
            seed=42,
        )
        assert set(result["solution"]).issubset({"0", "1"})
        assert result["cost"] >= 2.0

    def test_phase_estimation(self):
        result = waros.algorithms.phase_estimation("t", precision_bits=3, shots=128, seed=42)
        assert abs(result["phase"] - 0.125) < 0.01

    def test_random_walk(self):
        result = waros.algorithms.random_walk(6)
        assert abs(sum(result["probabilities"]) - 1.0) < 1e-9


class TestCompat:
    def test_qiskit_style_quantum_circuit(self):
        from waros.compat import QuantumCircuit

        circuit = QuantumCircuit(2, 2)
        circuit.h(0)
        circuit.cx(0, 1)
        circuit.measure([0, 1], [0, 1])

        result = circuit.run(shots=1000)
        assert abs(result.probability("00") - 0.5) < 0.1
        assert abs(result.probability("11") - 0.5) < 0.1
