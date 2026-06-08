use core::ptr;
use spin::Mutex;

/// Callee-saved registers pushed to stack by `context_switch`
#[repr(C, packed)]
struct InitialStackFrame {
    r15: u64,
    r14: u64,
    r13: u64,
    r12: u64,
    rbx: u64,
    rbp: u64,
    rip: u64, // Popped by `ret` instruction to jump to task entry point
}

/// Task states
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Ready,
    Running,
}

/// A structure representing a Thread/Task control block
pub struct Task {
    pub id: usize,
    pub rsp: usize, // Current stack pointer of the task
    pub state: TaskState,
}

impl Task {
    /// Initialize a task with a static stack and entry point
    pub fn new(id: usize, entry_point: fn() -> !, stack: &'static mut [u8]) -> Self {
        let stack_top = stack.as_mut_ptr() as usize + stack.len();
        
        // System V ABI requires the stack to be 16-byte aligned.
        // We subtract 8 to account for the fake return address, aligning it to 16 bytes when `ret` is executed.
        let aligned_top = (stack_top & !0xF) - 8;
        
        let frame_size = core::mem::size_of::<InitialStackFrame>();
        let frame_ptr = (aligned_top - frame_size) as *mut InitialStackFrame;
        
        unsafe {
            // Write initial stack frame data
            ptr::write(frame_ptr, InitialStackFrame {
                r15: 0,
                r14: 0,
                r13: 0,
                r12: 0,
                rbx: 0,
                rbp: 0,
                rip: entry_point as u64,
            });
        }

        Self {
            id,
            rsp: frame_ptr as usize,
            state: TaskState::Ready,
        }
    }
}

/// Global Scheduler State (Round Robin)
pub struct Scheduler {
    tasks: [Option<Task>; 4],
    current_task_idx: usize,
    main_task_rsp: usize, // Saved stack pointer of the main boot thread
}

impl Scheduler {
    const fn new() -> Self {
        Self {
            tasks: [None, None, None, None],
            current_task_idx: 0,
            main_task_rsp: 0,
        }
    }

    /// Add a task to the scheduler
    pub fn spawn(&mut self, task: Task) -> Result<(), &'static str> {
        for slot in self.tasks.iter_mut() {
            if slot.is_none() {
                *slot = Some(task);
                return Ok(());
            }
        }
        Err("No free task slots")
    }

    /// Cooperatively yield execution to the next ready task
    pub fn yield_now(&mut self) {
        let current_idx = self.current_task_idx;
        let mut next_idx = (current_idx + 1) % 4;

        // Find the next available task
        loop {
            if next_idx == current_idx {
                // No other tasks to run, stay on current task
                return;
            }
            if self.tasks[next_idx].is_some() {
                break;
            }
            next_idx = (next_idx + 1) % 4;
        }

        // We found a task to switch to!
        self.current_task_idx = next_idx;
        
        // Safety: We obtain raw pointers to swap stack pointers and call assembler context switch.
        unsafe {
            let old_rsp_ptr = if current_idx == 0 && self.tasks[0].is_none() {
                // If we are yielding from the main boot thread for the first time
                &mut self.main_task_rsp as *mut usize
            } else {
                &mut self.tasks[current_idx].as_mut().unwrap().rsp as *mut usize
            };

            let new_rsp = self.tasks[next_idx].as_ref().unwrap().rsp;

            context_switch(old_rsp_ptr, new_rsp);
        }
    }
}

/// Global static scheduler accessor
pub static SCHEDULER: Mutex<Scheduler> = Mutex::new(Scheduler::new());

/// Cooperative yield helper
pub fn yield_now() {
    SCHEDULER.lock().yield_now();
}

/// Naked Context Switch Assembly function
/// System V AMD64 ABI:
/// * First argument (old_rsp_ptr): rdi
/// * Second argument (new_rsp): rsi
#[unsafe(naked)]
pub unsafe extern "C" fn context_switch(_old_rsp_ptr: *mut usize, _new_rsp: usize) {
    core::arch::naked_asm!(
        "push rbp",
        "push rbx",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "mov [rdi], rsp", // Save current rsp to old_rsp_ptr
        "mov rsp, rsi",   // Load new task's rsp from new_rsp
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbx",
        "pop rbp",
        "ret"
    );
}
