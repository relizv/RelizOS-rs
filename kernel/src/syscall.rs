use x86_64::registers::model_specific::{Msr, Efer, EferFlags};
use crate::task::{self, Message};
use core::ptr;

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

/// Helper function to perform IPC Send
fn sys_send(dest_id: usize, msg_ptr: *const Message, current_rsp: usize) -> usize {
    // 1. Read message from sender
    let msg = unsafe { ptr::read(msg_ptr) };
    
    let mut sched = task::SCHEDULER.lock();
    let sender_id = sched.current_task_idx + 1; // Task ID is index + 1
    
    // Set sender field in message
    let mut msg = msg;
    msg.sender = sender_id;

    // 2. Check if destination task exists and is active
    let dest_idx = dest_id - 1;
    if dest_idx >= 4 || sched.tasks[dest_idx].is_none() {
        // Return error -1 (invalid destination) to caller in rax slot
        unsafe {
            ptr::write((current_rsp as *mut usize).add(14), -1isize as usize);
        }
        return current_rsp;
    }

    // 3. Check if destination is blocked in Receiving state from us (or from ANY)
    let dest_task = sched.tasks[dest_idx].as_mut().unwrap();
    if let task::TaskState::Receiving { src_id, buffer_ptr } = dest_task.state {
        if src_id.is_none() || src_id == Some(sender_id) {
            // Rendezvous! Copy message to destination buffer
            unsafe {
                ptr::write(buffer_ptr as *mut Message, msg);
                // Return 0 (success) to destination's syscall in rax slot
                ptr::write((dest_task.rsp as *mut usize).add(14), 0);
            }
            // Mark destination task as Ready
            dest_task.state = task::TaskState::Ready;

            // Return 0 (success) to sender's syscall in rax slot
            unsafe {
                ptr::write((current_rsp as *mut usize).add(14), 0);
            }
            return current_rsp;
        }
    }

    // 4. Destination is not ready. Block sender in Sending state
    let current_idx = sched.current_task_idx;
    sched.tasks[current_idx].as_mut().unwrap().state = task::TaskState::Sending { dest_id, msg };
    sched.save_current_rsp(current_rsp);

    // Select next ready task
    sched.select_next_task();
    let new_rsp = sched.get_current_rsp();

    // Update TSS and CURRENT_KERNEL_STACK
    if let Some(task) = sched.current_task() {
        unsafe {
            crate::gdt::set_interrupt_stack(x86_64::VirtAddr::new(task.kernel_stack_top as u64));
            CURRENT_KERNEL_STACK = task.kernel_stack_top as u64;
        }
    }

    new_rsp
}

/// Helper function to perform IPC Recv
fn sys_recv(src_filter: usize, msg_ptr: *mut Message, current_rsp: usize) -> usize {
    let mut sched = task::SCHEDULER.lock();
    let current_idx = sched.current_task_idx;
    let current_id = current_idx + 1;

    let src_id_option = if src_filter == 0 { None } else { Some(src_filter) };

    // 1. Search for a sender blocked on sending to us
    let mut sender_idx_found = None;
    for (idx, slot) in sched.tasks.iter().enumerate() {
        if let Some(ref task) = slot {
            if let task::TaskState::Sending { dest_id, msg: _ } = task.state {
                if dest_id == current_id {
                    if src_id_option.is_none() || src_id_option == Some(task.id) {
                        sender_idx_found = Some(idx);
                        break;
                    }
                }
            }
        }
    }

    if let Some(sender_idx) = sender_idx_found {
        // Rendezvous! Retrieve message from sender
        let sender_task = sched.tasks[sender_idx].as_mut().unwrap();
        if let task::TaskState::Sending { dest_id: _, msg } = sender_task.state {
            unsafe {
                // Copy message to receiver
                ptr::write(msg_ptr, msg);
                // Return 0 (success) to sender's syscall in rax slot
                ptr::write((sender_task.rsp as *mut usize).add(14), 0);
            }
            // Mark sender as Ready
            sender_task.state = task::TaskState::Ready;

            // Return 0 (success) to receiver's syscall in rax slot
            unsafe {
                ptr::write((current_rsp as *mut usize).add(14), 0);
            }
            return current_rsp;
        }
    }

    // 2. No sender is ready. Block receiver in Receiving state
    sched.tasks[current_idx].as_mut().unwrap().state = task::TaskState::Receiving {
        src_id: src_id_option,
        buffer_ptr: msg_ptr as usize,
    };
    sched.save_current_rsp(current_rsp);

    // Select next ready task
    sched.select_next_task();
    let new_rsp = sched.get_current_rsp();

    // Update TSS and CURRENT_KERNEL_STACK
    if let Some(task) = sched.current_task() {
        unsafe {
            crate::gdt::set_interrupt_stack(x86_64::VirtAddr::new(task.kernel_stack_top as u64));
            CURRENT_KERNEL_STACK = task.kernel_stack_top as u64;
        }
    }

    new_rsp
}

