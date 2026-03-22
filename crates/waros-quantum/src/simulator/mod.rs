use std::collections::HashMap;

use crate::circuit::{Circuit, Instruction};
use crate::complex::Complex;
use crate::error::{WarosError, WarosResult};
use crate::noise::NoiseModel;
use crate::result::QuantumResult;

mod mps;
mod statevector;
mod trajectory;

pub use self::mps::MPSSimulator;

/// Simulation backend selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Backend {
    /// State-vector simulation on a classical CPU.
    #[default]
    StateVector,
    /// Matrix-product-state simulation with the requested bond-dimension cap.
    MPS { max_bond_dim: usize },
    /// Choose a backend automatically from the circuit width.
    Auto,
}

/// Internal storage layout for the `StateVector` backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StateVectorLayout {
    /// Array-of-structs complex amplitudes.
    #[default]
    AoS,
    /// Struct-of-arrays real/imaginary amplitudes for SIMD-friendly loops.
    SoA,
}

/// Builder for configuring a [`Simulator`].
#[must_use]
#[derive(Debug, Clone)]
pub struct SimulatorBuilder {
    seed: Option<u64>,
    parallel: bool,
    backend: Backend,
    statevector_layout: StateVectorLayout,
    noise: NoiseModel,
}

impl SimulatorBuilder {
    fn new() -> Self {
        Self {
            seed: None,
            parallel: true,
            backend: Backend::StateVector,
            statevector_layout: StateVectorLayout::AoS,
            noise: NoiseModel::ideal(),
        }
    }

    /// Set a deterministic RNG seed.
    pub fn seed(mut self, seed: u64) -> Self {
        self.seed = Some(seed);
        self
    }

    /// Enable or disable Rayon-backed gate application for large circuits.
    pub fn parallel(mut self, parallel: bool) -> Self {
        self.parallel = parallel;
        self
    }

    /// Select the simulation backend.
    pub fn backend(mut self, backend: Backend) -> Self {
        self.backend = backend;
        self
    }

    /// Select the storage layout used by the `StateVector` backend.
    pub fn statevector_layout(mut self, layout: StateVectorLayout) -> Self {
        self.statevector_layout = layout;
        self
    }

    /// Enable a noise model for Monte Carlo trajectory simulation.
    pub fn noise(mut self, noise: NoiseModel) -> Self {
        self.noise = noise;
        self
    }

    /// Build the configured simulator.
    #[must_use]
    pub fn build(self) -> Simulator {
        Simulator {
            seed: self.seed,
            parallel: self.parallel,
            backend: self.backend,
            statevector_layout: self.statevector_layout,
            noise: self.noise,
        }
    }
}

/// State-vector quantum simulator.
///
/// ```rust
/// use waros_quantum::{Circuit, Simulator, WarosError};
///
/// # fn main() -> Result<(), WarosError> {
/// let mut c = Circuit::new(2)?;
/// c.h(0)?;
/// c.cnot(0, 1)?;
/// c.measure_all()?;
/// let result = Simulator::new().run(&c, 1000)?;
/// assert!(result.probability("00") > 0.4);
/// # Ok(())
/// # }
/// ```
pub struct Simulator {
    seed: Option<u64>,
    parallel: bool,
    backend: Backend,
    statevector_layout: StateVectorLayout,
    noise: NoiseModel,
}

impl Simulator {
    /// Create a builder for configuring a simulator instance.
    pub fn builder() -> SimulatorBuilder {
        SimulatorBuilder::new()
    }

    /// Create a simulator with entropy-backed randomness.
    #[must_use]
    pub fn new() -> Self {
        Self::builder().build()
    }

    /// Create a simulator with deterministic randomness.
    #[must_use]
    pub fn with_seed(seed: u64) -> Self {
        Self::builder().seed(seed).build()
    }

    /// Return the configured noise model.
    #[must_use]
    pub fn noise_model(&self) -> &NoiseModel {
        &self.noise
    }

