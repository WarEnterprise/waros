use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::quantum::display::format_basis_state;
use crate::quantum::state::QuantumState;

/// One active kernel-side quantum workspace.
pub struct QuantumSession {
    pub state: QuantumState,
    operations: Vec<String>,
    last_result_text: Option<String>,
}

impl QuantumSession {
    #[must_use]
    pub fn new(state: QuantumState) -> Self {
        Self {
            state,
            operations: Vec::new(),
            last_result_text: None,
        }
    }

    pub fn reset(&mut self) {
        self.state.reset();
        self.operations.clear();
        self.last_result_text = None;
    }

    pub fn record_operation(&mut self, operation: String) {
        self.operations.push(operation);
    }

    pub fn record_measurement(&mut self, results: &[(usize, usize)], shots: usize) {
        let mut text = format!("Measurement results ({shots} shots):\n");
        for &(basis, count) in results {
            let probability = (count as f64 / shots as f64) * 100.0;
            text.push_str(&format!(
                "  |{}> : {} ({probability:.1}%)\n",
                format_basis_state(basis, self.state.num_qubits),
                count
            ));
        }
        self.last_result_text = Some(text);
    }

    #[must_use]
    pub fn qasm_source(&self) -> String {
        let mut output = format!(
            "OPENQASM 2.0;\ninclude \"qelib1.inc\";\nqreg q[{}];\n",
            self.state.num_qubits
        );
        for operation in &self.operations {
            output.push_str(operation);
            output.push('\n');
        }
        output
    }

    #[must_use]
    pub fn last_result_text(&self) -> Option<&str> {
        self.last_result_text.as_deref()
    }

    #[must_use]
    pub fn operations(&self) -> &[String] {
        &self.operations
    }
}
