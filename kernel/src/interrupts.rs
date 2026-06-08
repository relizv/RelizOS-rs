use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode};
use x86_64::instructions::port::Port;
use x86_64::registers::control::Cr2;
use core::sync::atomic::{AtomicUsize, Ordering};
use crate::task;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// PIT timer frequency in Hz.  Controls scheduler quantum and uptime maths.
pub const PIT_HZ: u32 = 200;

// ---------------------------------------------------------------------------
// Lock-free SPSC byte queue (ISR → task)
// ---------------------------------------------------------------------------

/// 256-slot ring buffer.  Push is called from ISR (single producer),
/// pop is called from the input-server task (single consumer).
pub struct ByteQueue {
    buffer: [u8; 256],
    write_pos: AtomicUsize,
    read_pos: AtomicUsize,
}

unsafe impl Send for ByteQueue {}
unsafe impl Sync for ByteQueue {}

impl ByteQueue {
    pub const fn new() -> Self {
        Self {
            buffer: [0u8; 256],
            write_pos: AtomicUsize::new(0),
            read_pos: AtomicUsize::new(0),
        }
    }

    /// Push one byte (ISR context — must not block or allocate).
    pub fn push(&self, byte: u8) {
        let w = self.write_pos.load(Ordering::Relaxed);
        let next_w = (w + 1) & 0xFF;
        if next_w != self.read_pos.load(Ordering::Acquire) {
            unsafe {
                (self.buffer.as_ptr() as *mut u8).add(w).write_volatile(byte);
            }
            self.write_pos.store(next_w, Ordering::Release);
        }
        // Full → silently drop (256 slots is plenty for 200 Hz polling)
    }

    /// Pop one byte (task context).
    pub fn pop(&self) -> Option<u8> {
        let r = self.read_pos.load(Ordering::Relaxed);
        if r == self.write_pos.load(Ordering::Acquire) {
            return None;
        }
        let byte = unsafe { self.buffer.as_ptr().add(r).read_volatile() };
        self.read_pos.store((r + 1) & 0xFF, Ordering::Release);
        Some(byte)
    }
}

// ---------------------------------------------------------------------------
// Statics
// ---------------------------------------------------------------------------

/// The Interrupt Descriptor Table (initialised once on boot).
static mut IDT: InterruptDescriptorTable = InterruptDescriptorTable::new();

/// Monotonic tick counter incremented by the PIT IRQ0 handler.
pub static mut TIMER_TICKS: u64 = 0;

/// Keyboard scancode queue — filled by IRQ1 ISR, drained by input server.
pub static SCAN_QUEUE: ByteQueue = ByteQueue::new();

/// Mouse byte queue — filled by IRQ12 ISR, drained by input server.
pub static MOUSE_QUEUE: ByteQueue = ByteQueue::new();

// ---------------------------------------------------------------------------
// Initialisation
// ---------------------------------------------------------------------------

/// Set up IDT (exceptions + hardware IRQs), PS/2, PIC and PIT.
pub fn init() {
    unsafe {
        // ---- CPU exception handlers ----
        IDT.page_fault.set_handler_fn(page_fault_handler);
        IDT.general_protection_fault.set_handler_fn(general_protection_handler);
        IDT.invalid_opcode.set_handler_fn(invalid_opcode_handler);
        IDT.stack_segment_fault.set_handler_fn(stack_segment_handler);
        IDT.segment_not_present.set_handler_fn(segment_not_present_handler);
        IDT.double_fault.set_handler_fn(double_fault_handler)
            .set_stack_index(crate::gdt::DOUBLE_FAULT_IST_INDEX);

        // ---- Hardware IRQ handlers ----
        // IRQ0  (vector 0x20 = 32) — Timer  (naked, does context-switch)
        IDT[32].set_handler_fn(core::mem::transmute(timer_interrupt_handler as *const ()));
        // IRQ1  (vector 0x21 = 33) — Keyboard
        IDT[33].set_handler_fn(keyboard_isr);
        // IRQ12 (vector 0x2C = 44) — Mouse
        IDT[44].set_handler_fn(mouse_isr);

        IDT.load();

        // Initialise PS/2 controller (mouse streaming) before unmasking IRQs
        ps2_init();
        // Remap PIC and unmask IRQ0, IRQ1, IRQ2 (cascade), IRQ12
        pic_init();
        // Program PIT channel 0 to PIT_HZ
        pit_init();
    }
}

