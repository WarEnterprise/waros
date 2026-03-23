use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use arrayvec::ArrayString;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
use spin::{Lazy, Mutex};

use crate::kprintln;

pub static SCHEDULER: Lazy<Mutex<TaskScheduler>> = Lazy::new(|| Mutex::new(TaskScheduler::new()));

/// Cooperative round-robin scheduler for shell-spawned background work.
pub struct TaskScheduler {
    tasks: VecDeque<Task>,
    next_id: u64,
}

/// One cooperatively polled background task.
pub struct Task {
    pub id: u64,
    pub name: ArrayString<32>,
    pub state: TaskState,
    pub future: Pin<Box<dyn Future<Output = ()> + Send>>,
}

/// Observable scheduler state for shell reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Ready,
    Running,
    Waiting,
    Completed,
}

/// Snapshot row used by the `tasks` command.
#[derive(Debug, Clone)]
pub struct TaskInfo {
    pub id: u64,
    pub name: String,
    pub state: TaskState,
}

impl TaskScheduler {
    #[must_use]
    pub fn new() -> Self {
        Self {
            tasks: VecDeque::new(),
            next_id: 1,
        }
    }

    pub fn spawn(
        &mut self,
        name: &str,
        future: impl Future<Output = ()> + Send + 'static,
    ) -> Result<u64, &'static str> {
        let name = task_name(name)?;
        let id = self.next_id;
        self.next_id += 1;
        self.tasks.push_back(Task {
            id,
            name,
            state: TaskState::Ready,
            future: Box::pin(future),
        });
        Ok(id)
    }

    pub fn tick(&mut self) {
        let rounds = self.tasks.len();
        let waker = dummy_waker();
        let mut context = Context::from_waker(&waker);

        for _ in 0..rounds {
            let Some(mut task) = self.tasks.pop_front() else {
                break;
            };

            task.state = TaskState::Running;
            match task.future.as_mut().poll(&mut context) {
                Poll::Ready(()) => {
                    task.state = TaskState::Completed;
                }
                Poll::Pending => {
                    task.state = TaskState::Waiting;
                    self.tasks.push_back(task);
                }
            }
        }
    }

    #[must_use]
    pub fn list(&self) -> &VecDeque<Task> {
        &self.tasks
    }

    pub fn kill(&mut self, id: u64) -> Result<(), &'static str> {
        let Some(index) = self.tasks.iter().position(|task| task.id == id) else {
            return Err("task not found");
        };
        self.tasks.remove(index);
        Ok(())
    }
}

impl Default for TaskScheduler {
    fn default() -> Self {
        Self::new()
    }
}

/// Initialize the scheduler singleton.
pub fn init() {
    *SCHEDULER.lock() = TaskScheduler::new();
}

/// Spawn a background shell command.
pub fn spawn_command(command_line: &str) -> Result<u64, &'static str> {
    let task_name = if command_line.len() > 32 {
        &command_line[..32]
    } else {
        command_line
    };
    let command = command_line.trim().to_string();
    if command.is_empty() {
        return Err("missing command to spawn");
    }

    let command_for_task = command.clone();
    let id = SCHEDULER.lock().spawn(task_name, async move {
        yield_once().await;
        kprintln!();
        kprintln!("[Task] Running: {}", command_for_task);
        crate::shell::commands::execute_command(&command_for_task);
        kprintln!("[Task] Completed: {}", command_for_task);
        crate::shell::reprompt();
    })?;
    Ok(id)
}

/// Poll every runnable task once.
pub fn tick() {
    SCHEDULER.lock().tick();
}

/// Remove one background task.
pub fn kill(id: u64) -> Result<(), &'static str> {
    SCHEDULER.lock().kill(id)
}

/// Return a shell-friendly task list snapshot.
#[must_use]
pub fn snapshot() -> Vec<TaskInfo> {
    SCHEDULER
        .lock()
        .list()
        .iter()
        .map(|task| TaskInfo {
            id: task.id,
            name: task.name.as_str().to_string(),
            state: task.state,
        })
        .collect()
}

/// Return whether background work is ready to be polled again.
#[must_use]
pub fn has_tasks() -> bool {
    !SCHEDULER.lock().list().is_empty()
}

/// Yield exactly once back to the scheduler.
pub async fn yield_once() {
    YieldOnce(false).await;
}

struct YieldOnce(bool);

impl Future for YieldOnce {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        if self.0 {
            Poll::Ready(())
        } else {
            self.0 = true;
            Poll::Pending
        }
    }
}

fn task_name(name: &str) -> Result<ArrayString<32>, &'static str> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("task name cannot be empty");
    }
    if trimmed.len() > 32 {
        return Err("task name exceeds 32 characters");
    }

    let mut task_name = ArrayString::<32>::new();
    task_name.push_str(trimmed);
    Ok(task_name)
}

fn dummy_waker() -> Waker {
    // SAFETY: The vtable never dereferences the data pointer and only returns
    // identical no-op wakers, which is valid for a cooperatively polled loop.
    unsafe { Waker::from_raw(RawWaker::new(core::ptr::null(), &NOOP_WAKER_VTABLE)) }
}

static NOOP_WAKER_VTABLE: RawWakerVTable = RawWakerVTable::new(
    |_| RawWaker::new(core::ptr::null(), &NOOP_WAKER_VTABLE),
    |_| {},
    |_| {},
    |_| {},
);
