use core::ptr;
use spin::Mutex;

/// Fixed-size Message structure for synchronous IPC
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Message {
    pub sender: usize,
    pub msg_type: u64,
    pub arg1: u64,
    pub arg2: u64,
    pub arg3: u64,
    pub arg4: u64,
}

/// Task states
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Ready,
    Running,
    Sending { dest_id: usize, msg: Message },
    Receiving { src_id: Option<usize>, buffer_ptr: usize },
}

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

/// Thread Control Block (TCB)
pub struct Task {
    pub id: usize,
    pub rsp: usize, // Saved stack pointer (points to the saved registers frame on kernel stack)
    pub kernel_stack_top: usize, // Top of the kernel stack for TSS.rsp0
    pub state: TaskState,
}

impl Task {
    /// Check if the task is runnable
    pub fn is_runnable(&self) -> bool {
        self.state == TaskState::Ready || self.state == TaskState::Running
    }

    /// Initialize a task stack with the initial interrupt frame layout (Ring 0)
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
                r15: 0, r14: 0, r13: 0, r12: 0, rbp: 0, rbx: 0, r11: 0, r10: 0, r9: 0, r8: 0, rdi: 0, rsi: 0, rdx: 0, rcx: 0, rax: 0,
                // Interrupt state pushed by CPU
                rip: entry_point as u64,
                cs: 0x08,             // Kernel Code Selector
                rflags: 0x202,        // Enable Interrupts flag
                rsp: aligned_top as u64,
                ss: 0x00,             // Kernel Data Selector
            });
        }

        Self {
            id,
            rsp: frame_ptr as usize,
            kernel_stack_top: aligned_top,
            state: TaskState::Ready,
        }
    }

    /// Initialize a task stack for Ring 3 User Space execution with separate user and kernel stacks
    /// If allow_io is true, IOPL is set to 3 to permit port I/O.
    pub fn new_user(
        id: usize,
        entry_point: fn() -> !,
        user_stack: &'static mut [u8],
        kernel_stack: &'static mut [u8],
        allow_io: bool,
    ) -> Self {
        let user_stack_top = user_stack.as_mut_ptr() as usize + user_stack.len();
        let user_aligned_top = user_stack_top & !0xF;

        let kernel_stack_top = kernel_stack.as_mut_ptr() as usize + kernel_stack.len();
        let kernel_aligned_top = kernel_stack_top & !0xF;
        
        let frame_size = core::mem::size_of::<InitialInterruptStackFrame>();
        let frame_ptr = (kernel_aligned_top - frame_size) as *mut InitialInterruptStackFrame;
        
        // IOPL 3 is bits 12-13 of RFLAGS. Value 3 is 3 << 12 = 0x3000.
        // IF (interrupt flag) is bit 9. Value is 1 << 9 = 0x200.
        let rflags = if allow_io { 0x3202 } else { 0x202 };

        unsafe {
            // Write initial interrupt frame for Ring 3 user space onto kernel stack
            ptr::write(frame_ptr, InitialInterruptStackFrame {
                r15: 0, r14: 0, r13: 0, r12: 0, rbp: 0, rbx: 0, r11: 0, r10: 0, r9: 0, r8: 0, rdi: 0, rsi: 0, rdx: 0, rcx: 0, rax: 0,
                // Interrupt state pushed by CPU
                rip: entry_point as u64,
                cs: 0x23,             // User Code Selector (0x20 | 3)
                rflags,
                rsp: user_aligned_top as u64,
                ss: 0x1B,             // User Data Selector (0x18 | 3)
            });
        }

        Self {
            id,
            rsp: frame_ptr as usize,
            kernel_stack_top: kernel_aligned_top,
            state: TaskState::Ready,
        }
    }
}

/// Global Scheduler State (Round Robin)
pub struct Scheduler {
    pub tasks: [Option<Task>; 4],
    pub current_task_idx: usize,
    pub main_task_rsp: usize, // Saved stack pointer of the main boot thread
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

    /// Select the next ready/runnable task index in a Round Robin fashion
    pub fn select_next_task(&mut self) {
        let current_idx = self.current_task_idx;
        let mut next_idx = (current_idx + 1) % 4;

        loop {
            if next_idx == current_idx {
                // No other tasks are runnable, stay on current (or idle)
                return;
            }
            if let Some(ref task) = self.tasks[next_idx] {
                if task.is_runnable() {
                    break;
                }
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

    /// Access the currently running task
    pub fn current_task(&self) -> Option<&Task> {
        self.tasks[self.current_task_idx].as_ref()
    }
}

/// Global static scheduler accessor
pub static SCHEDULER: Mutex<Scheduler> = Mutex::new(Scheduler::new());

/// Start the first task. This function does not return.
pub unsafe fn start_first_task() -> ! {
    let new_rsp: usize;
    let kernel_stack_top: usize;

    {
        let mut sched = SCHEDULER.lock();
        sched.current_task_idx = 0;
        let first_task = sched.tasks[0].as_ref().expect("No tasks spawned");
        new_rsp = first_task.rsp;
        kernel_stack_top = first_task.kernel_stack_top;
    }

    // Update TSS privilege stack and CURRENT_KERNEL_STACK
    crate::gdt::set_interrupt_stack(x86_64::VirtAddr::new(kernel_stack_top as u64));
    crate::syscall::CURRENT_KERNEL_STACK = kernel_stack_top as u64;

    // Perform assembly jump into the first task
    core::arch::asm!(
        "mov rsp, {rsp}",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbp",
        "pop rbx",
        "pop r11",
        "pop r10",
        "pop r9",
        "pop r8",
        "pop rdi",
        "pop rsi",
        "pop rdx",
        "pop rcx",
        "pop rax",
        "iretq",
        rsp = in(reg) new_rsp,
        options(noreturn)
    );
}
