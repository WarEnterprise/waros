use std::collections::HashMap;

use nalgebra::DMatrix;
use num_complex::Complex64;
use rand::Rng;

use crate::circuit::Instruction;
use crate::complex::Complex;
use crate::error::{WarosError, WarosResult};
use crate::gate::{self, Gate};

const SVD_EPSILON: f64 = 1e-12;
const STATEVECTOR_LIMIT: usize = 20;

/// Matrix-product-state simulator backend.
#[derive(Debug, Clone)]
pub struct MPSSimulator {
    num_qubits: usize,
    max_bond_dim: usize,
    tensors: Vec<MPSTensor>,
    truncation_error: f64,
}

#[derive(Debug, Clone)]
struct MPSTensor {
    bond_left: usize,
    bond_right: usize,
    data: Vec<Complex>,
}

impl MPSSimulator {
    /// Create a new `|0...0⟩` MPS with the requested bond-dimension cap.
    ///
    /// # Errors
    ///
    /// Returns an error if `num_qubits` is zero or if `max_bond_dim` is zero.
    pub fn new(num_qubits: usize, max_bond_dim: usize) -> WarosResult<Self> {
        if num_qubits == 0 {
            return Err(WarosError::ZeroQubits);
        }
        if max_bond_dim == 0 {
            return Err(WarosError::SimulationError(
                "MPS backend requires max_bond_dim >= 1".into(),
            ));
        }

        let tensors = (0..num_qubits).map(|_| MPSTensor::zero()).collect();
        Ok(Self {
            num_qubits,
            max_bond_dim,
            tensors,
            truncation_error: 0.0,
        })
    }

    /// Apply a circuit instruction stream to the MPS.
    ///
    /// # Errors
    ///
    /// Returns an error if the instruction stream contains unsupported
    /// operations or invalid qubit indices.
    pub fn apply_instructions(&mut self, instructions: &[Instruction]) -> WarosResult<()> {
        for instruction in instructions {
            match instruction {
                Instruction::GateOp { gate, targets } => {
                    match (gate.num_qubits, targets.as_slice()) {
                        (1, [target]) => self.apply_1q(*target, gate)?,
                        (2, [q0, q1]) => self.apply_2q(*q0, *q1, gate)?,
                        _ => {
                            return Err(WarosError::SimulationError(format!(
                                "MPS backend does not support {}-qubit gate '{}'",
                                gate.num_qubits, gate.name
                            )));
                        }
                    }
                }
                Instruction::ConditionalGate { .. } => {
                    return Err(WarosError::SimulationError(
                        "MPS backend does not support conditional gates".into(),
                    ));
                }
                Instruction::Barrier { .. } | Instruction::Measure { .. } => {}
            }
        }

        Ok(())
    }

    /// Apply a single-qubit gate.
    ///
    /// # Errors
    ///
    /// Returns an error if `target` is out of range or the gate width is wrong.
    pub fn apply_1q(&mut self, target: usize, gate: &Gate) -> WarosResult<()> {
        if target >= self.num_qubits {
            return Err(WarosError::QubitOutOfRange(target, self.num_qubits));
        }
        if gate.num_qubits != 1 {
            return Err(WarosError::SimulationError(format!(
                "expected 1-qubit gate, got '{}'",
                gate.name
            )));
        }

        let tensor = &mut self.tensors[target];
        let mut updated = vec![Complex::ZERO; tensor.data.len()];
        for left in 0..tensor.bond_left {
            for right in 0..tensor.bond_right {
                let a0 = tensor.get(left, 0, right);
                let a1 = tensor.get(left, 1, right);
                updated[tensor.index(left, 0, right)] = gate.get(0, 0) * a0 + gate.get(0, 1) * a1;
                updated[tensor.index(left, 1, right)] = gate.get(1, 0) * a0 + gate.get(1, 1) * a1;
            }
        }
        tensor.data = updated;
        Ok(())
    }

    /// Apply a two-qubit gate, swapping intermediate sites as needed.
    ///
    /// # Errors
    ///
    /// Returns an error if either qubit is invalid, repeated, or the gate width
    /// is unsupported.
    pub fn apply_2q(&mut self, q0: usize, q1: usize, gate: &Gate) -> WarosResult<()> {
        if q0 >= self.num_qubits {
            return Err(WarosError::QubitOutOfRange(q0, self.num_qubits));
        }
        if q1 >= self.num_qubits {
            return Err(WarosError::QubitOutOfRange(q1, self.num_qubits));
        }
        if q0 == q1 {
            return Err(WarosError::SameQubit(q0, q1));
        }
        if gate.num_qubits != 2 {
            return Err(WarosError::SimulationError(format!(
                "expected 2-qubit gate, got '{}'",
                gate.name
            )));
        }

        let swap_gate = gate::swap();
        if q0 < q1 {
            for site in ((q0 + 1)..q1).rev() {
                self.apply_adjacent_2q(site - 1, &swap_gate)?;
            }
            self.apply_adjacent_2q(q0, gate)?;
            for site in (q0 + 1)..q1 {
                self.apply_adjacent_2q(site - 1, &swap_gate)?;
            }
        } else {
            for site in ((q1 + 1)..q0).rev() {
                self.apply_adjacent_2q(site, &swap_gate)?;
            }
            self.apply_adjacent_2q(q1, &swapped_gate(gate))?;
            for site in (q1 + 1)..q0 {
                self.apply_adjacent_2q(site, &swap_gate)?;
            }
        }

        Ok(())
    }