    /// Execute a circuit for the requested number of shots.
    ///
    /// # Errors
    ///
    /// Returns an error if `shots` is zero or if the simulator cannot allocate
    /// or evolve the requested quantum state.
    pub fn run(&self, circuit: &Circuit, shots: u32) -> WarosResult<QuantumResult> {
        if shots == 0 {
            return Err(WarosError::InvalidShots(shots));
        }

        let backend = self.effective_backend(circuit.num_qubits());

        if matches!(backend, Backend::StateVector)
            && (!self.noise.is_ideal() || statevector::has_mid_circuit_measurement(circuit))
        {
            let mut rng = self.make_rng();
            return trajectory::run_shot_mode(
                circuit,
                shots,
                &mut rng,
                self.parallel_for(circuit.num_qubits()),
                (!self.noise.is_ideal()).then_some(&self.noise),
            );
        }

        match backend {
            Backend::StateVector => self.run_sample_mode(circuit, shots),
            Backend::MPS { max_bond_dim } => self.run_mps_mode(circuit, shots, max_bond_dim),
            Backend::Auto => unreachable!("effective_backend must resolve Auto"),
        }
    }

    /// Return the final state vector for a noiseless circuit without
    /// mid-circuit measurements.
    ///
    /// # Errors
    ///
    /// Returns an error if the circuit contains mid-circuit measurements,
    /// if a non-ideal noise model is configured, or if the simulator cannot
    /// allocate the requested state vector.
    pub fn statevector(&self, circuit: &Circuit) -> WarosResult<Vec<Complex>> {
        if !self.noise.is_ideal() {
            return Err(WarosError::SimulationError(
                "statevector is unavailable when noise is enabled".into(),
            ));
        }
        if statevector::has_mid_circuit_measurement(circuit) {
            return Err(WarosError::SimulationError(
                "statevector is unavailable for circuits with mid-circuit measurements".into(),
            ));
        }

        match self.effective_backend(circuit.num_qubits()) {
            Backend::StateVector => {
                let num_qubits = circuit.num_qubits();
                match self.statevector_layout {
                    StateVectorLayout::AoS => {
                        let mut state = statevector::zero_state(num_qubits)?;
                        statevector::apply_gate_sequence(
                            &mut state,
                            num_qubits,
                            circuit.instructions(),
                            self.parallel_for(num_qubits),
                        );
                        Ok(state)
                    }
                    StateVectorLayout::SoA => {
                        let mut state = statevector::zero_state_soa(num_qubits)?;
                        statevector::apply_gate_sequence_soa(
                            &mut state,
                            num_qubits,
                            circuit.instructions(),
                            self.parallel_for(num_qubits),
                        );
                        Ok(state.into_aos())
                    }
                }
            }
            Backend::MPS { max_bond_dim } => {
                let mut simulator = MPSSimulator::new(circuit.num_qubits(), max_bond_dim)?;
                simulator.apply_instructions(circuit.instructions())?;
                simulator.to_statevector()
            }
            Backend::Auto => unreachable!("effective_backend must resolve Auto"),
        }
    }

    fn run_sample_mode(&self, circuit: &Circuit, shots: u32) -> WarosResult<QuantumResult> {
        let num_qubits = circuit.num_qubits();
        let probabilities = match self.statevector_layout {
            StateVectorLayout::AoS => {
                let mut state = statevector::zero_state(num_qubits)?;
                statevector::apply_gate_sequence(
                    &mut state,
                    num_qubits,
                    circuit.instructions(),
                    self.parallel_for(num_qubits),
                );
                state
                    .iter()
                    .map(|amplitude| amplitude.norm_sq())
                    .collect::<Vec<_>>()
            }
            StateVectorLayout::SoA => {
                let mut state = statevector::zero_state_soa(num_qubits)?;
                statevector::apply_gate_sequence_soa(
                    &mut state,
                    num_qubits,
                    circuit.instructions(),
                    self.parallel_for(num_qubits),
                );
                state.probabilities()
            }
        };
        let measured_qubits: Vec<usize> = circuit
            .instructions()
            .iter()
            .filter_map(|instruction| match instruction {
                Instruction::Measure { qubit, .. } => Some(*qubit),
                Instruction::GateOp { .. }
                | Instruction::ConditionalGate { .. }
                | Instruction::Barrier { .. } => None,
            })
            .collect();

        let mut rng = self.make_rng();
        let mut counts: HashMap<String, u32> = HashMap::new();
        for _ in 0..shots {
            let sample_index = statevector::sample(&probabilities, &mut rng);
            let bitstring = if measured_qubits.is_empty() {
                statevector::basis_state_string(sample_index, num_qubits)
            } else {
                measured_qubits
                    .iter()
                    .map(|qubit| {
                        if (sample_index >> qubit) & 1 == 1 {
                            '1'
                        } else {
                            '0'
                        }
                    })
                    .collect()
            };
            *counts.entry(bitstring).or_insert(0) += 1;
        }

        let output_qubits = if measured_qubits.is_empty() {
            num_qubits
        } else {
            measured_qubits.len()
        };
        Ok(QuantumResult::new(output_qubits, shots, counts))
    }

