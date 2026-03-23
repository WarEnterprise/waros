"""Python package facade for the WarOS native extension."""

from __future__ import annotations

import importlib
import importlib.machinery
import importlib.util
import site
import sys
from pathlib import Path
from types import ModuleType, SimpleNamespace


def _load_native() -> ModuleType:
    try:
        return importlib.import_module(f"{__name__}._waros")
    except ImportError as exc:
        for base in _site_package_roots():
            for suffix in importlib.machinery.EXTENSION_SUFFIXES:
                candidate = base / "waros" / f"_waros{suffix}"
                if not candidate.exists():
                    continue
                spec = importlib.util.spec_from_file_location(
                    f"{__name__}._waros",
                    candidate,
                )
                if spec is None or spec.loader is None:
                    continue
                native = importlib.util.module_from_spec(spec)
                sys.modules.setdefault(f"{__name__}._waros", native)
                spec.loader.exec_module(native)
                return native
        raise exc


def _site_package_roots() -> list[Path]:
    roots = [Path(path) for path in site.getsitepackages()]
    user_site = site.getusersitepackages()
    if isinstance(user_site, str):
        roots.append(Path(user_site))
    return roots


_native = _load_native()

Circuit = _native.Circuit
NoiseModel = _native.NoiseModel
QuantumResult = _native.QuantumResult
Simulator = _native.Simulator
__version__ = _native.__version__
crypto = _native.crypto
parse_qasm = _native.parse_qasm
phase_estimation = _native.phase_estimation
qaoa_maxcut = _native.qaoa_maxcut
random_walk = _native.random_walk
shor_factor = _native.shor_factor
simon_hidden_xor = _native.simon_hidden_xor
to_qasm = _native.to_qasm
vqe_hydrogen = _native.vqe_hydrogen


def run_bell_state(*, shots: int = 1000, seed: int | None = None):
    circuit = Circuit(2)
    circuit.h(0)
    circuit.cnot(0, 1)
    circuit.measure_all()
    return Simulator(seed=seed).run(circuit, shots=shots)


def run_grover(*, target: str = "11", shots: int = 1000, seed: int | None = None):
    if target != "11":
        raise ValueError("run_grover currently supports target='11' only")

    circuit = Circuit(2)
    circuit.h(0)
    circuit.h(1)
    circuit.cz(0, 1)
    circuit.h(0)
    circuit.h(1)
    circuit.x(0)
    circuit.x(1)
    circuit.cz(0, 1)
    circuit.x(0)
    circuit.x(1)
    circuit.h(0)
    circuit.h(1)
    circuit.measure_all()
    return Simulator(seed=seed).run(circuit, shots=shots)


def run_teleport(
    *,
    state_theta: float = 1.047,
    shots: int = 1000,
    seed: int | None = None,
):
    simulator = Simulator(seed=seed)
    circuit = Circuit(3)
    circuit.ry(0, state_theta)
    circuit.h(1)
    circuit.cnot(1, 2)
    circuit.cnot(0, 1)
    circuit.h(0)
    circuit.measure_into(0, 0)
    circuit.measure_into(1, 1)
    circuit.cnot(1, 2)
    circuit.cz(0, 2)
    circuit.measure_into(2, 2)
    return simulator.run(circuit, shots=shots)

algorithms = SimpleNamespace(
    phase_estimation=phase_estimation,
    shor_factor=shor_factor,
    vqe_hydrogen=vqe_hydrogen,
    qaoa_maxcut=qaoa_maxcut,
    simon_hidden_xor=simon_hidden_xor,
    random_walk=random_walk,
    run_bell_state=run_bell_state,
    run_grover=run_grover,
    run_teleport=run_teleport,
)

__all__ = [
    "Circuit",
    "NoiseModel",
    "QuantumResult",
    "Simulator",
    "__version__",
    "algorithms",
    "crypto",
    "parse_qasm",
    "to_qasm",
]