    /// Sample measurement outcomes from the MPS.
    #[must_use]
    pub fn measure(&self, shots: usize, rng: &mut impl Rng) -> Vec<(usize, usize)> {
        let mut counts = HashMap::<usize, usize>::new();
        for _ in 0..shots {
            let basis = self.sample_basis_state(rng);
            *counts.entry(basis).or_insert(0) += 1;
        }

        let mut non_zero = counts.into_iter().collect::<Vec<_>>();
        non_zero.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
        non_zero
    }

    /// Return the probability of a basis state.
    #[must_use]
    pub fn probability(&self, basis_state: usize) -> f64 {
        amplitude_for_basis(&self.tensors, basis_state).map_or(0.0, Complex::norm_sq)
    }

    /// Convert the MPS back into a full state vector for small systems.
    ///
    /// # Errors
    ///
    /// Returns an error if `num_qubits > 20`.
    pub fn to_statevector(&self) -> WarosResult<Vec<Complex>> {
        if self.num_qubits > STATEVECTOR_LIMIT {
            return Err(WarosError::TooManyQubits(
                self.num_qubits,
                STATEVECTOR_LIMIT,
            ));
        }

        let dim = 1usize << self.num_qubits;
        Ok((0..dim)
            .map(|basis_state| {
                amplitude_for_basis(&self.tensors, basis_state).unwrap_or(Complex::ZERO)
            })
            .collect())
    }

    /// Return the bond dimensions between neighboring sites.
    #[must_use]
    pub fn bond_dimensions(&self) -> Vec<usize> {
        self.tensors
            .iter()
            .take(self.tensors.len().saturating_sub(1))
            .map(|tensor| tensor.bond_right)
            .collect()
    }

    /// Return the accumulated discarded singular-value weight.
    #[must_use]
    pub fn truncation_error(&self) -> f64 {
        self.truncation_error
    }

    fn apply_adjacent_2q(&mut self, left_site: usize, gate: &Gate) -> WarosResult<()> {
        let left = self.tensors[left_site].clone();
        let right = self.tensors[left_site + 1].clone();
        let mut theta = vec![Complex::ZERO; left.bond_left * 2 * 2 * right.bond_right];

        for bond_left in 0..left.bond_left {
            for physical_left in 0..2 {
                for physical_right in 0..2 {
                    for bond_right in 0..right.bond_right {
                        let mut amplitude = Complex::ZERO;
                        for shared_bond in 0..left.bond_right {
                            amplitude += left.get(bond_left, physical_left, shared_bond)
                                * right.get(shared_bond, physical_right, bond_right);
                        }
                        theta[theta_index(
                            right.bond_right,
                            bond_left,
                            physical_left,
                            physical_right,
                            bond_right,
                        )] = amplitude;
                    }
                }
            }
        }

        let mut matrix_data =
            vec![Complex64::new(0.0, 0.0); left.bond_left * 2 * 2 * right.bond_right];
        for bond_left in 0..left.bond_left {
            for bond_right in 0..right.bond_right {
                let inputs = [
                    theta[theta_index(right.bond_right, bond_left, 0, 0, bond_right)],
                    theta[theta_index(right.bond_right, bond_left, 0, 1, bond_right)],
                    theta[theta_index(right.bond_right, bond_left, 1, 0, bond_right)],
                    theta[theta_index(right.bond_right, bond_left, 1, 1, bond_right)],
                ];
                for row in 0..4 {
                    let updated = gate.get(row, 0) * inputs[0]
                        + gate.get(row, 1) * inputs[1]
                        + gate.get(row, 2) * inputs[2]
                        + gate.get(row, 3) * inputs[3];
                    let physical_left = usize::from(row >= 2);
                    let physical_right = row % 2;
                    let matrix_row = bond_left * 2 + physical_left;
                    let matrix_col = physical_right * right.bond_right + bond_right;
                    matrix_data[matrix_row * (2 * right.bond_right) + matrix_col] = to_na(updated);
                }
            }
        }

        let matrix =
            DMatrix::from_row_slice(left.bond_left * 2, 2 * right.bond_right, &matrix_data);
        let svd = matrix.svd(true, true);
        let u = svd
            .u
            .ok_or_else(|| WarosError::SimulationError("MPS SVD failed to compute U".into()))?;
        let v_t = svd
            .v_t
            .ok_or_else(|| WarosError::SimulationError("MPS SVD failed to compute V^T".into()))?;

        let mut kept_rank = svd
            .singular_values
            .iter()
            .take_while(|value| **value > SVD_EPSILON)
            .count()
            .max(1);
        kept_rank = kept_rank.min(self.max_bond_dim);

        self.truncation_error += svd
            .singular_values
            .iter()
            .skip(kept_rank)
            .map(|value| value * value)
            .sum::<f64>();

        let mut left_tensor = MPSTensor::new(left.bond_left, kept_rank);
        let mut right_tensor = MPSTensor::new(kept_rank, right.bond_right);

        for bond_left in 0..left.bond_left {
            for physical in 0..2 {
                let row = bond_left * 2 + physical;
                for bond in 0..kept_rank {
                    left_tensor.set(bond_left, physical, bond, from_na(u[(row, bond)]));
                }
            }
        }

        for bond in 0..kept_rank {
            let singular = svd.singular_values[bond];
            for physical in 0..2 {
                for bond_right in 0..right.bond_right {
                    let column = physical * right.bond_right + bond_right;
                    right_tensor.set(
                        bond,
                        physical,
                        bond_right,
                        from_na(v_t[(bond, column)]) * singular,
                    );
                }
            }
        }

        self.tensors[left_site] = left_tensor;
        self.tensors[left_site + 1] = right_tensor;
        Ok(())
    }

