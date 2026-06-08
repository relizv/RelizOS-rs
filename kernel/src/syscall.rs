use x86_64::registers::model_specific::{Msr, Efer, EferFlags};
use crate::task;

const IA32_STAR: u32 = 0xC0000081;
const IA32_LSTAR: u32 = 0xC0000082;
const IA32_FMASK: u32 = 0xC0000084;

// Static variables to temporarily store stack pointers during syscall context switch.
// Safe because interrupts are disabled during the swap.
pub static mut USER_RSP: u64 = 0;
pub static mut CURRENT_KERNEL_STACK: u64 = 0;

static mut SYSCALL_STACK: [u8; 8192] = [0; 8192];

/// Initialize MSR registers and enable syscall/sysret
pub fn init() {
    unsafe {
        let mut star = Msr::new(IA32_STAR);
        let mut lstar = Msr::new(IA32_LSTAR);
        let mut fmask = Msr::new(IA32_FMASK);

        // 1. STAR MSR:
        // * Bits 32-47: Kernel Code Selector (0x08)
        // * Bits 48-63: Base User Selector (0x10 | 3 = 0x13) -> User Data is 0x1B, User Code is 0x23
        star.write((0x08u64 << 32) | (0x13u64 << 48));

        // 2. LSTAR MSR: Address of our assembly syscall handler
        lstar.write(syscall_handler as u64);

        // 3. FMASK MSR: Mask out Interrupt Flag (IF = bit 9 = 0x200) and Direction Flag (DF = bit 10 = 0x400)
        // This disables interrupts automatically when entering syscall_handler to prevent race conditions.
        fmask.write(0x200 | 0x400);

        // 4. EFER MSR: Enable System Call Extensions (SCE)
        Efer::update(|flags| {
            *flags |= EferFlags::SYSTEM_CALL_EXTENSIONS;
        });

        // Set up the top of the dedicated kernel syscall stack (temporary until first task switch)
        let stack_top = &raw mut SYSCALL_STACK as *const u8 as u64 + 8192;
        CURRENT_KERNEL_STACK = stack_top;
    }
}

/// System call dispatcher called from assembly
/// Takes rax, rdi, rsi, rdx, and the current task's kernel stack pointer (rsp).
/// Returns the next task's kernel stack pointer.
#[no_mangle]
pub extern "C" fn handle_syscall(rax: u64, rdi: u64, rsi: u64, _rdx: u64, current_rsp: usize) -> usize {
    match rax {
        1 => {
            // Syscall 1: Print String
            // rdi = pointer to string bytes, rsi = string length
            let ptr = rdi as *const u8;
            let len = rsi as usize;
            
            // Safety: We assume user space sends a valid memory pointer within its own range.
            unsafe {
                let slice = core::slice::from_raw_parts(ptr, len);
                if let Ok(text) = core::str::from_utf8(slice) {
                    crate::print!("{}", text);
                }
            }
            current_rsp
        }
        2 => {
            // Syscall 2: Yield
            let mut sched = task::SCHEDULER.lock();
            
            // Save current task's rsp
            sched.save_current_rsp(current_rsp);
            
            // Select next task
            sched.select_next_task();
            let new_rsp = sched.get_current_rsp();
            
            // Update TSS and CURRENT_KERNEL_STACK for the next task
            if let Some(task) = sched.current_task() {
                unsafe {
                    crate::gdt::set_interrupt_stack(x86_64::VirtAddr::new(task.kernel_stack_top as u64));
                    CURRENT_KERNEL_STACK = task.kernel_stack_top as u64;
                }
            }
            
            new_rsp
        }
        _ => {
            // Unknown syscall: return same rsp to resume current task
            current_rsp
        }
    }
}

/// Naked Syscall Handler assembly.
/// Handles stack swapping, pushes user state, calls dispatcher, restores state, and calls iretq.
#[unsafe(naked)]
pub unsafe extern "C" fn syscall_handler() {
    core::arch::naked_asm!(
        // Disable interrupts (safety double check)
        "cli",
        
        // 1. Save user stack pointer to USER_RSP
        "mov qword ptr [rip + {USER_RSP}], rsp",
        // 2. Switch to the task's kernel stack
        "mov rsp, qword ptr [rip + {CURRENT_KERNEL_STACK}]",

        // 3. Construct the Interrupt Stack Frame (ss, rsp, rflags, cs, rip)
        "push 0x1B",                          // ss (User Data Selector)
        "push qword ptr [rip + {USER_RSP}]",  // rsp (User Stack Pointer)
        "push r11",                           // rflags (Saved by CPU in r11)
        "push 0x23",                          // cs (User Code Selector)
        "push rcx",                           // rip (Saved by CPU in rcx)

        // 4. Push all general-purpose registers (15 registers)
        "push rax",
        "push rcx",
        "push rdx",
        "push rsi",
        "push rdi",
        "push r8",
        "push r9",
        "push r10",
        "push r11",
        "push rbx",
        "push rbp",
        "push r12",
        "push r13",
        "push r14",
        "push r15",

        // 5. Call the Rust dispatcher
        // System V AMD64 ABI: First 6 arguments in rdi, rsi, rdx, rcx, r8, r9.
        // We pass:
        // rdi = rax (syscall number)
        // rsi = rdi (arg 1)
        // rdx = rsi (arg 2)
        // rcx = rdx (arg 3)
        // r8 = rsp (current_rsp)
        "mov r8, rsp",
        "mov rcx, rdx",
        "mov rdx, rsi",
        "mov rsi, rdi",
        "mov rdi, rax",
        "call {handle_syscall}", // returns new task's rsp in rax

        // 6. Switch stack pointer to the returned rsp (could be different if we yielded!)
        "mov rsp, rax",

        // 7. Restore all general-purpose registers
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

        // 8. Return to Ring 3 using iretq (restores rip, cs, rflags, rsp, ss from stack)
        "iretq",

        USER_RSP = sym USER_RSP,
        CURRENT_KERNEL_STACK = sym CURRENT_KERNEL_STACK,
        handle_syscall = sym handle_syscall,
    );
}
