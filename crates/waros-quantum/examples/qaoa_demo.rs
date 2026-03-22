use waros_quantum::{qaoa_maxcut, Graph, Simulator};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("============================================================");
    println!("  WarOS Quantum - QAOA: MaxCut Optimization");
    println!("  Solving graph partitioning with quantum circuits");
    println!("============================================================");
    println!();

    let simulator = Simulator::with_seed(42);
    let graph = Graph::square();
    let result = qaoa_maxcut(&graph, 2, 20, 1_000, &simulator)?;
    let ratio = result.approximation_ratio.unwrap_or(0.0);

    println!("Graph: Square (4 vertices, 4 edges)");
    println!("  0 -- 1");
    println!("  |    |");
    println!("  3 -- 2");
    println!();
    println!("QAOA with p = 2 layers...");
    println!("  Optimization iterations: {}", result.cost_history.len());
    println!(
        "  Best solution found: {:?}",
        result
            .best_solution
            .iter()
            .map(|bit| if *bit { '1' } else { '0' })
            .collect::<String>()
    );
    println!("  Best cut value:        {:.1}", result.best_cost);
    println!("  Approximation ratio:   {:.2}", ratio);
    println!();
    println!("  QAOA alternates cost and mixer unitaries to bias toward good cuts.");

    Ok(())
}