    fn sample_basis_state(&self, rng: &mut impl Rng) -> usize {
        let mut environment = vec![Complex::ONE];
        let mut basis_state = 0usize;

        for (site, tensor) in self.tensors.iter().enumerate() {
            let zero_branch = propagate_branch(&environment, tensor, 0);
            let one_branch = propagate_branch(&environment, tensor, 1);
            let probability_zero = branch_probability(&zero_branch);
            let probability_one = branch_probability(&one_branch);
            let threshold =
                probability_zero / (probability_zero + probability_one).max(SVD_EPSILON);

            if rng.gen::<f64>() < threshold {
                environment = normalize_branch(zero_branch, probability_zero);
            } else {
                basis_state |= 1usize << site;
                environment = normalize_branch(one_branch, probability_one);
            }
        }

        basis_state
    }
}

impl MPSTensor {
    fn zero() -> Self {
        Self {
            bond_left: 1,
            bond_right: 1,
            data: vec![Complex::ONE, Complex::ZERO],
        }
    }

    fn new(bond_left: usize, bond_right: usize) -> Self {
        Self {
            bond_left,
            bond_right,
            data: vec![Complex::ZERO; bond_left * 2 * bond_right],
        }
    }

    fn index(&self, bond_left: usize, physical: usize, bond_right: usize) -> usize {
        (bond_left * 2 + physical) * self.bond_right + bond_right
    }

    fn get(&self, bond_left: usize, physical: usize, bond_right: usize) -> Complex {
        self.data[self.index(bond_left, physical, bond_right)]
    }

    fn set(&mut self, bond_left: usize, physical: usize, bond_right: usize, value: Complex) {
        let index = self.index(bond_left, physical, bond_right);
        self.data[index] = value;
    }
}

fn theta_index(
    right_dim: usize,
    bond_left: usize,
    physical_left: usize,
    physical_right: usize,
    bond_right: usize,
) -> usize {
    (((bond_left * 2 + physical_left) * 2 + physical_right) * right_dim) + bond_right
}

fn propagate_branch(environment: &[Complex], tensor: &MPSTensor, physical: usize) -> Vec<Complex> {
    let mut branch = vec![Complex::ZERO; tensor.bond_right];
    for (bond_left, amplitude) in environment.iter().copied().enumerate() {
        for (bond_right, target) in branch.iter_mut().enumerate() {
            *target += amplitude * tensor.get(bond_left, physical, bond_right);
        }
    }
    branch
}

fn normalize_branch(branch: Vec<Complex>, probability: f64) -> Vec<Complex> {
    if probability <= SVD_EPSILON {
        return branch;
    }
    let scale = 1.0 / probability.sqrt();
    branch.into_iter().map(|value| value * scale).collect()
}

fn branch_probability(branch: &[Complex]) -> f64 {
    branch.iter().map(|value| value.norm_sq()).sum()
}

fn amplitude_for_basis(tensors: &[MPSTensor], basis_state: usize) -> Option<Complex> {
    let mut environment = vec![Complex::ONE];
    for (site, tensor) in tensors.iter().enumerate() {
        environment = propagate_branch(
            &environment,
            tensor,
            usize::from((basis_state >> site) & 1 == 1),
        );
    }
    environment.into_iter().next()
}

fn swapped_gate(gate: &Gate) -> Gate {
    let permutation = [0usize, 2, 1, 3];
    let mut matrix = [[Complex::ZERO; 4]; 4];
    for row in 0..4 {
        for col in 0..4 {
            matrix[row][col] = gate.get(permutation[row], permutation[col]);
        }
    }
    Gate::two_qubit(&format!("{}^swap", gate.name), matrix)
}

fn to_na(value: Complex) -> Complex64 {
    Complex64::new(value.re, value.im)
}

fn from_na(value: Complex64) -> Complex {
    Complex::new(value.re, value.im)
}
