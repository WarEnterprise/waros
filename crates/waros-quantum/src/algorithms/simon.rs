use crate::circuit::Circuit;
use crate::error::{WarosError, WarosResult};
use crate::simulator::Simulator;

/// Result of Simon's hidden-XOR problem.
#[derive(Debug, Clone)]
pub struct SimonResult {
    pub secret: Vec<bool>,
    pub iterations_needed: usize,
}

/// Run Simon's algorithm for an `n`-bit oracle.
///
/// # Errors
///
/// Returns [`WarosError`] when `n` is zero or when the supplied oracle fails to
/// build a valid circuit.
pub fn simon_algorithm<F>(oracle: F, n: usize, simulator: &Simulator) -> WarosResult<SimonResult>
where
    F: Fn(&mut Circuit, usize, usize) -> WarosResult<()>,
{
    if n == 0 {
        return Err(WarosError::SimulationError(
            "Simon's algorithm requires at least one input qubit".into(),
        ));
    }

    let mut circuit = Circuit::new(2 * n)?;
    for qubit in 0..n {
        circuit.h(qubit)?;
    }
    oracle(&mut circuit, 0, n)?;
    for qubit in 0..n {
        circuit.h(qubit)?;
    }

    let mut equations = Vec::new();
    if simulator.noise_model().is_ideal() {
        let probabilities = input_register_probabilities(&circuit, n, simulator)?;
        let mut supported = probabilities
            .into_iter()
            .enumerate()
            .filter(|(value, probability)| *value != 0 && *probability > 1e-9)
            .collect::<Vec<_>>();
        supported.sort_by(|lhs, rhs| rhs.1.total_cmp(&lhs.1));

        for (value, _) in supported {
            let equation = bits_from_index(value, n);
            if increases_rank(&equations, &equation, n) {
                equations.push(equation);
            }
            if gf2_rank(&equations, n) >= n.saturating_sub(1) {
                break;
            }
        }
    } else {
        for qubit in 0..n {
            circuit.measure(qubit)?;
        }

        for _ in 0..(3 * n) {
            let result = simulator.run(&circuit, 1)?;
            let (state, _) = result.most_probable();
            let equation = bits_from_state(state, n);
            if equation.iter().any(|bit| *bit) && increases_rank(&equations, &equation, n) {
                equations.push(equation);
            }
            if gf2_rank(&equations, n) >= n.saturating_sub(1) {
                break;
            }
        }
    }

    Ok(SimonResult {
        secret: solve_gf2(&equations, n),
        iterations_needed: equations.len(),
    })
}

/// Build a compact hidden-XOR oracle with the property `f(x) = f(x xor s)`.
///
/// # Errors
///
/// Returns [`WarosError`] when the secret is the all-zero bitstring or when the
/// generated oracle contains invalid qubit references.
pub fn apply_hidden_xor_oracle(
    circuit: &mut Circuit,
    input_start: usize,
    output_start: usize,
    secret: &[bool],
) -> WarosResult<()> {
    let Some(pivot) = secret.iter().position(|bit| *bit) else {
        return Err(WarosError::SimulationError(
            "Simon oracle secret must be non-zero".into(),
        ));
    };

    for (bit, active) in secret.iter().copied().enumerate() {
        if bit == pivot {
            continue;
        }

        circuit.cnot(input_start + bit, output_start + bit)?;
        if active {
            circuit.cnot(input_start + pivot, output_start + bit)?;
        }
    }

    Ok(())
}

/// Solve the Simon equations `y_i · s = 0 (mod 2)` by brute force over `GF(2)`.
#[must_use]
pub fn solve_gf2(equations: &[Vec<bool>], width: usize) -> Vec<bool> {
    for candidate in 1usize..(1usize << width) {
        let bits = (0..width)
            .map(|bit| ((candidate >> bit) & 1) == 1)
            .collect::<Vec<_>>();

        if equations
            .iter()
            .all(|equation| !parity_dot(equation, &bits))
        {
            return bits;
        }
    }

    vec![false; width]
}

fn parity_dot(lhs: &[bool], rhs: &[bool]) -> bool {
    lhs.iter()
        .zip(rhs.iter())
        .fold(false, |parity, (left, right)| parity ^ (*left && *right))
}

fn input_register_probabilities(
    circuit: &Circuit,
    n: usize,
    simulator: &Simulator,
) -> WarosResult<Vec<f64>> {
    let state = simulator.statevector(circuit)?;
    let mut probabilities = vec![0.0; 1usize << n];
    let input_mask = (1usize << n) - 1;

    for (basis_index, amplitude) in state.iter().enumerate() {
        probabilities[basis_index & input_mask] += amplitude.norm_sq();
    }

    Ok(probabilities)
}

fn bits_from_state(state: &str, width: usize) -> Vec<bool> {
    state.chars().take(width).map(|bit| bit == '1').collect()
}

fn bits_from_index(index: usize, width: usize) -> Vec<bool> {
    (0..width).map(|bit| ((index >> bit) & 1) == 1).collect()
}

fn increases_rank(existing: &[Vec<bool>], candidate: &[bool], width: usize) -> bool {
    let previous = gf2_rank(existing, width);
    let mut augmented = existing.to_vec();
    augmented.push(candidate.to_vec());
    gf2_rank(&augmented, width) > previous
}

fn gf2_rank(equations: &[Vec<bool>], width: usize) -> usize {
    let mut rows = equations.to_vec();
    let mut rank = 0usize;

    for column in 0..width {
        let Some(pivot_row) = (rank..rows.len()).find(|row| rows[*row][column]) else {
            continue;
        };
        rows.swap(rank, pivot_row);

        for row in 0..rows.len() {
            if row != rank && rows[row][column] {
                let pivot_values = rows[rank][column..width].to_vec();
                for (destination, source) in
                    rows[row][column..width].iter_mut().zip(pivot_values.iter())
                {
                    *destination ^= *source;
                }
            }
        }

        rank += 1;
        if rank == rows.len() {
            break;
        }
    }

    rank
}
