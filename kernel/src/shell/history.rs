use alloc::collections::VecDeque;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use spin::{Lazy, Mutex};

const HISTORY_LIMIT: usize = 32;

static HISTORY: Lazy<Mutex<CommandHistory>> = Lazy::new(|| Mutex::new(CommandHistory::new()));

struct CommandHistory {
    entries: VecDeque<String>,
}

impl CommandHistory {
    fn new() -> Self {
        Self {
            entries: VecDeque::with_capacity(HISTORY_LIMIT),
        }
    }

    fn push(&mut self, command: &str) {
        let command = command.trim();
        if command.is_empty() {
            return;
        }

        if self.entries.back().is_some_and(|entry| entry == command) {
            return;
        }

        if self.entries.len() == HISTORY_LIMIT {
            let _ = self.entries.pop_front();
        }

        self.entries.push_back(command.to_string());
    }

    fn snapshot(&self) -> Vec<String> {
        self.entries.iter().cloned().collect()
    }

    fn clear(&mut self) {
        self.entries.clear();
    }
}

/// Push a command into the shell history ring.
pub fn push(command: &str) {
    HISTORY.lock().push(command);
}

/// Return the most recent commands in chronological order.
#[must_use]
pub fn snapshot() -> Vec<String> {
    HISTORY.lock().snapshot()
}

/// Clear shell history at session teardown to avoid cross-session leakage.
pub fn clear() {
    HISTORY.lock().clear();
}