// ---------------------------------------------------------------------------
// CPU exception handlers
// ---------------------------------------------------------------------------

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

extern "x86-interrupt" fn general_protection_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) {
    crate::println!("=== GENERAL PROTECTION FAULT (#GP) ===");
    crate::println!("  Error code: {:#X}", error_code);
    crate::println!("  RIP: {:?}", stack_frame.instruction_pointer);
    loop { x86_64::instructions::hlt(); }
}

extern "x86-interrupt" fn invalid_opcode_handler(
    stack_frame: InterruptStackFrame,
) {
    crate::println!("=== INVALID OPCODE (#UD) ===");
    crate::println!("  RIP: {:?}", stack_frame.instruction_pointer);
    loop { x86_64::instructions::hlt(); }
}

extern "x86-interrupt" fn stack_segment_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) {
    crate::println!("=== STACK SEGMENT FAULT (#SS) ===");
    crate::println!("  Error code: {:#X}", error_code);
    crate::println!("  RIP: {:?}", stack_frame.instruction_pointer);
    loop { x86_64::instructions::hlt(); }
}

extern "x86-interrupt" fn segment_not_present_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) {
    crate::println!("=== SEGMENT NOT PRESENT (#NP) ===");
    crate::println!("  Error code: {:#X}", error_code);
    crate::println!("  RIP: {:?}", stack_frame.instruction_pointer);
    loop { x86_64::instructions::hlt(); }
}

/// Double Fault — uses IST stack, keep output minimal to avoid overflow!
extern "x86-interrupt" fn double_fault_handler(
    stack_frame: InterruptStackFrame,
    _error_code: u64,
) -> ! {
    crate::println!("=== DOUBLE FAULT ===");
    crate::println!("  RIP: {:?}", stack_frame.instruction_pointer);
    crate::println!("System halted.");
    loop { x86_64::instructions::hlt(); }
}

// ---------------------------------------------------------------------------
// Keyboard ISR (IRQ1, vector 33)
// ---------------------------------------------------------------------------

extern "x86-interrupt" fn keyboard_isr(_frame: InterruptStackFrame) {
    let scancode: u8 = unsafe { Port::<u8>::new(0x60).read() };
    SCAN_QUEUE.push(scancode);
    unsafe { pic_send_eoi(); }
}

// ---------------------------------------------------------------------------
// Mouse ISR (IRQ12, vector 44)
// ---------------------------------------------------------------------------

extern "x86-interrupt" fn mouse_isr(_frame: InterruptStackFrame) {
    let byte: u8 = unsafe { Port::<u8>::new(0x60).read() };
    MOUSE_QUEUE.push(byte);
    unsafe { pic_send_eoi_slave(); }
}

// ---------------------------------------------------------------------------
// 8259 PIC
// ---------------------------------------------------------------------------

