use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use waros_quantum::{Backend, Circuit, Simulator, StateVectorLayout};

fn hadamard_circuit(num_qubits: usize) -> Circuit {
    let mut circuit = Circuit::new(num_qubits).expect("valid circuit");
    for qubit in 0..num_qubits {
        circuit.h(qubit).expect("valid gate");
    }
    circuit
}

fn bell_chain_circuit(num_qubits: usize) -> Circuit {
    let mut circuit = Circuit::new(num_qubits).expect("valid circuit");
    circuit.h(0).expect("valid gate");
    for qubit in 0..(num_qubits - 1) {
        circuit.cnot(qubit, qubit + 1).expect("valid gate");
    }
    circuit
}

fn qft_circuit(num_qubits: usize) -> Circuit {
    let mut circuit = Circuit::new(num_qubits).expect("valid circuit");
    let qubits: Vec<usize> = (0..num_qubits).collect();
    circuit.qft(&qubits).expect("valid qft");
    circuit
}

fn grover_oracle_all_ones(
    circuit: &mut Circuit,
    search_qubits: &[usize],
    ancilla_qubits: &[usize],
) -> Result<(), waros_quantum::WarosError> {
    match search_qubits {
        [] => {}
        [qubit] => {
            circuit.z(*qubit)?;
        }
        [control, target] => {
            circuit.cz(*control, *target)?;
        }
        _ => {
            circuit.toffoli(search_qubits[0], search_qubits[1], ancilla_qubits[0])?;
            for offset in 2..(search_qubits.len() - 1) {
                circuit.toffoli(
                    search_qubits[offset],
                    ancilla_qubits[offset - 2],
                    ancilla_qubits[offset - 1],
                )?;
            }
            circuit.cz(
                *ancilla_qubits.last().expect("ancilla exists"),
                *search_qubits.last().expect("search qubit exists"),
            )?;
            for offset in (2..(search_qubits.len() - 1)).rev() {
                circuit.toffoli(
                    search_qubits[offset],
                    ancilla_qubits[offset - 2],
                    ancilla_qubits[offset - 1],
                )?;
            }
            circuit.toffoli(search_qubits[0], search_qubits[1], ancilla_qubits[0])?;
        }
    }

    Ok(())
}

fn grover_circuit(search_qubits: usize) -> Circuit {
    let ancilla_count = search_qubits.saturating_sub(2);
    let total_qubits = search_qubits + ancilla_count;
    let search_register: Vec<usize> = (0..search_qubits).collect();
    let ancilla_register: Vec<usize> = (search_qubits..total_qubits).collect();

    let mut circuit = Circuit::new(total_qubits).expect("valid circuit");
    for &qubit in &search_register {
        circuit.h(qubit).expect("valid gate");
    }

    grover_oracle_all_ones(&mut circuit, &search_register, &ancilla_register)
        .expect("valid oracle");

    for &qubit in &search_register {
        circuit.h(qubit).expect("valid gate");
        circuit.x(qubit).expect("valid gate");
    }
    grover_oracle_all_ones(&mut circuit, &search_register, &ancilla_register)
        .expect("valid diffuser");
    for &qubit in &search_register {
        circuit.x(qubit).expect("valid gate");
        circuit.h(qubit).expect("valid gate");
    }

    circuit
}

fn bench_hadamard_n(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("bench_hadamard_n");
    let simulator = Simulator::builder().parallel(true).build();
    for &num_qubits in &[10usize, 15, 20, 25] {
        let circuit = hadamard_circuit(num_qubits);
        group.bench_with_input(
            BenchmarkId::from_parameter(num_qubits),
            &circuit,
            |bench, circuit| {
                bench.iter(|| {
                    simulator
                        .statevector(black_box(circuit))
                        .expect("simulation succeeds")
                });
            },
        );
    }
    group.finish();
}

