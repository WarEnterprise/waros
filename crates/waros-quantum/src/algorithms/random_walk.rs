/// Result of a discrete-time quantum random walk on a 1D line.
#[derive(Debug, Clone)]
pub struct RandomWalkResult {
    pub positions: Vec<isize>,
    pub probabilities: Vec<f64>,
    pub variance: f64,
}

type Complex = (f64, f64);

/// Simulate a Hadamard-coin quantum walk for a small number of steps.
#[must_use]
pub fn quantum_random_walk(steps: usize) -> RandomWalkResult {
    let width = 2 * steps + 1;
    let origin = steps;
    let mut amplitudes = vec![[(0.0, 0.0); 2]; width];
    let inv_sqrt_two = std::f64::consts::FRAC_1_SQRT_2;

    // Use (|0> + i|1>) / sqrt(2) as the initial coin state.
    amplitudes[origin][0] = (inv_sqrt_two, 0.0);
    amplitudes[origin][1] = (0.0, inv_sqrt_two);

    for _ in 0..steps {
        let mut coined = vec![[(0.0, 0.0); 2]; width];
        for (position, [coin_zero, coin_one]) in amplitudes.iter().copied().enumerate() {
            coined[position][0] = scale(add(coin_zero, coin_one), inv_sqrt_two);
            coined[position][1] = scale(sub(coin_zero, coin_one), inv_sqrt_two);
        }

        let mut shifted = vec![[(0.0, 0.0); 2]; width];
        for position in 0..width {
            if position > 0 {
                shifted[position - 1][0] = add(shifted[position - 1][0], coined[position][0]);
            }
            if position + 1 < width {
                shifted[position + 1][1] = add(shifted[position + 1][1], coined[position][1]);
            }
        }
        amplitudes = shifted;
    }

    let positions = (-(steps as isize)..=(steps as isize)).collect::<Vec<_>>();
    let probabilities = amplitudes
        .iter()
        .map(|[coin_zero, coin_one]| norm_sq(*coin_zero) + norm_sq(*coin_one))
        .collect::<Vec<_>>();
    let variance = positions
        .iter()
        .zip(probabilities.iter())
        .map(|(position, probability)| (*position as f64).powi(2) * probability)
        .sum();

    RandomWalkResult {
        positions,
        probabilities,
        variance,
    }
}

fn add(lhs: Complex, rhs: Complex) -> Complex {
    (lhs.0 + rhs.0, lhs.1 + rhs.1)
}

fn sub(lhs: Complex, rhs: Complex) -> Complex {
    (lhs.0 - rhs.0, lhs.1 - rhs.1)
}

fn scale(value: Complex, scalar: f64) -> Complex {
    (value.0 * scalar, value.1 * scalar)
}

fn norm_sq(value: Complex) -> f64 {
    value.0 * value.0 + value.1 * value.1
}