/// System call dispatcher called from assembly
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
                // Return 0 (success) in rax slot
                ptr::write((current_rsp as *mut usize).add(14), 0);
            }
            current_rsp
        }
        2 => {
            // Syscall 2: Yield
            let mut sched = task::SCHEDULER.lock();
            sched.save_current_rsp(current_rsp);
            sched.select_next_task();
            let new_rsp = sched.get_current_rsp();
            
            if let Some(task) = sched.current_task() {
                unsafe {
                    crate::gdt::set_interrupt_stack(x86_64::VirtAddr::new(task.kernel_stack_top as u64));
                    CURRENT_KERNEL_STACK = task.kernel_stack_top as u64;
                }
            }
            // Return 0 (success) in rax slot
            unsafe {
                ptr::write((current_rsp as *mut usize).add(14), 0);
            }
            new_rsp
        }
        3 => {
            // Syscall 3: IPC Send
            // rdi = dest_id, rsi = msg_ptr
            sys_send(rdi as usize, rsi as *const Message, current_rsp)
        }
        4 => {
            // Syscall 4: IPC Recv
            // rdi = src_filter, rsi = msg_ptr
            sys_recv(rdi as usize, rsi as *mut Message, current_rsp)
        }
        5 => {
            // Syscall 5: Get Used Memory
            let used = unsafe { crate::allocator::USED_MEMORY };
            unsafe {
                ptr::write((current_rsp as *mut usize).add(14), used);
            }
            current_rsp
        }
        6 => {
            // Syscall 6: Get Task Info
            let buffer_ptr = rdi as *mut u8;
            let max_len = rsi as usize;

            let mut info_str = [0u8; 512];
            let mut cursor = 0;

            let mut append = |s: &str| {
                let bytes = s.as_bytes();
                let len = core::cmp::min(bytes.len(), info_str.len() - cursor);
                info_str[cursor..cursor+len].copy_from_slice(&bytes[..len]);
                cursor += len;
            };

            let sched = task::SCHEDULER.lock();
            for slot in sched.tasks.iter() {
                if let Some(ref task) = slot {
                    append("Task ");
                    let mut id_buf = [0u8; 10];
                    let mut id = task.id;
                    let mut i_idx = 10;
                    if id == 0 {
                        i_idx -= 1;
                        id_buf[i_idx] = b'0';
                    } else {
                        while id > 0 {
                            i_idx -= 1;
                            id_buf[i_idx] = b'0' + (id % 10) as u8;
                            id /= 10;
                        }
                    }
                    if let Ok(id_str) = core::str::from_utf8(&id_buf[i_idx..]) {
                        append(id_str);
                    }
                    append(": ");
                    match task.state {
                        task::TaskState::Ready => append("Ready"),
                        task::TaskState::Running => append("Running"),
                        task::TaskState::Sending { dest_id, msg: _ } => {
                            append("Sending to ");
                            let mut d_buf = [0u8; 10];
                            let mut dest = dest_id;
                            let mut d_idx = 10;
                            while dest > 0 {
                                d_idx -= 1;
                                d_buf[d_idx] = b'0' + (dest % 10) as u8;
                                dest /= 10;
                            }
                            if let Ok(dest_str) = core::str::from_utf8(&d_buf[d_idx..]) {
                                append(dest_str);
                            }
                        }
                        task::TaskState::Receiving { src_id, buffer_ptr: _ } => {
                            if let Some(src) = src_id {
                                append("Receiving from ");
                                let mut s_buf = [0u8; 10];
                                let mut src_id_val = src;
                                let mut s_idx = 10;
                                while src_id_val > 0 {
                                    s_idx -= 1;
                                    s_buf[s_idx] = b'0' + (src_id_val % 10) as u8;
                                    src_id_val /= 10;
                                }
                                if let Ok(src_str) = core::str::from_utf8(&s_buf[s_idx..]) {
                                    append(src_str);
                                }
                            } else {
                                append("Receiving from ANY");
                            }
                        }
                    }
                    append("\n");
                }
            }

            unsafe {
                core::ptr::copy_nonoverlapping(info_str.as_ptr(), buffer_ptr, core::cmp::min(cursor, max_len));
                ptr::write((current_rsp as *mut usize).add(14), cursor);
            }
            current_rsp
        }
        _ => {
            // Unknown syscall: return -1 in rax slot
            unsafe {
                ptr::write((current_rsp as *mut usize).add(14), -1isize as usize);
            }
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

        // 6. Switch stack pointer to the returned rsp
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

        // 8. Return to Ring 3 using iretq
        "iretq",

        USER_RSP = sym USER_RSP,
        CURRENT_KERNEL_STACK = sym CURRENT_KERNEL_STACK,
        handle_syscall = sym handle_syscall,
    );
}
