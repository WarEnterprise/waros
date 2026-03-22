use rand::Rng;

use crate::error::{WarosError, WarosResult};
use crate::simulator::Simulator;

/// Result of Shor's small-integer factoring demo.
#[derive(Debug, Clone)]
pub struct ShorResult {
    pub n: u64,
    pub factors: (u64, u64),
    pub base_a: u64,
    pub period_r: u64,
    pub attempts: u32,
    pub success: bool,
}

/// Factor a small integer using a simulator-friendly variant of Shor's algorithm.
///
/// This implementation demonstrates the quantum period-finding workflow for
/// `N <= 21` by sampling phases from the exact modular-order spectrum and using
/// continued fractions to recover the order.
pub fn shor_factor(n: u64, simulator: &Simulator) -> WarosResult<ShorResult> {
    if n < 4 {
        return Err(WarosError::SimulationError("N must be at least 4".into()));
    }
    if n % 2 == 0 {
        return Ok(trivial_factor(n, 2, 1));
    }
    if let Some(factor) = prime_power_factor(n) {
        return Ok(trivial_factor(n, factor, 1));
    }

    let precision_bits = ((2.0 * (n as f64).log2()).ceil() as usize).max(4);
    if precision_bits > 16 {
        return Err(WarosError::TooManyQubits(precision_bits, 16));
    }

    let candidates: Vec<u64> = (2..n).filter(|candidate| gcd(*candidate, n) == 1).collect();
    if candidates.is_empty() {
        return Ok(ShorResult {
            n,
            factors: (1, n),
            base_a: 0,
            period_r: 0,
            attempts: 0,
            success: false,
        });
    }

    let mut rng = simulator.make_rng();
    let start_index = rng.gen_range(0..candidates.len());

    for attempt in 0..candidates.len().min(10) {
        let a = candidates[(start_index + attempt) % candidates.len()];
        let r = quantum_order_finding(a, n, precision_bits, &mut rng)?;
        if r == 0 || r % 2 != 0 {
            continue;
        }

        let half_power = mod_pow(a, r / 2, n);
        if half_power == n - 1 || half_power == 0 {
            continue;
        }

        let factor_plus = gcd(half_power + 1, n);
        let factor_minus = gcd(half_power.saturating_sub(1), n);
        for factor in [factor_plus, factor_minus] {
            if factor > 1 && factor < n {
                return Ok(ShorResult {
                    n,
                    factors: (factor, n / factor),
                    base_a: a,
                    period_r: r,
                    attempts: attempt as u32 + 1,
                    success: true,
                });
            }
        }
    }

    Ok(ShorResult {
        n,
        factors: (1, n),
        base_a: 0,
        period_r: 0,
        attempts: candidates.len().min(10) as u32,
        success: false,
    })
}

/// Modular exponentiation by repeated squaring.
#[must_use]
pub fn mod_pow(mut base: u64, mut exponent: u64, modulus: u64) -> u64 {
    if modulus == 1 {
        return 0;
    }

    let mut result = 1u64;
    base %= modulus;
    while exponent > 0 {
        if exponent & 1 == 1 {
            result = result.wrapping_mul(base) % modulus;
        }
        exponent >>= 1;
        base = base.wrapping_mul(base) % modulus;
    }
    result
}

/// Greatest common divisor via the Euclidean algorithm.
#[must_use]
pub fn gcd(mut lhs: u64, mut rhs: u64) -> u64 {
    while rhs != 0 {
        let remainder = lhs % rhs;
        lhs = rhs;
        rhs = remainder;
    }
    lhs
}

/// Recover a denominator from a measured phase using continued fractions.
#[must_use]
pub fn continued_fraction_period(phase: f64, max_denominator: u64) -> u64 {
    if !(0.0..1.0).contains(&phase) {
        return 0;
    }

    let mut value = phase;
    let mut p_prev = 0u64;
    let mut p_curr = 1u64;
    let mut q_prev = 1u64;
    let mut q_curr = 0u64;

    for _ in 0..24 {
        let coefficient = value.floor() as u64;
        let p_next = coefficient.saturating_mul(p_curr).saturating_add(p_prev);
        let q_next = coefficient.saturating_mul(q_curr).saturating_add(q_prev);
        if q_next > max_denominator {
            break;
        }

        p_prev = p_curr;
        p_curr = p_next;
        q_prev = q_curr;
        q_curr = q_next;

        let fractional = value - coefficient as f64;
        if fractional.abs() < 1e-12 {
            break;
        }
        value = 1.0 / fractional;
    }

    q_curr
}

fn quantum_order_finding(
    a: u64,
    n: u64,
    precision_bits: usize,
    rng: &mut rand::rngs::StdRng,
) -> WarosResult<u64> {
    let true_order = multiplicative_order(a, n).ok_or_else(|| {
        WarosError::SimulationError(format!(
            "failed to find multiplicative order for a = {a}, n = {n}"
        ))
    })?;

    let coprime_phases: Vec<u64> = (1..true_order)
        .filter(|numerator| gcd(*numerator, true_order) == 1)
        .collect();
    if coprime_phases.is_empty() {
        return Ok(true_order);
    }

    let numerator = coprime_phases[rng.gen_range(0..coprime_phases.len())];
    let precision_scale = 1u64 << precision_bits;
    let measured_value = ((numerator * precision_scale) as f64 / true_order as f64).round() as u64;
    let estimated_phase = measured_value as f64 / precision_scale as f64;

    let candidate = continued_fraction_period(estimated_phase, n);
    if candidate > 0 && mod_pow(a, candidate, n) == 1 {
        return Ok(candidate);
    }

    for multiple in 1..=n {
        let refined = candidate.saturating_mul(multiple);
        if refined > 0 && mod_pow(a, refined, n) == 1 {
            return Ok(refined);
        }
    }

    Ok(true_order)
}

fn multiplicative_order(a: u64, n: u64) -> Option<u64> {
    if gcd(a, n) != 1 {
        return None;
    }

    let mut value = 1u64;
    for order in 1..=n {
        value = (value * a) % n;
        if value == 1 {
            return Some(order);
        }
    }
    None
}

fn prime_power_factor(n: u64) -> Option<u64> {
    for base in 2..=((n as f64).sqrt() as u64 + 1) {
        let mut value = base * base;
        while value < n {
            value = value.saturating_mul(base);
        }
        if value == n {
            return Some(base);
        }
    }
    None
}

fn trivial_factor(n: u64, factor: u64, attempts: u32) -> ShorResult {
    ShorResult {
        n,
        factors: (factor, n / factor),
        base_a: factor,
        period_r: 0,
        attempts,
        success: true,
    }
}
