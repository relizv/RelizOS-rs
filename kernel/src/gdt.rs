use x86_64::structures::gdt::{GlobalDescriptorTable, Descriptor, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;
use x86_64::VirtAddr;
use spin::Once;

pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;

// Static TSS
static mut TSS: TaskStateSegment = TaskStateSegment::new();
// Static GDT
static mut GDT: GlobalDescriptorTable = GlobalDescriptorTable::new();

pub struct Selectors {
    pub kernel_code: SegmentSelector,
    pub kernel_data: SegmentSelector,
    pub user_data: SegmentSelector,
    pub user_code: SegmentSelector,
    pub tss: SegmentSelector,
}

pub static SELECTORS: Once<Selectors> = Once::new();

// Static buffers for privilege stacks to avoid dynamic allocation
static mut PRIVILEGE_STACK: [u8; 4096] = [0; 4096];
static mut DOUBLE_FAULT_STACK: [u8; 16384] = [0; 16384];

/// Initialize GDT and TSS
pub fn init() {
    use x86_64::instructions::segmentation::{CS, DS, Segment};
    use x86_64::instructions::tables::load_tss;

    unsafe {
        // 1. Initialize TSS Privilege Stack (RSP0)
        // This is the kernel stack loaded when an interrupt occurs while in Ring 3
        let stack_top = VirtAddr::from_ptr(&raw mut PRIVILEGE_STACK as *const u8) + 4096usize;
        TSS.privilege_stack_table[0] = stack_top;

        // Set up IST double fault stack
        let df_stack_top = VirtAddr::from_ptr(&raw mut DOUBLE_FAULT_STACK as *const u8) + 4096usize;
        TSS.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = df_stack_top;

        // 2. Initialize GDT
        // x86_64 Syscall/Sysret expects segments to be adjacent:
        // * Kernel Code segment selector: 0x08 (index 1)
        // * Kernel Data segment selector: 0x10 (index 2)
        // * User Data segment selector: 0x1B (index 3, RPL 3)
        // * User Code segment selector: 0x23 (index 4, RPL 3)
        let kernel_code = GDT.add_entry(Descriptor::kernel_code_segment());
        let kernel_data = GDT.add_entry(Descriptor::kernel_data_segment());
        let user_data = GDT.add_entry(Descriptor::user_data_segment());
        let user_code = GDT.add_entry(Descriptor::user_code_segment());
        let tss = GDT.add_entry(Descriptor::tss_segment(&TSS));

        GDT.load();

        // 3. Load segment selectors
        CS::set_reg(kernel_code);
        
        // In 64-bit mode, DS, ES, SS segment registers are mostly ignored but should be loaded
        // with the kernel data selector or 0.
        DS::set_reg(kernel_data);

        // Load TSS register
        load_tss(tss);

        SELECTORS.call_once(|| Selectors {
            kernel_code,
            kernel_data,
            user_data,
            user_code,
            tss,
        });
    }
}

/// Dynamically update the TSS privilege stack pointer (RSP0) for Ring 3 -> Ring 0 transitions
pub unsafe fn set_interrupt_stack(stack_top: VirtAddr) {
    TSS.privilege_stack_table[0] = stack_top;
}