    fn run_mps_mode(
        &self,
        circuit: &Circuit,
        shots: u32,
        max_bond_dim: usize,
    ) -> WarosResult<QuantumResult> {
        if !self.noise.is_ideal() {
            return Err(WarosError::SimulationError(
                "MPS backend does not yet support noise models".into(),
            ));
        }
        if statevector::has_mid_circuit_measurement(circuit) {
            return Err(WarosError::SimulationError(
                "MPS backend does not support mid-circuit measurements".into(),
            ));
        }

        let mut simulator = MPSSimulator::new(circuit.num_qubits(), max_bond_dim)?;
        simulator.apply_instructions(circuit.instructions())?;

        let measured_qubits: Vec<usize> = circuit
            .instructions()
            .iter()
            .filter_map(|instruction| match instruction {
                Instruction::Measure { qubit, .. } => Some(*qubit),
                Instruction::GateOp { .. }
                | Instruction::ConditionalGate { .. }
                | Instruction::Barrier { .. } => None,
            })
            .collect();

        let mut rng = self.make_rng();
        let mut counts: HashMap<String, u32> = HashMap::new();
        for (basis_state, count) in simulator.measure(shots as usize, &mut rng) {
            let bitstring = if measured_qubits.is_empty() {
                statevector::basis_state_string(basis_state, circuit.num_qubits())
            } else {
                measured_qubits
                    .iter()
                    .map(|qubit| {
                        if (basis_state >> qubit) & 1 == 1 {
                            '1'
                        } else {
                            '0'
                        }
                    })
                    .collect()
            };
            counts.insert(
                bitstring,
                u32::try_from(count).map_err(|_| {
                    WarosError::SimulationError("MPS measurement count overflowed u32".into())
                })?,
            );
        }

        let output_qubits = if measured_qubits.is_empty() {
            circuit.num_qubits()
        } else {
            measured_qubits.len()
        };
        Ok(QuantumResult::new(output_qubits, shots, counts))
    }

    pub(crate) fn make_rng(&self) -> rand::rngs::StdRng {
        use rand::SeedableRng;

        match self.seed {
            Some(seed) => rand::rngs::StdRng::seed_from_u64(seed),
            None => rand::rngs::StdRng::from_entropy(),
        }
    }

    fn parallel_for(&self, num_qubits: usize) -> bool {
        self.parallel && num_qubits >= statevector::PARALLEL_QUBIT_THRESHOLD
    }

    fn effective_backend(&self, num_qubits: usize) -> Backend {
        match self.backend {
            Backend::Auto if num_qubits > 20 => Backend::MPS { max_bond_dim: 64 },
            Backend::Auto => Backend::StateVector,
            backend => backend,
        }
    }
}

