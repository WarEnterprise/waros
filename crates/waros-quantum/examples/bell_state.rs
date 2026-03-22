use waros_quantum::{Circuit, Simulator, WarosError};

fn main() -> Result<(), WarosError> {
    println!("Bell state example\n");

    let mut circuit = Circuit::new(2)?;
    circuit.h(0)?;
    circuit.cnot(0, 1)?;
    circuit.measure_all()?;

    println!("{circuit}\n");

    let result = Simulator::new().run(&circuit, 10_000)?;
    result.print_histogram();

    let p00 = result.probability("00");
    let p11 = result.probability("11");
    println!("\nP(|00>)={:.1}% P(|11>)={:.1}%", p00 * 100.0, p11 * 100.0);

    if result.probability("01") < 0.01 && result.probability("10") < 0.01 {
        println!("Entanglement detected.");
    }

    Ok(())
}
