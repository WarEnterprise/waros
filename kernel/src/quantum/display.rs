use alloc::string::String;

use crate::display::console::Colors;
use crate::quantum::state::QuantumState;
use crate::{kprint, kprint_colored, kprintln};

const STATE_EPSILON: f64 = 1e-10;
const HISTOGRAM_WIDTH: usize = 28;

/// Render measurement results as counts plus an ASCII histogram.
pub fn display_results(results: &[(usize, usize)], bit_count: usize, total_shots: usize) {
    if total_shots == 0 {
        kprintln!("Measurement skipped: 0 shots requested.");
        return;
    }

    kprintln!("Measurement results ({} shots):", total_shots);
    let max_count = results.first().map_or(1, |(_, count)| *count);

    for &(basis_index, count) in results {
        let probability = (count as f64 / total_shots as f64) * 100.0;
        let bar_length = if max_count == 0 {
            0
        } else {
            (count * HISTOGRAM_WIDTH) / max_count
        };

        kprint!(
            "  |{}> : {:>5} ({:>5.1}%) ",
            format_basis_state(basis_index, bit_count),
            count,
            probability
        );
        kprint_colored!(Colors::CYAN, "{}\n", repeat_char('#', bar_length.max(1)));
    }
}

/// Render the non-zero amplitudes of a state vector.
pub fn display_state(state: &QuantumState) {
    kprintln!(
        "State vector ({} qubits, {} amplitudes):",
        state.num_qubits,
        state.amplitudes.len()
    );

    let mut displayed = false;
    for (basis_index, amplitude) in state.amplitudes.iter().copied().enumerate() {
        let probability = amplitude.0 * amplitude.0 + amplitude.1 * amplitude.1;
        if probability < STATE_EPSILON {
            continue;
        }

        displayed = true;
        kprint!(
            "  |{}> : ",
            format_basis_state(basis_index, state.num_qubits)
        );
        if amplitude.1 >= 0.0 {
            kprint!("{:.4} + {:.4}i", amplitude.0, amplitude.1);
        } else {
            kprint!("{:.4} - {:.4}i", amplitude.0, -amplitude.1);
        }
        kprintln!("  ({:.1}%)", probability * 100.0);
    }

    if !displayed {
        kprintln!("  <all amplitudes numerically zero>");
    }
}

/// Render the probability distribution of the current state.
pub fn display_probabilities(state: &QuantumState) {
    kprintln!("Probability distribution ({} qubits):", state.num_qubits);
    let probabilities = state.probabilities();
    let max_probability = probabilities
        .iter()
        .copied()
        .fold(0.0_f64, f64::max)
        .max(STATE_EPSILON);

    for (basis_index, probability) in probabilities.into_iter().enumerate() {
        if probability < STATE_EPSILON {
            continue;
        }

        let bar_length = ((probability / max_probability) * HISTOGRAM_WIDTH as f64) as usize;
        kprint!(
            "  |{}> : {:>5.1}% ",
            format_basis_state(basis_index, state.num_qubits),
            probability * 100.0
        );
        kprint_colored!(Colors::BLUE, "{}\n", repeat_char('=', bar_length.max(1)));
    }
}

/// Format a compact basis index into a binary ket string with the most-significant qubit on the left.
#[must_use]
pub fn format_basis_state(index: usize, bit_count: usize) -> String {
    let mut output = String::with_capacity(bit_count.max(1));
    for bit in (0..bit_count).rev() {
        if (index >> bit) & 1 == 1 {
            output.push('1');
        } else {
            output.push('0');
        }
    }
    output
}

fn repeat_char(character: char, count: usize) -> String {
    let mut output = String::with_capacity(count);
    for _ in 0..count {
        output.push(character);
    }
    output
}
