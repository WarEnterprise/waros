use std::f64::consts::PI;

use waros_quantum::{Circuit, NoiseChannel, NoiseModel, Simulator};

const SHOTS: u32 = 10_000;

fn shannon_entropy(result: &waros_quantum::QuantumResult) -> f64 {
    result
        .histogram()
        .into_iter()
        .filter_map(|(_, _, probability)| {
            (probability > 0.0).then_some(-probability * probability.log2())
        })
        .sum()
}

fn bell_circuit() -> Circuit {
    let mut circuit = Circuit::new(2).expect("valid circuit");
    circuit.h(0).expect("valid gate");
    circuit.cnot(0, 1).expect("valid gate");
    circuit.measure_all().expect("valid measurement");
    circuit
}

#[test]
fn test_ideal_noise_matches_noiseless() {
    let circuit = bell_circuit();
    let noiseless = Simulator::with_seed(7)
        .run(&circuit, SHOTS)
        .expect("simulation succeeds");
    let ideal = Simulator::builder()
        .seed(7)
        .noise(NoiseModel::ideal())
        .build()
        .run(&circuit, SHOTS)
        .expect("simulation succeeds");
    assert_eq!(noiseless.counts(), ideal.counts());
}

#[test]
fn test_depolarizing_reduces_fidelity() {
    let circuit = bell_circuit();
    let noisy = Simulator::builder()
        .seed(11)
        .noise(NoiseModel::uniform(1.0, 1.0, 0.0))
        .build()
        .run(&circuit, SHOTS)
        .expect("simulation succeeds");

    for state in ["00", "01", "10", "11"] {
        let probability = noisy.probability(state);
        assert!(
            (probability - 0.25).abs() < 0.11,
            "{state} probability was {probability:.4}"
        );
    }
}

#[test]
fn test_bitflip_on_zero_state() {
    let mut circuit = Circuit::new(1).expect("valid circuit");
    circuit.rz(0, PI / 7.0).expect("valid gate");
    circuit.measure(0).expect("valid measurement");

    let noise = NoiseModel {
        single_qubit_noise: vec![NoiseChannel::BitFlip { probability: 1.0 }],
        ..NoiseModel::ideal()
    };
    let result = Simulator::builder()
        .seed(3)
        .noise(noise)
        .build()
        .run(&circuit, 1_000)
        .expect("simulation succeeds");
    assert_eq!(result.probability("1"), 1.0);
}

#[test]
fn test_readout_error_flips_bits() {
    let mut circuit = Circuit::new(1).expect("valid circuit");
    circuit.measure(0).expect("valid measurement");

    let noise = NoiseModel {
        measurement_noise: vec![NoiseChannel::ReadoutError { probability: 1.0 }],
        ..NoiseModel::ideal()
    };
    let result = Simulator::builder()
        .seed(5)
        .noise(noise)
        .build()
        .run(&circuit, 1_000)
        .expect("simulation succeeds");
    assert_eq!(result.probability("1"), 1.0);
}

#[test]
fn test_amplitude_damping_decays_to_zero() {
    let mut circuit = Circuit::new(1).expect("valid circuit");
    circuit.x(0).expect("valid gate");
    circuit.measure(0).expect("valid measurement");

    let noise = NoiseModel {
        single_qubit_noise: vec![NoiseChannel::AmplitudeDamping { gamma: 1.0 }],
        ..NoiseModel::ideal()
    };
    let result = Simulator::builder()
        .seed(13)
        .noise(noise)
        .build()
        .run(&circuit, 1_000)
        .expect("simulation succeeds");
    assert_eq!(result.probability("0"), 1.0);
}

#[test]
fn test_ibm_noise_bell_state() {
    let circuit = bell_circuit();
    let result = Simulator::builder()
        .seed(17)
        .noise(NoiseModel::ibm_like())
        .build()
        .run(&circuit, SHOTS)
        .expect("simulation succeeds");
    let fidelity = result.probability("00") + result.probability("11");
    assert!(fidelity < 1.0);
    assert!(fidelity > 0.9, "Bell fidelity too low: {fidelity:.4}");
}

#[test]
fn test_noise_seed_reproducible() {
    let circuit = bell_circuit();
    let first = Simulator::builder()
        .seed(19)
        .noise(NoiseModel::ibm_like())
        .build()
        .run(&circuit, 2_000)
        .expect("simulation succeeds");
    let second = Simulator::builder()
        .seed(19)
        .noise(NoiseModel::ibm_like())
        .build()
        .run(&circuit, 2_000)
        .expect("simulation succeeds");
    assert_eq!(first.counts(), second.counts());
}

