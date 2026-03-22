use waros_quantum::{Circuit, Simulator, WarosError};

fn main() -> Result<(), WarosError> {
    println!("Grover search example (2 qubits)\n");

    let mut circuit = Circuit::new(2)?;

    circuit.h(0)?;
    circuit.h(1)?;

    circuit.cz(0, 1)?;

    circuit.h(0)?;
    circuit.h(1)?;
    circuit.x(0)?;
    circuit.x(1)?;
    circuit.cz(0, 1)?;
    circuit.x(0)?;
    circuit.x(1)?;
    circuit.h(0)?;
    circuit.h(1)?;
    circuit.measure_all()?;

    let result = Simulator::new().run(&circuit, 10_000)?;
    result.print_histogram();

    let (best, count) = result.most_probable();
    println!(
        "\nFound |{}> with {:.1}% probability",
        best,
        f64::from(count) / 100.0
    );

    Ok(())
}
