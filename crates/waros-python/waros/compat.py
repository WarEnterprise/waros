"""Qiskit-style compatibility helpers for WarOS."""

from __future__ import annotations

from typing import Iterable, Sequence

from . import Circuit, Simulator


class QuantumCircuit:
    """Qiskit-style facade over :class:`waros.Circuit`.

    Parameters
    ----------
    num_qubits:
        Number of quantum wires.
    num_clbits:
        Number of classical bits. Stored for API compatibility.
    """

    def __init__(self, num_qubits: int, num_clbits: int = 0) -> None:
        self.num_qubits = num_qubits
        self.num_clbits = num_clbits
        self._circuit = Circuit(num_qubits)

    def h(self, qubit: int) -> None:
        self._circuit.h(qubit)

    def x(self, qubit: int) -> None:
        self._circuit.x(qubit)

    def y(self, qubit: int) -> None:
        self._circuit.y(qubit)

    def z(self, qubit: int) -> None:
        self._circuit.z(qubit)

    def s(self, qubit: int) -> None:
        self._circuit.s(qubit)

    def t(self, qubit: int) -> None:
        self._circuit.t(qubit)

    def rx(self, theta: float, qubit: int) -> None:
        self._circuit.rx(qubit, theta)

    def ry(self, theta: float, qubit: int) -> None:
        self._circuit.ry(qubit, theta)

    def rz(self, theta: float, qubit: int) -> None:
        self._circuit.rz(qubit, theta)

    def cx(self, control: int, target: int) -> None:
        self._circuit.cx(control, target)

    def cz(self, control: int, target: int) -> None:
        self._circuit.cz(control, target)

    def swap(self, q0: int, q1: int) -> None:
        self._circuit.swap(q0, q1)

    def measure(
        self,
        qubits: int | Sequence[int],
        classical_bits: int | Sequence[int],
    ) -> None:
        if isinstance(qubits, int) and isinstance(classical_bits, int):
            self._circuit.measure_into(qubits, classical_bits)
            return

        qubit_list = list(_as_sequence(qubits))
        classical_list = list(_as_sequence(classical_bits))
        if len(qubit_list) != len(classical_list):
            msg = "qubits and classical_bits must have the same length"
            raise ValueError(msg)
        for qubit, classical_bit in zip(qubit_list, classical_list, strict=True):
            self._circuit.measure_into(qubit, classical_bit)

    def measure_all(self) -> None:
        self._circuit.measure_all()

    def draw(self) -> str:
        return self._circuit.draw()

    def to_qasm(self) -> str:
        return self._circuit.to_qasm()

    def run(self, shots: int = 1000, simulator: Simulator | None = None):
        active_simulator = simulator if simulator is not None else Simulator()
        return active_simulator.run(self._circuit, shots=shots)

    def to_native(self) -> Circuit:
        return self._circuit.copy()


def _as_sequence(values: int | Sequence[int]) -> Iterable[int]:
    return [values] if isinstance(values, int) else values