#[test]
fn test_zero_noise_no_effect() {
    let circuit = bell_circuit();
    let zero_noise = NoiseModel {
        single_qubit_noise: vec![
            NoiseChannel::Depolarizing { probability: 0.0 },
            NoiseChannel::AmplitudeDamping { gamma: 0.0 },
            NoiseChannel::PhaseDamping { lambda: 0.0 },
        ],
        two_qubit_noise: vec![NoiseChannel::BitFlip { probability: 0.0 }],
        measurement_noise: vec![NoiseChannel::ReadoutError { probability: 0.0 }],
        idle_noise: vec![NoiseChannel::PhaseFlip { probability: 0.0 }],
    };
    let noiseless = Simulator::with_seed(23)
        .run(&circuit, SHOTS)
        .expect("simulation succeeds");
    let result = Simulator::builder()
        .seed(23)
        .noise(zero_noise)
        .build()
        .run(&circuit, SHOTS)
        .expect("simulation succeeds");
    assert_eq!(noiseless.counts(), result.counts());
}

#[test]
fn test_noise_increases_entropy() {
    let circuit = bell_circuit();
    let ideal = Simulator::with_seed(29)
        .run(&circuit, SHOTS)
        .expect("simulation succeeds");
    let noisy = Simulator::builder()
        .seed(29)
        .noise(NoiseModel::uniform(0.3, 0.5, 0.2))
        .build()
        .run(&circuit, SHOTS)
        .expect("simulation succeeds");
    assert!(shannon_entropy(&noisy) > shannon_entropy(&ideal));
}

#[test]
fn test_phase_damping_preserves_populations() {
    let mut circuit = Circuit::new(1).expect("valid circuit");
    circuit.h(0).expect("valid gate");
    circuit.measure(0).expect("valid measurement");

    let noise = NoiseModel {
        single_qubit_noise: vec![NoiseChannel::PhaseDamping { lambda: 1.0 }],
        ..NoiseModel::ideal()
    };
    let result = Simulator::builder()
        .seed(31)
        .noise(noise)
        .build()
        .run(&circuit, SHOTS)
        .expect("simulation succeeds");
    assert!((result.probability("0") - 0.5).abs() < 0.03);
    assert!((result.probability("1") - 0.5).abs() < 0.03);
}

#[test]
fn test_idle_noise_applies_on_barrier() {
    let mut circuit = Circuit::new(1).expect("valid circuit");
    circuit.x(0).expect("valid gate");
    circuit.barrier_all();
    circuit.measure(0).expect("valid measurement");

    let noise = NoiseModel {
        idle_noise: vec![NoiseChannel::AmplitudeDamping { gamma: 1.0 }],
        ..NoiseModel::ideal()
    };
    let result = Simulator::builder()
        .seed(37)
        .noise(noise)
        .build()
        .run(&circuit, 1_000)
        .expect("simulation succeeds");
    assert_eq!(result.probability("0"), 1.0);
}

#[test]
fn test_uniform_profile_is_not_ideal() {
    assert_ne!(NoiseModel::uniform(0.1, 0.2, 0.3), NoiseModel::ideal());
}

#[test]
fn test_ionq_noise_outperforms_ibm_noise() {
    let mut circuit = Circuit::new(5).expect("valid circuit");
    circuit.h(0).expect("valid gate");
    for target in 1..5 {
        circuit.cnot(0, target).expect("valid gate");
    }
    circuit.measure_all().expect("valid measurement");

    let ibm = Simulator::builder()
        .seed(41)
        .noise(NoiseModel::ibm_like())
        .build()
        .run(&circuit, SHOTS)
        .expect("simulation succeeds");
    let ionq = Simulator::builder()
        .seed(41)
        .noise(NoiseModel::ionq_like())
        .build()
        .run(&circuit, SHOTS)
        .expect("simulation succeeds");

    let ibm_fidelity = ibm.probability("00000") + ibm.probability("11111");
    let ionq_fidelity = ionq.probability("00000") + ionq.probability("11111");
    assert!(ionq_fidelity > ibm_fidelity);
}

#[test]
fn test_measurement_noise_only_affects_classical_bits() {
    let mut circuit = Circuit::new(1).expect("valid circuit");
    circuit.x(0).expect("valid gate");
    circuit.measure(0).expect("valid measurement");

    let noisy = Simulator::builder()
        .seed(43)
        .noise(NoiseModel {
            measurement_noise: vec![NoiseChannel::ReadoutError { probability: 0.5 }],
            ..NoiseModel::ideal()
        })
        .build()
        .run(&circuit, SHOTS)
        .expect("simulation succeeds");

    assert!(noisy.probability("1") > 0.4);
    assert!(noisy.probability("1") < 0.6);
}

#[test]
fn test_amplitude_damping_partial_decay_reduces_excited_population() {
    let mut circuit = Circuit::new(1).expect("valid circuit");
    circuit.x(0).expect("valid gate");
    circuit.measure(0).expect("valid measurement");

    let result = Simulator::builder()
        .seed(47)
        .noise(NoiseModel {
            single_qubit_noise: vec![NoiseChannel::AmplitudeDamping { gamma: 0.5 }],
            ..NoiseModel::ideal()
        })
        .build()
        .run(&circuit, SHOTS)
        .expect("simulation succeeds");

    assert!(result.probability("1") < 0.6);
    assert!(result.probability("1") > 0.4);
}
