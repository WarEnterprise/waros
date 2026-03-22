use pyo3::prelude::*;
use pyo3::types::{PyDict, PyModule};

use waros_quantum::algorithms::{
    apply_hidden_xor_oracle, qaoa_maxcut, quantum_phase_estimation, quantum_random_walk,
    shor_factor, simon_algorithm, vqe, AnsatzType, Graph, Hamiltonian,
};
use waros_quantum::{gate, Circuit, Simulator};

use crate::value_error;

pub fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(phase_estimation, module)?)?;
    module.add_function(wrap_pyfunction!(shor_factor_py, module)?)?;
    module.add_function(wrap_pyfunction!(vqe_hydrogen, module)?)?;
    module.add_function(wrap_pyfunction!(qaoa_maxcut_py, module)?)?;
    module.add_function(wrap_pyfunction!(simon_hidden_xor, module)?)?;
    module.add_function(wrap_pyfunction!(random_walk, module)?)?;
    Ok(())
}

#[pyfunction(name = "phase_estimation")]
#[pyo3(signature = (unitary, eigenstate=None, precision_bits=3, shots=1024, seed=None))]
fn phase_estimation(
    py: Python<'_>,
    unitary: &str,
    eigenstate: Option<&str>,
    precision_bits: usize,
    shots: u32,
    seed: Option<u64>,
) -> PyResult<Py<PyDict>> {
    let simulator = seeded_simulator(seed);
    let target_qubit = precision_bits;
    let (gate, prep) = phase_estimation_inputs(unitary, eigenstate)?;
    let result = quantum_phase_estimation(
        &gate,
        |circuit| prep(circuit, target_qubit),
        precision_bits,
        shots,
        &simulator,
    )
    .map_err(value_error)?;

    let dict = PyDict::new_bound(py);
    dict.set_item("phase", result.phase)?;
    dict.set_item("measured_value", result.measured_value)?;
    dict.set_item("precision_bits", result.precision_bits)?;
    dict.set_item("counts", result.counts)?;
    Ok(dict.unbind())
}

#[pyfunction(name = "shor_factor")]
#[pyo3(signature = (n, seed=None))]
fn shor_factor_py(py: Python<'_>, n: u64, seed: Option<u64>) -> PyResult<Py<PyDict>> {
    let simulator = seeded_simulator(seed);
    let result = shor_factor(n, &simulator).map_err(value_error)?;

    let dict = PyDict::new_bound(py);
    dict.set_item("n", result.n)?;
    dict.set_item("factors", vec![result.factors.0, result.factors.1])?;
    dict.set_item("base_a", result.base_a)?;
    dict.set_item("period_r", result.period_r)?;
    dict.set_item("attempts", result.attempts)?;
    dict.set_item("success", result.success)?;
    Ok(dict.unbind())
}

#[pyfunction]
#[pyo3(signature = (max_iterations=30, shots=1000, seed=None))]
fn vqe_hydrogen(
    py: Python<'_>,
    max_iterations: usize,
    shots: u32,
    seed: Option<u64>,
) -> PyResult<Py<PyDict>> {
    let simulator = seeded_simulator(seed);
    let result = vqe(
        &Hamiltonian::hydrogen_molecule(),
        AnsatzType::RyLinear,
        &[0.0, 0.0],
        max_iterations,
        shots,
        &simulator,
    )
    .map_err(value_error)?;

    let dict = PyDict::new_bound(py);
    dict.set_item("energy", result.energy)?;
    dict.set_item("optimal_params", result.optimal_params)?;
    dict.set_item("energy_history", result.energy_history)?;
    dict.set_item("iterations", result.iterations)?;
    dict.set_item("converged", result.converged)?;
    Ok(dict.unbind())
}