/// Remap PIC to offsets 0x20/0x28 and unmask timer, keyboard, cascade, mouse.
unsafe fn pic_init() {
    let mut master_cmd: Port<u8> = Port::new(0x20);
    let mut master_data: Port<u8> = Port::new(0x21);
    let mut slave_cmd: Port<u8> = Port::new(0xA0);
    let mut slave_data: Port<u8> = Port::new(0xA1);

    // ICW1: Start initialization
    master_cmd.write(0x11);
    slave_cmd.write(0x11);

    // ICW2: Vector offset (Master → 0x20, Slave → 0x28)
    master_data.write(0x20);
    slave_data.write(0x28);

    // ICW3: Cascade
    master_data.write(4); // Slave on IRQ2
    slave_data.write(2);

    // ICW4: 8086 mode
    master_data.write(0x01);
    slave_data.write(0x01);

    // OCW1 — interrupt masks
    // Master: enable IRQ0 (timer), IRQ1 (keyboard), IRQ2 (cascade to slave)
    //   bits: 1111_1000 = 0xF8
    master_data.write(0xF8);
    // Slave: enable IRQ12 (mouse) = bit 4 on slave
    //   bits: 1110_1111 = 0xEF
    slave_data.write(0xEF);
}

/// Send EOI to master PIC only (for IRQ0-7).
#[inline]
pub unsafe fn pic_send_eoi() {
    Port::<u8>::new(0x20).write(0x20);
}

/// Send EOI to both slave and master PIC (required for IRQ8-15).
#[inline]
pub unsafe fn pic_send_eoi_slave() {
    Port::<u8>::new(0xA0).write(0x20); // slave first
    Port::<u8>::new(0x20).write(0x20); // then master
}

// ---------------------------------------------------------------------------
// PS/2 controller init (keyboard + mouse)
// ---------------------------------------------------------------------------

unsafe fn ps2_wait_write() {
    for _ in 0..100_000u32 {
        if Port::<u8>::new(0x64).read() & 0x02 == 0 { return; }
    }
}

unsafe fn ps2_wait_read() {
    for _ in 0..100_000u32 {
        if Port::<u8>::new(0x64).read() & 0x01 != 0 { return; }
    }
}

/// Enable the auxiliary (mouse) device and start packet streaming.
unsafe fn ps2_init() {
    let mut cmd = Port::<u8>::new(0x64);
    let mut data = Port::<u8>::new(0x60);

    // 1. Enable auxiliary mouse device
    ps2_wait_write();
    cmd.write(0xA8);

    // 2. Enable IRQ12 in controller config, enable mouse clocks
    ps2_wait_write();
    cmd.write(0x20); // read config byte
    ps2_wait_read();
    let mut config = data.read();
    config |= 0x02;   // IRQ12 enable
    config &= !0x20;  // un-inhibit mouse clock

    ps2_wait_write();
    cmd.write(0x60); // write config byte
    ps2_wait_write();
    data.write(config);

    // 3. Mouse: set defaults
    ps2_wait_write();
    cmd.write(0xD4); // next byte → mouse
    ps2_wait_write();
    data.write(0xF6);
    ps2_wait_read();
    let _ = data.read(); // ACK

    // 4. Mouse: enable packet streaming
    ps2_wait_write();
    cmd.write(0xD4);
    ps2_wait_write();
    data.write(0xF4);
    ps2_wait_read();
    let _ = data.read(); // ACK
}

// ---------------------------------------------------------------------------
// PIT (Programmable Interval Timer)
// ---------------------------------------------------------------------------

/// Program PIT channel 0 to fire at `PIT_HZ` Hz.
unsafe fn pit_init() {
    let divisor = (1_193_182u32 / PIT_HZ) as u16;
    let mut cmd: Port<u8> = Port::new(0x43);
    let mut ch0: Port<u8> = Port::new(0x40);
    cmd.write(0x36); // channel 0, lo/hi byte, mode 3 (square wave)
    ch0.write((divisor & 0xFF) as u8);
    ch0.write((divisor >> 8) as u8);
}

// ---------------------------------------------------------------------------
// Timer interrupt (IRQ0, vector 32) — context-switching handler
// ---------------------------------------------------------------------------

/// Rust helper invoked by the naked timer handler.
/// Saves old RSP, picks next task, sends EOI, returns new RSP.
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
/// Saves all registers, calls handle_timer_interrupt, switches stack,
/// restores registers, and executes iretq.
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