impl Default for Simulator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Circuit, NoiseChannel, NoiseModel};

    #[test]
    fn zero_state_measures_to_zero() {
        let mut circuit = Circuit::new(1).expect("valid circuit");
        circuit.measure(0).expect("valid measurement");
        let result = Simulator::with_seed(42)
            .run(&circuit, 1_000)
            .expect("simulation succeeds");
        assert!((result.probability("0") - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn bell_state_statistics_are_reproducible() {
        let mut circuit = Circuit::new(2).expect("valid circuit");
        circuit.h(0).expect("valid gate");
        circuit.cnot(0, 1).expect("valid gate");
        circuit.measure_all().expect("valid measurements");

        let first = Simulator::with_seed(123)
            .run(&circuit, 1_000)
            .expect("simulation succeeds");
        let second = Simulator::with_seed(123)
            .run(&circuit, 1_000)
            .expect("simulation succeeds");
        assert_eq!(first.counts(), second.counts());
    }

    #[test]
    fn statevector_rejects_mid_circuit_measurement() {
        let mut circuit = Circuit::new(2).expect("valid circuit");
        circuit.h(0).expect("valid gate");
        circuit.measure(0).expect("valid measurement");
        circuit.cnot(0, 1).expect("valid gate");

        let error = Simulator::new()
            .statevector(&circuit)
            .expect_err("mid-circuit measurements must be rejected");
        assert!(matches!(error, WarosError::SimulationError(_)));
    }

    #[test]
    fn statevector_rejects_noisy_simulator() {
        let circuit = Circuit::new(1).expect("valid circuit");
        let error = Simulator::builder()
            .noise(NoiseModel {
                single_qubit_noise: vec![NoiseChannel::BitFlip { probability: 0.1 }],
                ..NoiseModel::ideal()
            })
            .build()
            .statevector(&circuit)
            .expect_err("noisy statevector must be rejected");
        assert!(matches!(error, WarosError::SimulationError(_)));
    }

    #[test]
    fn builder_configures_seed_backend_and_noise() {
        let simulator = Simulator::builder()
            .seed(9)
            .parallel(false)
            .backend(Backend::StateVector)
            .statevector_layout(StateVectorLayout::SoA)
            .noise(NoiseModel::uniform(0.0, 0.0, 0.0))
            .build();

        let mut circuit = Circuit::new(1).expect("valid circuit");
        circuit.h(0).expect("valid gate");
        circuit.measure(0).expect("valid measurement");

        let first = simulator.run(&circuit, 256).expect("simulation succeeds");
        let second = Simulator::builder()
            .seed(9)
            .parallel(false)
            .backend(Backend::StateVector)
            .statevector_layout(StateVectorLayout::SoA)
            .noise(NoiseModel::uniform(0.0, 0.0, 0.0))
            .build()
            .run(&circuit, 256)
            .expect("simulation succeeds");
        assert_eq!(first.counts(), second.counts());
    }

    #[test]
    fn parallel_and_sequential_statevectors_match() {
        let mut circuit = Circuit::new(16).expect("valid circuit");
        for qubit in 0..16 {
            circuit.h(qubit).expect("valid gate");
        }
        for qubit in 0..15 {
            circuit.cnot(qubit, qubit + 1).expect("valid gate");
        }

        let sequential = Simulator::builder()
            .parallel(false)
            .build()
            .statevector(&circuit)
            .expect("statevector succeeds");
        let parallel = Simulator::builder()
            .parallel(true)
            .build()
            .statevector(&circuit)
            .expect("statevector succeeds");

        assert_eq!(sequential, parallel);
    }

    #[test]
    fn soa_and_aos_statevectors_match() {
        let mut circuit = Circuit::new(8).expect("valid circuit");
        for qubit in 0..8 {
            circuit.h(qubit).expect("valid gate");
        }
        for qubit in 0..7 {
            circuit.cnot(qubit, qubit + 1).expect("valid gate");
        }

        let aos = Simulator::builder()
            .statevector_layout(StateVectorLayout::AoS)
            .build()
            .statevector(&circuit)
            .expect("statevector succeeds");
        let soa = Simulator::builder()
            .statevector_layout(StateVectorLayout::SoA)
            .build()
            .statevector(&circuit)
            .expect("statevector succeeds");

        assert_eq!(aos, soa);
    }
}