#[pyfunction(name = "qaoa_maxcut")]
#[pyo3(signature = (graph="square", p=2, max_iterations=20, shots=1000, seed=None))]
fn qaoa_maxcut_py(
    py: Python<'_>,
    graph: &str,
    p: usize,
    max_iterations: usize,
    shots: u32,
    seed: Option<u64>,
) -> PyResult<Py<PyDict>> {
    let simulator = seeded_simulator(seed);
    let graph = match graph {
        "triangle" => Graph::triangle(),
        "square" => Graph::square(),
        "petersen_5" | "petersen" => Graph::petersen_5(),
        other => {
            return Err(value_error(format!(
                "unknown graph '{other}', expected triangle, square, or petersen_5"
            )));
        }
    };

    let result = qaoa_maxcut(&graph, p, max_iterations, shots, &simulator).map_err(value_error)?;
    let best_solution = result
        .best_solution
        .iter()
        .map(|bit| if *bit { '1' } else { '0' })
        .collect::<String>();

    let dict = PyDict::new_bound(py);
    dict.set_item("best_solution", best_solution)?;
    dict.set_item("best_cost", result.best_cost)?;
    dict.set_item("optimal_gamma", result.optimal_gamma)?;
    dict.set_item("optimal_beta", result.optimal_beta)?;
    dict.set_item("approximation_ratio", result.approximation_ratio)?;
    dict.set_item("cost_history", result.cost_history)?;
    Ok(dict.unbind())
}

#[pyfunction]
#[pyo3(signature = (secret, seed=None))]
fn simon_hidden_xor(py: Python<'_>, secret: Vec<bool>, seed: Option<u64>) -> PyResult<Py<PyDict>> {
    if secret.is_empty() || secret.iter().all(|bit| !*bit) {
        return Err(value_error("Simon secret must be a non-zero bitstring"));
    }

    let simulator = seeded_simulator(seed);
    let result = simon_algorithm(
        |circuit: &mut Circuit, input_start, output_start| {
            apply_hidden_xor_oracle(circuit, input_start, output_start, &secret)
        },
        secret.len(),
        &simulator,
    )
    .map_err(value_error)?;

    let dict = PyDict::new_bound(py);
    dict.set_item("secret", result.secret)?;
    dict.set_item("iterations_needed", result.iterations_needed)?;
    Ok(dict.unbind())
}

#[pyfunction]
fn random_walk(py: Python<'_>, steps: usize) -> PyResult<Py<PyDict>> {
    let result = quantum_random_walk(steps);

    let dict = PyDict::new_bound(py);
    dict.set_item("positions", result.positions)?;
    dict.set_item("probabilities", result.probabilities)?;
    dict.set_item("variance", result.variance)?;
    Ok(dict.unbind())
}

fn seeded_simulator(seed: Option<u64>) -> Simulator {
    seed.map_or_else(Simulator::new, Simulator::with_seed)
}

fn phase_estimation_inputs(
    unitary: &str,
    eigenstate: Option<&str>,
) -> PyResult<(waros_quantum::gate::Gate, fn(&mut Circuit, usize))> {
    match (unitary, eigenstate.unwrap_or_default()) {
        ("identity", _) | ("i", _) => Ok((identity_gate(), |_circuit, _target| {})),
        ("z", "" | "one") => Ok((gate::z(), |circuit, target| {
            let _ = circuit.x(target);
        })),
        ("s", "" | "one") => Ok((gate::s(), |circuit, target| {
            let _ = circuit.x(target);
        })),
        ("t", "" | "one") => Ok((gate::t(), |circuit, target| {
            let _ = circuit.x(target);
        })),
        ("x", "plus") => Ok((gate::x(), |circuit, target| {
            let _ = circuit.h(target);
        })),
        ("x", "minus") => Ok((gate::x(), |circuit, target| {
            let _ = circuit.x(target);
            let _ = circuit.h(target);
        })),
        _ => Err(value_error(
            "unsupported phase-estimation unitary/eigenstate pair",
        )),
    }
}

fn identity_gate() -> waros_quantum::gate::Gate {
    waros_quantum::gate::Gate::single(
        "I",
        [
            [waros_quantum::Complex::ONE, waros_quantum::Complex::ZERO],
            [waros_quantum::Complex::ZERO, waros_quantum::Complex::ONE],
        ],
    )
}
