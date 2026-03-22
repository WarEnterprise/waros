"""Python package facade for the WarOS native extension."""

from __future__ import annotations

import importlib.util
import site
import sys
from pathlib import Path
from types import SimpleNamespace


def _load_native():
    try:
        from . import waros as native  # type: ignore[attr-defined]
        return native
    except ImportError:
        for base in _site_package_roots():
            candidate = base / "waros" / "waros.pyd"
            if candidate.exists():
                spec = importlib.util.spec_from_file_location("waros.waros", candidate)
                if spec is None or spec.loader is None:
                    continue
                native = importlib.util.module_from_spec(spec)
                sys.modules.setdefault("waros.waros", native)
                spec.loader.exec_module(native)
                return native
        raise


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

algorithms = SimpleNamespace(
    phase_estimation=phase_estimation,
    shor_factor=shor_factor,
    vqe_hydrogen=vqe_hydrogen,
    qaoa_maxcut=qaoa_maxcut,
    simon_hidden_xor=simon_hidden_xor,
    random_walk=random_walk,
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
