use std::f64::consts::E;

/// Individual noise channel applied after specific operations.
#[derive(Debug, Clone, PartialEq)]
pub enum NoiseChannel {
    /// Depolarizing channel: apply a random Pauli error with probability `p`.
    Depolarizing { probability: f64 },
    /// Amplitude damping channel with decay probability `gamma`.
    AmplitudeDamping { gamma: f64 },
    /// Phase damping channel with dephasing factor `lambda`.
    PhaseDamping { lambda: f64 },
    /// Bit-flip channel with probability `p`.
    BitFlip { probability: f64 },
    /// Phase-flip channel with probability `p`.
    PhaseFlip { probability: f64 },
    /// Readout error applied to a classical measurement bit.
    ReadoutError { probability: f64 },
}

impl NoiseChannel {
    pub(crate) fn is_effective(&self) -> bool {
        match self {
            Self::Depolarizing { probability }
            | Self::BitFlip { probability }
            | Self::PhaseFlip { probability }
            | Self::ReadoutError { probability } => *probability > 0.0,
            Self::AmplitudeDamping { gamma } => *gamma > 0.0,
            Self::PhaseDamping { lambda } => *lambda > 0.0,
        }
    }
}

/// Noise model for realistic quantum simulation.
///
/// The current simulator uses quantum trajectories over the state-vector
/// backend, so each shot evolves independently when noise is enabled.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NoiseModel {
    /// Noise applied after every single-qubit gate.
    pub single_qubit_noise: Vec<NoiseChannel>,
    /// Noise applied after every two-qubit gate.
    pub two_qubit_noise: Vec<NoiseChannel>,
    /// Noise applied to measured classical outcomes.
    pub measurement_noise: Vec<NoiseChannel>,
    /// Noise applied on logical idle steps such as barriers.
    pub idle_noise: Vec<NoiseChannel>,
}

impl NoiseModel {
    /// Return an ideal, noiseless profile.
    #[must_use]
    pub fn ideal() -> Self {
        Self::default()
    }

    /// Return an IBM-like superconducting transmon profile.
    #[must_use]
    pub fn ibm_like() -> Self {
        Self::from_hardware(100.0, 80.0, 100.0, 0.999, 0.99, 0.98)
    }

    /// Return an IonQ-like trapped-ion profile.
    #[must_use]
    pub fn ionq_like() -> Self {
        Self::from_hardware(1_000_000.0, 1_000_000.0, 100_000.0, 0.9997, 0.995, 0.996)
    }

    /// Build a uniform depolarizing/readout model.
    #[must_use]
    pub fn uniform(single_q_error: f64, two_q_error: f64, readout_error: f64) -> Self {
        Self {
            single_qubit_noise: vec![NoiseChannel::Depolarizing {
                probability: clamp_probability(single_q_error),
            }],
            two_qubit_noise: vec![NoiseChannel::Depolarizing {
                probability: clamp_probability(two_q_error),
            }],
            measurement_noise: vec![NoiseChannel::ReadoutError {
                probability: clamp_probability(readout_error),
            }],
            idle_noise: Vec::new(),
        }
    }

    /// Build a noise model from hardware-scale parameters.
    #[must_use]
    pub fn from_hardware(
        t1_us: f64,
        t2_us: f64,
        gate_time_ns: f64,
        single_q_fidelity: f64,
        two_q_fidelity: f64,
        readout_fidelity: f64,
    ) -> Self {
        let gate_duration_us = (gate_time_ns / 1_000.0).max(0.0);
        let gamma = relaxation_parameter(gate_duration_us, t1_us);
        let lambda = relaxation_parameter(gate_duration_us, t2_us);
        let single_q_error = clamp_probability(1.0 - single_q_fidelity);
        let two_q_error = clamp_probability(1.0 - two_q_fidelity);
        let readout_error = clamp_probability(1.0 - readout_fidelity);

        Self {
            single_qubit_noise: vec![
                NoiseChannel::Depolarizing {
                    probability: single_q_error,
                },
                NoiseChannel::AmplitudeDamping { gamma },
                NoiseChannel::PhaseDamping { lambda },
            ],
            two_qubit_noise: vec![
                NoiseChannel::Depolarizing {
                    probability: two_q_error,
                },
                NoiseChannel::AmplitudeDamping { gamma },
                NoiseChannel::PhaseDamping { lambda },
            ],
            measurement_noise: vec![NoiseChannel::ReadoutError {
                probability: readout_error,
            }],
            idle_noise: vec![
                NoiseChannel::AmplitudeDamping {
                    gamma: (gamma * 0.5).clamp(0.0, 1.0),
                },
                NoiseChannel::PhaseDamping {
                    lambda: (lambda * 0.5).clamp(0.0, 1.0),
                },
            ],
        }
    }

    pub(crate) fn is_ideal(&self) -> bool {
        [
            &self.single_qubit_noise,
            &self.two_qubit_noise,
            &self.measurement_noise,
            &self.idle_noise,
        ]
        .into_iter()
        .flatten()
        .all(|channel| !channel.is_effective())
    }
}

fn clamp_probability(probability: f64) -> f64 {
    probability.clamp(0.0, 1.0)
}

fn relaxation_parameter(gate_time_us: f64, timescale_us: f64) -> f64 {
    if timescale_us <= 0.0 {
        return 1.0;
    }
    (1.0 - E.powf(-gate_time_us / timescale_us)).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ideal_model_has_no_effective_channels() {
        assert!(NoiseModel::ideal().is_ideal());
    }

    #[test]
    fn uniform_model_populates_expected_channels() {
        let model = NoiseModel::uniform(0.1, 0.2, 0.3);
        assert_eq!(model.single_qubit_noise.len(), 1);
        assert_eq!(model.two_qubit_noise.len(), 1);
        assert_eq!(model.measurement_noise.len(), 1);
    }

    #[test]
    fn hardware_profile_clamps_probabilities() {
        let model = NoiseModel::from_hardware(-1.0, -1.0, 10.0, 1.5, -1.0, 2.0);
        assert!(!model.is_ideal());
    }
}
