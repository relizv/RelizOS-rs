use x86_64::structures::paging::{PageTable, PageTableFlags};
use x86_64::registers::control::Cr3;
use x86_64::PhysAddr;
use core::alloc::Layout;

/// Retrieve the active Level 4 page table
pub unsafe fn active_level4_table() -> &'static mut PageTable {
    let (level4_table_frame, _) = Cr3::read();
    let phys = level4_table_frame.start_address().as_u64();
    // Under our identity mapping configuration, virtual address = physical address
    let virt = phys;
    &mut *(virt as *mut PageTable)
}

/// Map a virtual page to a physical page, dynamically allocating page tables as needed
pub unsafe fn map_page(
    virt_page: usize,
    phys_frame: usize,
    flags: PageTableFlags,
) -> Result<(), &'static str> {
    let l4_table = active_level4_table();
    
    let l4_index = (virt_page >> 39) & 0x1FF;
    let l3_index = (virt_page >> 30) & 0x1FF;
    let l2_index = (virt_page >> 21) & 0x1FF;
    let l1_index = (virt_page >> 12) & 0x1FF;

    // 1. Get or create Level 3 table
    let l3_table_ptr = if l4_table[l4_index].is_unused() {
        let layout = Layout::from_size_align(4096, 4096).unwrap();
        let alloc_ptr = alloc::alloc::alloc_zeroed(layout);
        if alloc_ptr.is_null() {
            return Err("Failed to allocate L3 page table");
        }
        let phys_addr = alloc_ptr as u64;
        l4_table[l4_index].set_addr(
            PhysAddr::new(phys_addr),
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE,
        );
        phys_addr as *mut PageTable
    } else {
        l4_table[l4_index].addr().as_u64() as *mut PageTable
    };

    let l3_table = &mut *l3_table_ptr;

    // 2. Get or create Level 2 table
    let l2_table_ptr = if l3_table[l3_index].is_unused() {
        let layout = Layout::from_size_align(4096, 4096).unwrap();
        let alloc_ptr = alloc::alloc::alloc_zeroed(layout);
        if alloc_ptr.is_null() {
            return Err("Failed to allocate L2 page table");
        }
        let phys_addr = alloc_ptr as u64;
        l3_table[l3_index].set_addr(
            PhysAddr::new(phys_addr),
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE,
        );
        phys_addr as *mut PageTable
    } else {
        l3_table[l3_index].addr().as_u64() as *mut PageTable
    };

    let l2_table = &mut *l2_table_ptr;

    // 3. Get or create Level 1 table
    let l1_table_ptr = if l2_table[l2_index].is_unused() {
        let layout = Layout::from_size_align(4096, 4096).unwrap();
        let alloc_ptr = alloc::alloc::alloc_zeroed(layout);
        if alloc_ptr.is_null() {
            return Err("Failed to allocate L1 page table");
        }
        let phys_addr = alloc_ptr as u64;
        l2_table[l2_index].set_addr(
            PhysAddr::new(phys_addr),
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE,
        );
        phys_addr as *mut PageTable
    } else {
        l2_table[l2_index].addr().as_u64() as *mut PageTable
    };

    let l1_table = &mut *l1_table_ptr;

    // 4. Map virtual page to physical frame in Level 1 table
    l1_table[l1_index].set_addr(
        PhysAddr::new(phys_frame as u64),
        flags,
    );

    // Flush TLB for this virtual address
    x86_64::instructions::tlb::flush(x86_64::VirtAddr::new(virt_page as u64));

    Ok(())
}