fn bench_bell_state_n(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("bench_bell_state_n");
    let simulator = Simulator::builder().parallel(true).build();
    for &num_qubits in &[10usize, 15, 20, 25] {
        let circuit = bell_chain_circuit(num_qubits);
        group.bench_with_input(
            BenchmarkId::from_parameter(num_qubits),
            &circuit,
            |bench, circuit| {
                bench.iter(|| {
                    simulator
                        .statevector(black_box(circuit))
                        .expect("simulation succeeds")
                });
            },
        );
    }
    group.finish();
}

fn bench_qft_n(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("bench_qft_n");
    let simulator = Simulator::builder().parallel(true).build();
    for &num_qubits in &[10usize, 15, 20, 25] {
        let circuit = qft_circuit(num_qubits);
        group.bench_with_input(
            BenchmarkId::from_parameter(num_qubits),
            &circuit,
            |bench, circuit| {
                bench.iter(|| {
                    simulator
                        .statevector(black_box(circuit))
                        .expect("simulation succeeds")
                });
            },
        );
    }
    group.finish();
}

fn bench_grover_n(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("bench_grover_n");
    let simulator = Simulator::builder().parallel(true).build();
    for &num_qubits in &[5usize, 10, 15] {
        let circuit = grover_circuit(num_qubits);
        group.bench_with_input(
            BenchmarkId::from_parameter(num_qubits),
            &circuit,
            |bench, circuit| {
                bench.iter(|| {
                    simulator
                        .statevector(black_box(circuit))
                        .expect("simulation succeeds")
                });
            },
        );
    }
    group.finish();
}

fn bench_statevector_layouts(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("bench_statevector_layouts");
    let aos_simulator = Simulator::builder()
        .parallel(true)
        .statevector_layout(StateVectorLayout::AoS)
        .build();
    let soa_simulator = Simulator::builder()
        .parallel(true)
        .statevector_layout(StateVectorLayout::SoA)
        .build();

    for (label, circuit) in [
        ("hadamard_20", hadamard_circuit(20)),
        ("bell_chain_20", bell_chain_circuit(20)),
    ] {
        group.bench_with_input(
            BenchmarkId::new("AoS", label),
            &circuit,
            |bench, circuit| {
                bench.iter(|| {
                    aos_simulator
                        .statevector(black_box(circuit))
                        .expect("simulation succeeds")
                });
            },
        );
        group.bench_with_input(
            BenchmarkId::new("SoA", label),
            &circuit,
            |bench, circuit| {
                bench.iter(|| {
                    soa_simulator
                        .statevector(black_box(circuit))
                        .expect("simulation succeeds")
                });
            },
        );
    }

    group.finish();
}

fn bench_backend_comparison(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("bench_backend_comparison");
    let statevector_simulator = Simulator::builder()
        .backend(Backend::StateVector)
        .statevector_layout(StateVectorLayout::SoA)
        .build();
    let mps_simulator = Simulator::builder()
        .backend(Backend::MPS { max_bond_dim: 64 })
        .build();

    let bell_chain = bell_chain_circuit(20);
    group.bench_with_input(
        BenchmarkId::new("StateVector", "bell_chain_20"),
        &bell_chain,
        |bench, circuit| {
            bench.iter(|| {
                statevector_simulator
                    .statevector(black_box(circuit))
                    .expect("simulation succeeds")
            });
        },
    );
    group.bench_with_input(
        BenchmarkId::new("MPS", "bell_chain_20"),
        &bell_chain,
        |bench, circuit| {
            bench.iter(|| {
                mps_simulator
                    .run(black_box(circuit), 128)
                    .expect("simulation succeeds")
            });
        },
    );

    let hadamard_40 = hadamard_circuit(40);
    group.bench_with_input(
        BenchmarkId::new("MPS", "hadamard_40"),
        &hadamard_40,
        |bench, circuit| {
            bench.iter(|| {
                mps_simulator
                    .run(black_box(circuit), 128)
                    .expect("simulation succeeds")
            });
        },
    );

    group.finish();
}

criterion_group!(
    statevector_benches,
    bench_hadamard_n,
    bench_bell_state_n,
    bench_qft_n,
    bench_grover_n,
    bench_statevector_layouts,
    bench_backend_comparison
);
criterion_main!(statevector_benches);
