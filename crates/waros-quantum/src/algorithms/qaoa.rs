use crate::circuit::Circuit;
use crate::error::{WarosError, WarosResult};
use crate::simulator::Simulator;

/// Weighted graph used for MaxCut instances.
#[derive(Debug, Clone)]
pub struct Graph {
    pub num_vertices: usize,
    pub edges: Vec<(usize, usize, f64)>,
}

/// Result of a QAOA MaxCut optimization run.
#[derive(Debug, Clone)]
pub struct QAOAResult {
    pub best_solution: Vec<bool>,
    pub best_cost: f64,
    pub optimal_gamma: Vec<f64>,
    pub optimal_beta: Vec<f64>,
    pub approximation_ratio: Option<f64>,
    pub cost_history: Vec<f64>,
}

/// Run QAOA for a weighted MaxCut problem.
pub fn qaoa_maxcut(
    graph: &Graph,
    p: usize,
    max_iterations: usize,
    shots: u32,
    simulator: &Simulator,
) -> WarosResult<QAOAResult> {
    if p == 0 {
        return Err(WarosError::SimulationError(
            "QAOA requires at least one layer".into(),
        ));
    }
    if graph.num_vertices > 12 {
        return Err(WarosError::TooManyQubits(graph.num_vertices, 12));
    }

    let mut gamma = vec![0.5; p];
    let mut beta = vec![0.5; p];
    let mut cost_history = Vec::new();
    let mut best_cost = f64::NEG_INFINITY;
    let mut best_solution = vec![false; graph.num_vertices];
    let learning_rate = 0.2;
    let epsilon = 0.05;

    for _ in 0..max_iterations {
        let (cost, solution) = evaluate_qaoa(graph, &gamma, &beta, shots, simulator)?;
        cost_history.push(cost);
        if cost > best_cost {
            best_cost = cost;
            best_solution = solution;
        }

        let (grad_gamma, grad_beta) =
            numerical_gradient(graph, &gamma, &beta, epsilon, shots, simulator)?;

        let mut improved = false;
        for scale in [learning_rate, learning_rate * 0.5, learning_rate * 0.25] {
            let candidate_gamma: Vec<f64> = gamma
                .iter()
                .zip(grad_gamma.iter())
                .map(|(parameter, derivative)| parameter + scale * derivative)
                .collect();
            let candidate_beta: Vec<f64> = beta
                .iter()
                .zip(grad_beta.iter())
                .map(|(parameter, derivative)| parameter + scale * derivative)
                .collect();
            let (candidate_cost, candidate_solution) =
                evaluate_qaoa(graph, &candidate_gamma, &candidate_beta, shots, simulator)?;

            if candidate_cost > cost + 1e-10 {
                gamma = candidate_gamma;
                beta = candidate_beta;
                if candidate_cost > best_cost {
                    best_cost = candidate_cost;
                    best_solution = candidate_solution;
                }
                improved = true;
                break;
            }
        }

        if !improved {
            break;
        }
    }

    let optimal_cost = classical_maxcut(graph);
    Ok(QAOAResult {
        best_solution,
        best_cost,
        optimal_gamma: gamma,
        optimal_beta: beta,
        approximation_ratio: (optimal_cost > 0.0).then_some(best_cost / optimal_cost),
        cost_history,
    })
}

/// Exhaustively compute the optimal MaxCut value for a small graph.
#[must_use]
pub fn classical_maxcut(graph: &Graph) -> f64 {
    let mut best = f64::NEG_INFINITY;
    for mask in 0..(1usize << graph.num_vertices) {
        let assignment = bits_from_index(mask, graph.num_vertices);
        best = best.max(maxcut_cost(graph, &assignment));
    }
    best
}

impl Graph {
    #[must_use]
    pub fn triangle() -> Self {
        Self {
            num_vertices: 3,
            edges: vec![(0, 1, 1.0), (1, 2, 1.0), (0, 2, 1.0)],
        }
    }

    #[must_use]
    pub fn square() -> Self {
        Self {
            num_vertices: 4,
            edges: vec![(0, 1, 1.0), (1, 2, 1.0), (2, 3, 1.0), (3, 0, 1.0)],
        }
    }

    #[must_use]
    pub fn petersen_5() -> Self {
        Self {
            num_vertices: 5,
            edges: vec![
                (0, 1, 1.0),
                (1, 2, 1.0),
                (2, 3, 1.0),
                (3, 4, 1.0),
                (4, 0, 1.0),
            ],
        }
    }
}

