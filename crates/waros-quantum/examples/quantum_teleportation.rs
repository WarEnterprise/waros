use std::f64::consts::PI;

use waros_quantum::{Circuit, Simulator, WarosError};

fn main() -> Result<(), WarosError> {
    let theta = PI / 3.0;
    let expected_p0 = (theta / 2.0).cos().powi(2);
    let expected_p1 = (theta / 2.0).sin().powi(2);

    println!("Teleporting Ry({theta:.4})|0>");
    println!("Expected P(0) = {:.1}%", expected_p0 * 100.0);
    println!("Expected P(1) = {:.1}%\n", expected_p1 * 100.0);

    let mut direct = Circuit::new(1)?;
    direct.ry(0, theta)?;
    direct.measure(0)?;
    println!("Direct measurement:");
    Simulator::new().run(&direct, 10_000)?.print_histogram();

    let mut circuit = Circuit::new(3)?;
    circuit.ry(0, theta)?;
    circuit.h(1)?;
    circuit.cnot(1, 2)?;
    circuit.cnot(0, 1)?;
    circuit.h(0)?;
    circuit.measure(0)?;
    circuit.measure(1)?;
    circuit.cnot(1, 2)?;
    circuit.cz(0, 2)?;
    circuit.measure(2)?;

    let result = Simulator::new().run(&circuit, 10_000)?;

    let mut bob_0 = 0u32;
    let mut bob_1 = 0u32;
    for (bits, count) in result.counts() {
        match bits.as_bytes().get(2) {
            Some(b'0') => bob_0 += count,
            Some(b'1') => bob_1 += count,
            _ => {}
        }
    }

    let total = bob_0 + bob_1;
    println!("\nBob's qubit:");
    println!(
        "  |0>: {} ({:.1}%)",
        bob_0,
        f64::from(bob_0) / f64::from(total) * 100.0
    );
    println!(
        "  |1>: {} ({:.1}%)",
        bob_1,
        f64::from(bob_1) / f64::from(total) * 100.0
    );

    Ok(())
}
