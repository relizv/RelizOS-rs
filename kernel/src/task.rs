use core::ptr;
use spin::Mutex;

/// Full register state pushed to stack during hardware interrupt preemption.
/// Layout matches the order registers are pushed by timer_interrupt_handler.
#[repr(C, packed)]
struct InitialInterruptStackFrame {
    // Pushed by assembly handler (15 general-purpose registers)
    r15: u64,
    r14: u64,
    r13: u64,
    r12: u64,
    rbp: u64,
    rbx: u64,
    r11: u64,
    r10: u64,
    r9: u64,
    r8: u64,
    rdi: u64,
    rsi: u64,
    rdx: u64,
    rcx: u64,
    rax: u64,
    // Pushed automatically by CPU during interrupt
    rip: u64,
    cs: u64,
    rflags: u64,
    rsp: u64,
    ss: u64,
}

/// Task states
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Ready,
    Running,
}

/// Thread Control Block (TCB)
pub struct Task {
    pub id: usize,
    pub rsp: usize, // Saved stack pointer (points to the saved registers frame)
    pub state: TaskState,
}

impl Task {
    /// Initialize a task stack with the initial interrupt frame layout
    pub fn new(id: usize, entry_point: fn() -> !, stack: &'static mut [u8]) -> Self {
        let stack_top = stack.as_mut_ptr() as usize + stack.len();
        
        // Align stack top to 16 bytes
        let aligned_top = stack_top & !0xF;
        
        let frame_size = core::mem::size_of::<InitialInterruptStackFrame>();
        let frame_ptr = (aligned_top - frame_size) as *mut InitialInterruptStackFrame;
        
        unsafe {
            // Write initial interrupt frame
            ptr::write(frame_ptr, InitialInterruptStackFrame {
                // Initial GP registers
                r15: 0,
                r14: 0,
                r13: 0,
                r12: 0,
                rbp: 0,
                rbx: 0,
                r11: 0,
                r10: 0,
                r9: 0,
                r8: 0,
                rdi: 0,
                rsi: 0,
                rdx: 0,
                rcx: 0,
                rax: 0,
                // Interrupt state pushed by CPU
                rip: entry_point as u64,
                cs: 0x08,             // Kernel Code Selector
                rflags: 0x202,        // Enable Interrupts flag (IF bit 9 enabled!)
                rsp: aligned_top as u64,
                ss: 0x00,             // Kernel Data Selector (usually 0 in long mode)
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

    /// Save the stack pointer of the currently running task
    pub fn save_current_rsp(&mut self, rsp: usize) {
        if self.current_task_idx == 0 && self.tasks[0].is_none() {
            self.main_task_rsp = rsp;
        } else if let Some(ref mut task) = self.tasks[self.current_task_idx] {
            task.rsp = rsp;
        }
    }

    /// Select the next ready task index in a Round Robin fashion
    pub fn select_next_task(&mut self) {
        let current_idx = self.current_task_idx;
        let mut next_idx = (current_idx + 1) % 4;

        loop {
            if next_idx == current_idx {
                // No other tasks, stay on current
                return;
            }
            if self.tasks[next_idx].is_some() {
                break;
            }
            next_idx = (next_idx + 1) % 4;
        }

        self.current_task_idx = next_idx;
    }

    /// Get the stack pointer of the currently active task
    pub fn get_current_rsp(&self) -> usize {
        if self.current_task_idx == 0 && self.tasks[0].is_none() {
            self.main_task_rsp
        } else {
            self.tasks[self.current_task_idx].as_ref().unwrap().rsp
        }
    }

    /// Cooperatively yield execution (fallback/cooperative interface)
    pub fn yield_now(&mut self) {
        let current_idx = self.current_task_idx;
        self.select_next_task();
        let next_idx = self.current_task_idx;

        if next_idx == current_idx {
            return;
        }
        
        unsafe {
            let old_rsp_ptr = if current_idx == 0 && self.tasks[0].is_none() {
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

/// Naked Context Switch Assembly function for cooperative yields
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