fn evaluate_qaoa(
    graph: &Graph,
    gamma: &[f64],
    beta: &[f64],
    shots: u32,
    simulator: &Simulator,
) -> WarosResult<(f64, Vec<bool>)> {
    let mut circuit = Circuit::new(graph.num_vertices)?;

    for qubit in 0..graph.num_vertices {
        circuit.h(qubit)?;
    }

    for layer in 0..gamma.len() {
        for &(u, v, weight) in &graph.edges {
            circuit.cnot(u, v)?;
            circuit.rz(v, 2.0 * gamma[layer] * weight)?;
            circuit.cnot(u, v)?;
        }
        for qubit in 0..graph.num_vertices {
            circuit.rx(qubit, 2.0 * beta[layer])?;
        }
    }

    if simulator.noise_model().is_ideal() {
        let state = simulator.statevector(&circuit)?;
        let mut expected_cost = 0.0;
        let mut best_probability = -1.0;
        let mut best_solution = vec![false; graph.num_vertices];

        for (index, amplitude) in state.iter().enumerate() {
            let probability = amplitude.norm_sq();
            if probability <= 1e-12 {
                continue;
            }

            let assignment = bits_from_index(index, graph.num_vertices);
            let cost = maxcut_cost(graph, &assignment);
            expected_cost += probability * cost;

            if cost > maxcut_cost(graph, &best_solution)
                || (cost - maxcut_cost(graph, &best_solution)).abs() < 1e-12
                    && probability > best_probability
            {
                best_probability = probability;
                best_solution = assignment;
            }
        }

        Ok((expected_cost, best_solution))
    } else {
        let mut measured = circuit.clone();
        measured.measure_all()?;
        let result = simulator.run(&measured, shots.max(1))?;

        let mut expected_cost = 0.0;
        let mut best_cost = f64::NEG_INFINITY;
        let mut best_solution = vec![false; graph.num_vertices];

        for (state, count) in result.counts() {
            let assignment = state.chars().map(|bit| bit == '1').collect::<Vec<_>>();
            let cost = maxcut_cost(graph, &assignment);
            expected_cost += cost * f64::from(*count) / f64::from(result.total_shots());
            if cost > best_cost {
                best_cost = cost;
                best_solution = assignment;
            }
        }

        Ok((expected_cost, best_solution))
    }
}

fn numerical_gradient(
    graph: &Graph,
    gamma: &[f64],
    beta: &[f64],
    epsilon: f64,
    shots: u32,
    simulator: &Simulator,
) -> WarosResult<(Vec<f64>, Vec<f64>)> {
    let mut grad_gamma = vec![0.0; gamma.len()];
    let mut grad_beta = vec![0.0; beta.len()];

    for index in 0..gamma.len() {
        let mut gamma_plus = gamma.to_vec();
        let mut gamma_minus = gamma.to_vec();
        gamma_plus[index] += epsilon;
        gamma_minus[index] -= epsilon;
        let (cost_plus, _) = evaluate_qaoa(graph, &gamma_plus, beta, shots, simulator)?;
        let (cost_minus, _) = evaluate_qaoa(graph, &gamma_minus, beta, shots, simulator)?;
        grad_gamma[index] = (cost_plus - cost_minus) / (2.0 * epsilon);
    }

    for index in 0..beta.len() {
        let mut beta_plus = beta.to_vec();
        let mut beta_minus = beta.to_vec();
        beta_plus[index] += epsilon;
        beta_minus[index] -= epsilon;
        let (cost_plus, _) = evaluate_qaoa(graph, gamma, &beta_plus, shots, simulator)?;
        let (cost_minus, _) = evaluate_qaoa(graph, gamma, &beta_minus, shots, simulator)?;
        grad_beta[index] = (cost_plus - cost_minus) / (2.0 * epsilon);
    }

    Ok((grad_gamma, grad_beta))
}

/// Evaluate the weighted MaxCut cost for a bit assignment.
#[must_use]
pub fn maxcut_cost(graph: &Graph, assignment: &[bool]) -> f64 {
    graph
        .edges
        .iter()
        .map(|(u, v, weight)| {
            if assignment[*u] != assignment[*v] {
                *weight
            } else {
                0.0
            }
        })
        .sum()
}

fn bits_from_index(index: usize, width: usize) -> Vec<bool> {
    (0..width).map(|bit| ((index >> bit) & 1) == 1).collect()
}
