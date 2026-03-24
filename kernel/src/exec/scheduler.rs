use alloc::collections::VecDeque;

use super::context;
use super::process::{Priority, ProcessState};
use super::PROCESS_TABLE;

pub const DEFAULT_TIME_SLICE: u64 = 10;
pub const QUANTUM_TIME_SLICE: u64 = 50;

pub struct Scheduler {
    run_queues: [VecDeque<u32>; 7],
    current_pid: Option<u32>,
    idle_pid: u32,
    pub context_switch_count: u64,
    enabled: bool,
}

impl Scheduler {
    #[must_use]
    pub fn new() -> Self {
        Self {
            run_queues: Default::default(),
            current_pid: None,
            idle_pid: 0,
            context_switch_count: 0,
            enabled: false,
        }
    }

    pub fn enable(&mut self) {
        self.enabled = true;
    }

    pub fn disable(&mut self) {
        self.enabled = false;
    }

    pub fn timer_tick(&mut self) {
        if !self.enabled {
            return;
        }

        if let Some(pid) = self.current_pid {
            let mut process_table = PROCESS_TABLE.lock();
            if let Some(process) = process_table.get_mut(pid) {
                process.cpu_ticks = process.cpu_ticks.saturating_add(1);
                process.time_slice = process.time_slice.saturating_sub(1);
                if process.time_slice == 0 {
                    process.state = ProcessState::Ready;
                    process.time_slice = self.time_slice_for(process.priority);
                    self.enqueue(pid, process.priority);
                }
            }
        }

        self.select_next();
    }

    pub fn enqueue(&mut self, pid: u32, priority: Priority) {
        let queue_index = priority as usize;
        if queue_index < self.run_queues.len() && !self.run_queues[queue_index].contains(&pid) {
            self.run_queues[queue_index].push_back(pid);
        }
    }

    pub fn dequeue(&mut self, pid: u32) {
        for queue in &mut self.run_queues {
            queue.retain(|candidate| *candidate != pid);
        }
        if self.current_pid == Some(pid) {
            self.current_pid = None;
        }
    }

    pub fn set_current_pid(&mut self, pid: Option<u32>) {
        if self.current_pid != pid {
            self.context_switch_count = self.context_switch_count.saturating_add(1);
            self.current_pid = pid;
            context::activate_process(pid);
        }
    }

    #[must_use]
    pub fn current_pid(&self) -> Option<u32> {
        self.current_pid
    }

    #[must_use]
    pub fn idle_pid(&self) -> u32 {
        self.idle_pid
    }

    fn select_next(&mut self) {
        for queue in &mut self.run_queues {
            if let Some(pid) = queue.pop_front() {
                self.set_current_pid(Some(pid));
                let mut process_table = PROCESS_TABLE.lock();
                if let Some(process) = process_table.get_mut(pid) {
                    process.state = ProcessState::Running;
                }
                return;
            }
        }
        if self.current_pid.is_none() && self.idle_pid != 0 {
            self.set_current_pid(Some(self.idle_pid));
        }
    }

    fn time_slice_for(&self, priority: Priority) -> u64 {
        match priority {
            Priority::RealTime => 100,
            Priority::System => 50,
            Priority::Quantum => QUANTUM_TIME_SLICE,
            Priority::Interactive => 20,
            Priority::Normal => DEFAULT_TIME_SLICE,
            Priority::Batch => 5,
            Priority::Idle => 1,
        }
    }
}
