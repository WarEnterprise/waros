use libm::{cos, sin};

use crate::quantum::state::Complex;

/// 1/sqrt(2), reused by Hadamard and T phase definitions.
pub const SQRT2_INV: f64 = 0.707_106_781_186_547_5;

/// Single-qubit gate represented as a 2x2 complex matrix.
pub struct Gate1Q {
    pub matrix: [[Complex; 2]; 2],
}

/// Two-qubit gate represented as a 4x4 complex matrix.
pub struct Gate2Q {
    pub matrix: [[Complex; 4]; 4],
}

/// Hadamard gate.
#[must_use]
pub fn hadamard() -> Gate1Q {
    Gate1Q {
        matrix: [
            [(SQRT2_INV, 0.0), (SQRT2_INV, 0.0)],
            [(SQRT2_INV, 0.0), (-SQRT2_INV, 0.0)],
        ],
    }
}

/// Pauli-X gate.
#[must_use]
pub fn pauli_x() -> Gate1Q {
    Gate1Q {
        matrix: [[(0.0, 0.0), (1.0, 0.0)], [(1.0, 0.0), (0.0, 0.0)]],
    }
}

/// Pauli-Y gate.
#[must_use]
pub fn pauli_y() -> Gate1Q {
    Gate1Q {
        matrix: [[(0.0, 0.0), (0.0, -1.0)], [(0.0, 1.0), (0.0, 0.0)]],
    }
}

/// Pauli-Z gate.
#[must_use]
pub fn pauli_z() -> Gate1Q {
    Gate1Q {
        matrix: [[(1.0, 0.0), (0.0, 0.0)], [(0.0, 0.0), (-1.0, 0.0)]],
    }
}

/// Phase gate S = sqrt(Z).
#[must_use]
pub fn s_gate() -> Gate1Q {
    Gate1Q {
        matrix: [[(1.0, 0.0), (0.0, 0.0)], [(0.0, 0.0), (0.0, 1.0)]],
    }
}

/// T gate = exp(i*pi/4) on `|1>`.
#[must_use]
pub fn t_gate() -> Gate1Q {
    Gate1Q {
        matrix: [
            [(1.0, 0.0), (0.0, 0.0)],
            [(0.0, 0.0), (SQRT2_INV, SQRT2_INV)],
        ],
    }
}

/// Rotation around the X axis.
#[must_use]
pub fn rx(theta: f64) -> Gate1Q {
    let cosine = cos(theta * 0.5);
    let sine = sin(theta * 0.5);
    Gate1Q {
        matrix: [[(cosine, 0.0), (0.0, -sine)], [(0.0, -sine), (cosine, 0.0)]],
    }
}

/// Rotation around the Y axis.
#[must_use]
pub fn ry(theta: f64) -> Gate1Q {
    let cosine = cos(theta * 0.5);
    let sine = sin(theta * 0.5);
    Gate1Q {
        matrix: [[(cosine, 0.0), (-sine, 0.0)], [(sine, 0.0), (cosine, 0.0)]],
    }
}

/// Rotation around the Z axis.
#[must_use]
pub fn rz(theta: f64) -> Gate1Q {
    let cosine = cos(theta * 0.5);
    let sine = sin(theta * 0.5);
    Gate1Q {
        matrix: [[(cosine, -sine), (0.0, 0.0)], [(0.0, 0.0), (cosine, sine)]],
    }
}

/// Controlled-NOT gate.
#[must_use]
pub fn cnot() -> Gate2Q {
    let zero = (0.0, 0.0);
    let one = (1.0, 0.0);
    Gate2Q {
        matrix: [
            [one, zero, zero, zero],
            [zero, one, zero, zero],
            [zero, zero, zero, one],
            [zero, zero, one, zero],
        ],
    }
}

/// Controlled-Z gate.
#[must_use]
pub fn cz() -> Gate2Q {
    let zero = (0.0, 0.0);
    let one = (1.0, 0.0);
    let minus_one = (-1.0, 0.0);
    Gate2Q {
        matrix: [
            [one, zero, zero, zero],
            [zero, one, zero, zero],
            [zero, zero, one, zero],
            [zero, zero, zero, minus_one],
        ],
    }
}

/// SWAP gate.
#[must_use]
pub fn swap() -> Gate2Q {
    let zero = (0.0, 0.0);
    let one = (1.0, 0.0);
    Gate2Q {
        matrix: [
            [one, zero, zero, zero],
            [zero, zero, one, zero],
            [zero, one, zero, zero],
            [zero, zero, zero, one],
        ],
    }
}

/// Controlled phase rotation `diag(1, 1, 1, e^(i*theta))`.
#[must_use]
pub fn controlled_phase(theta: f64) -> Gate2Q {
    let zero = (0.0, 0.0);
    let one = (1.0, 0.0);
    let phase = (cos(theta), sin(theta));
    Gate2Q {
        matrix: [
            [one, zero, zero, zero],
            [zero, one, zero, zero],
            [zero, zero, one, zero],
            [zero, zero, zero, phase],
        ],
    }
}
