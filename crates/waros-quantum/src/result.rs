use std::collections::HashMap;

use crate::error::{WarosError, WarosResult};

/// Result of executing a quantum circuit multiple times.
#[must_use]
#[derive(Debug, Clone)]
pub struct QuantumResult {
    num_qubits: usize,
    total_shots: u32,
    counts: HashMap<String, u32>,
}

impl QuantumResult {
    pub(crate) fn new(num_qubits: usize, total_shots: u32, counts: HashMap<String, u32>) -> Self {
        Self {
            num_qubits,
            total_shots,
            counts,
        }
    }

    /// Return the raw measurement counts.
    #[must_use]
    pub fn counts(&self) -> &HashMap<String, u32> {
        &self.counts
    }

    /// Return the number of shots used to build this result.
    #[must_use]
    pub fn total_shots(&self) -> u32 {
        self.total_shots
    }

    /// Return the measured probability of a basis state.
    #[must_use]
    pub fn probability(&self, state: &str) -> f64 {
        f64::from(self.counts.get(state).copied().unwrap_or(0)) / f64::from(self.total_shots)
    }

    /// Return the most frequently observed basis state.
    #[must_use]
    pub fn most_probable(&self) -> (&str, u32) {
        self.counts
            .iter()
            .max_by_key(|(_, count)| *count)
            .map_or(("", 0), |(state, count)| (state.as_str(), *count))
    }

    /// Return histogram rows sorted by descending count.
    #[must_use]
    pub fn histogram(&self) -> Vec<(&str, u32, f64)> {
        let mut histogram: Vec<_> = self
            .counts
            .iter()
            .map(|(state, count)| {
                (
                    state.as_str(),
                    *count,
                    f64::from(*count) / f64::from(self.total_shots),
                )
            })
            .collect();
        histogram.sort_by(|lhs, rhs| rhs.1.cmp(&lhs.1));
        histogram
    }

    /// Print a simple ASCII histogram.
    pub fn print_histogram(&self) {
        println!(
            "Results ({} shots, {} qubits):",
            self.total_shots, self.num_qubits
        );
        let histogram = self.histogram();
        let max_count = histogram.first().map_or(1, |entry| entry.1);
        for (state, count, probability) in histogram {
            let scaled = if max_count == 0 {
                0
            } else {
                count.saturating_mul(30) / max_count
            };
            let bar_len = usize::try_from(scaled).unwrap_or(30);
            let bar = "#".repeat(bar_len);
            println!(
                "  |{}>: {:>5} ({:>5.1}%) {}",
                state,
                count,
                probability * 100.0,
                bar
            );
        }
    }

    /// Compute the expectation value of Z on the measured qubit index.
    ///
    /// # Errors
    ///
    /// Returns an error if `qubit` is outside the measured output width.
    pub fn expectation_z(&self, qubit: usize) -> WarosResult<f64> {
        if qubit >= self.num_qubits {
            return Err(WarosError::QubitOutOfRange(qubit, self.num_qubits));
        }

        let mut expectation = 0.0;
        for (state, count) in &self.counts {
            let bit = state.as_bytes().get(qubit).copied().unwrap_or(b'0');
            let eigenvalue = if bit == b'0' { 1.0 } else { -1.0 };
            expectation += eigenvalue * (f64::from(*count) / f64::from(self.total_shots));
        }
        Ok(expectation)
    }
}

impl std::fmt::Display for QuantumResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (state, count, probability) in self.histogram() {
            writeln!(f, "  |{}>: {} ({:.1}%)", state, count, probability * 100.0)?;
        }
        Ok(())
    }
}
