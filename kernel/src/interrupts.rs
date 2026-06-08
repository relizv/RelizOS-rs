use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode};
use x86_64::instructions::port::Port;
use x86_64::registers::control::Cr2;
use crate::task;

// Static mutable IDT. Safe because it is initialized once on boot.
static mut IDT: InterruptDescriptorTable = InterruptDescriptorTable::new();

pub static mut TIMER_TICKS: u64 = 0;

/// Initialize the IDT, register handlers, and load it
pub fn init() {
    unsafe {
        // CPU exception handlers
        IDT.page_fault.set_handler_fn(page_fault_handler);
        IDT.general_protection_fault.set_handler_fn(general_protection_handler);
        IDT.invalid_opcode.set_handler_fn(invalid_opcode_handler);
        IDT.stack_segment_fault.set_handler_fn(stack_segment_handler);
        IDT.segment_not_present.set_handler_fn(segment_not_present_handler);
        IDT.double_fault.set_handler_fn(double_fault_handler)
            .set_stack_index(crate::gdt::DOUBLE_FAULT_IST_INDEX);
        
        // Timer interrupt is mapped to offset 0x20 (index 32)
        IDT[32].set_handler_fn(core::mem::transmute(timer_interrupt_handler as *const ()));
        
        IDT.load();
        
        // Initialize PIC and remap interrupts
        pic_init();
    }
}

/// Page Fault (#PF) Exception Handler
extern "x86-interrupt" fn page_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    crate::println!("=== PAGE FAULT (#PF) ===");
    crate::println!("  CR2 (accessed addr): {:?}", Cr2::read());
    crate::println!("  Error code: {:?}", error_code);
    crate::println!("  RIP: {:?}", stack_frame.instruction_pointer);
    loop { x86_64::instructions::hlt(); }
}

/// General Protection Fault (#GP) Exception Handler
extern "x86-interrupt" fn general_protection_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) {
    crate::println!("=== GENERAL PROTECTION FAULT (#GP) ===");
    crate::println!("  Error code: {:#X}", error_code);
    crate::println!("  RIP: {:?}", stack_frame.instruction_pointer);
    loop { x86_64::instructions::hlt(); }
}

/// Invalid Opcode (#UD) Exception Handler
extern "x86-interrupt" fn invalid_opcode_handler(
    stack_frame: InterruptStackFrame,
) {
    crate::println!("=== INVALID OPCODE (#UD) ===");
    crate::println!("  RIP: {:?}", stack_frame.instruction_pointer);
    loop { x86_64::instructions::hlt(); }
}

/// Stack Segment Fault (#SS) Exception Handler
extern "x86-interrupt" fn stack_segment_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) {
    crate::println!("=== STACK SEGMENT FAULT (#SS) ===");
    crate::println!("  Error code: {:#X}", error_code);
    crate::println!("  RIP: {:?}", stack_frame.instruction_pointer);
    loop { x86_64::instructions::hlt(); }
}

/// Segment Not Present (#NP) Exception Handler
extern "x86-interrupt" fn segment_not_present_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) {
    crate::println!("=== SEGMENT NOT PRESENT (#NP) ===");
    crate::println!("  Error code: {:#X}", error_code);
    crate::println!("  RIP: {:?}", stack_frame.instruction_pointer);
    loop { x86_64::instructions::hlt(); }
}

/// Double Fault Exception Handler — uses IST stack, keep output minimal!
extern "x86-interrupt" fn double_fault_handler(
    stack_frame: InterruptStackFrame,
    _error_code: u64,
) -> ! {
    crate::println!("=== DOUBLE FAULT ===");
    crate::println!("  RIP: {:?}", stack_frame.instruction_pointer);
    crate::println!("System halted.");
    loop { x86_64::instructions::hlt(); }
}

/// Initialize the 8259 PIC using raw port I/O.
/// Remaps Master PIC to offset 0x20 and Slave to 0x28.
unsafe fn pic_init() {
    let mut master_cmd: Port<u8> = Port::new(0x20);
    let mut master_data: Port<u8> = Port::new(0x21);
    let mut slave_cmd: Port<u8> = Port::new(0xA0);
    let mut slave_data: Port<u8> = Port::new(0xA1);

    // ICW1: Start initialization
    master_cmd.write(0x11);
    slave_cmd.write(0x11);

    // ICW2: Vector offset (Master -> 0x20, Slave -> 0x28)
    master_data.write(0x20);
    slave_data.write(0x28);

    // ICW3: Cascade setup
    master_data.write(4); // Slave is at IRQ2
    slave_data.write(2);  // Slave cascade identity

    // ICW4: Environment info (8086 mode)
    master_data.write(0x01);
    slave_data.write(0x01);

    // Set interrupt masks: enable only IRQ 0 (Timer)
    // Bit 0 = 0 (enabled), Bits 1-7 = 1 (disabled)
    master_data.write(0xFE); 
    slave_data.write(0xFF); // Disable all slave interrupts
}

/// Send End of Interrupt (EOI) signal to Master PIC
#[inline]
pub unsafe fn pic_send_eoi() {
    let mut master_cmd: Port<u8> = Port::new(0x20);
    master_cmd.write(0x20);
}

/// Rust helper function invoked by naked timer interrupt handler.
/// Receives the stack pointer of the interrupted task, saves it,
/// triggers the scheduler to pick the next task, sends PIC EOI,
/// and returns the stack pointer of the new task.
#[no_mangle]
pub extern "C" fn handle_timer_interrupt(old_rsp: usize) -> usize {
    unsafe {
        TIMER_TICKS += 1;
    }

    let mut sched = task::SCHEDULER.lock();
    
    // 1. Save old task stack pointer
    sched.save_current_rsp(old_rsp);

    // 2. Select next ready task
    sched.select_next_task();
    let new_rsp = sched.get_current_rsp();

    // 3. Update TSS privilege stack and CURRENT_KERNEL_STACK for the next task
    if let Some(task) = sched.current_task() {
        unsafe {
            crate::gdt::set_interrupt_stack(x86_64::VirtAddr::new(task.kernel_stack_top as u64));
            crate::syscall::CURRENT_KERNEL_STACK = task.kernel_stack_top as u64;
        }
    }

    // 4. Send End of Interrupt (EOI) signal to the PIC
    unsafe {
        pic_send_eoi();
    }

    new_rsp
}

/// Naked Timer Interrupt Handler.
/// Saves all registers, calls handle_timer_interrupt, switches stack, restores registers, and executes iretq.
#[unsafe(naked)]
pub unsafe extern "C" fn timer_interrupt_handler() {
    core::arch::naked_asm!(
        // 1. Push all general-purpose registers to save context
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

        // 2. Pass current stack pointer (rsp) to the Rust handler
        "mov rdi, rsp",
        "call handle_timer_interrupt", // returns new stack pointer in rax

        // 3. Switch stack pointer (rsp) to the new task stack pointer
        "mov rsp, rax",

        // 4. Pop general-purpose registers to load new context
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

        // 5. Return from interrupt (pops rip, cs, rflags, rsp, ss)
        "iretq"
    );
}
