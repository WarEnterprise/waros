use waros_quantum::{shor_factor, Simulator};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("============================================================");
    println!("  WarOS Quantum - Shor's Factoring Algorithm");
    println!("  The algorithm that threatens RSA encryption");
    println!("============================================================");
    println!();

    let simulator = Simulator::with_seed(42);
    let result = shor_factor(15, &simulator)?;

    println!("Factoring N = {}...", result.n);
    println!("  Using quantum period-finding with a = {}", result.base_a);
    println!("  Period found: r = {}", result.period_r);
    if result.success {
        println!(
            "  Computing factors from a^(r/2) +/- 1 produced {} and {}",
            result.factors.0, result.factors.1
        );
        println!();
        println!(
            "  OK: {} = {} x {} (found in {} attempt{})",
            result.n,
            result.factors.0,
            result.factors.1,
            result.attempts,
            if result.attempts == 1 { "" } else { "s" }
        );
    } else {
        println!("  Shor demo did not find non-trivial factors in this run.");
    }
    println!();
    println!("  Note: this simulator demo targets tiny composite numbers only.");
    println!("  The same period-finding structure scales to the RSA-breaking regime.");

    Ok(())
}
