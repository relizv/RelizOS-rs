use x86_64::structures::paging::{PageTable, PageTableFlags};
use x86_64::registers::control::Cr3;
use x86_64::PhysAddr;
use spin::Mutex;
use bootloader_api::info::{MemoryRegion, MemoryRegionKind};

// ---------------------------------------------------------------------------
// Physical memory offset (set once during init, read many times after)
// ---------------------------------------------------------------------------

static mut PHYS_MEM_OFFSET: u64 = 0;

/// Convert a physical address to a virtual address using the bootloader's
/// physical-memory offset mapping.
#[inline]
pub fn phys_to_virt(phys: u64) -> u64 {
    phys + unsafe { PHYS_MEM_OFFSET }
}

// ---------------------------------------------------------------------------
// Frame allocator — hands out physical 4 KiB frames from boot memory map
// ---------------------------------------------------------------------------

/// Simple bump allocator that walks through the usable memory regions
/// reported by the bootloader and returns one 4 KiB frame at a time.
pub struct BootInfoFrameAllocator {
    regions: *const MemoryRegion,
    region_count: usize,
    next_region: usize,
    next_frame_in_region: u64,
}

unsafe impl Send for BootInfoFrameAllocator {}
unsafe impl Sync for BootInfoFrameAllocator {}

impl BootInfoFrameAllocator {
    /// Create a new frame allocator from a raw pointer to the memory region
    /// array and its length.  Caller guarantees the slice remains valid for
    /// the lifetime of the allocator.
    pub fn new(regions: *const MemoryRegion, count: usize) -> Self {
        Self {
            regions,
            region_count: count,
            next_region: 0,
            next_frame_in_region: 0,
        }
    }

    fn region(&self, idx: usize) -> &MemoryRegion {
        unsafe { &*self.regions.add(idx) }
    }

    /// Allocate one 4 KiB physical frame.  Returns the *physical* address of
    /// the start of the frame, or `None` if all usable memory is exhausted.
    pub fn allocate_frame(&mut self) -> Option<u64> {
        loop {
            if self.next_region >= self.region_count {
                return None;
            }
            let region = self.region(self.next_region);
            if region.kind != MemoryRegionKind::Usable {
                self.next_region += 1;
                self.next_frame_in_region = 0;
                continue;
            }
            let start_frame = (region.start + 0xFFF) >> 12; // align up
            let end_frame = region.end >> 12;
            let frame = start_frame + self.next_frame_in_region;
            if frame < end_frame {
                self.next_frame_in_region += 1;
                return Some(frame << 12);
            } else {
                self.next_region += 1;
                self.next_frame_in_region = 0;
            }
        }
    }
}

pub static FRAME_ALLOCATOR: Mutex<Option<BootInfoFrameAllocator>> = Mutex::new(None);

/// Allocate a physical frame, zero it, and return its physical address.
fn alloc_zeroed_frame() -> Result<u64, &'static str> {
    let mut guard = FRAME_ALLOCATOR.lock();
    let alloc = guard.as_mut().ok_or("Frame allocator not initialized")?;
    let phys = alloc.allocate_frame().ok_or("Out of physical frames")?;
    unsafe {
        core::ptr::write_bytes(phys_to_virt(phys) as *mut u8, 0, 4096);
    }
    Ok(phys)
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Initialise the paging subsystem.
///
/// * `phys_offset` — virtual-to-physical offset from `boot_info.physical_memory_offset`
/// * `regions_ptr` / `regions_len` — pointer and length of `boot_info.memory_regions`
pub unsafe fn init(phys_offset: u64, regions_ptr: *const MemoryRegion, regions_len: usize) {
    PHYS_MEM_OFFSET = phys_offset;
    *FRAME_ALLOCATOR.lock() = Some(BootInfoFrameAllocator::new(regions_ptr, regions_len));
}

/// Retrieve the active Level 4 page table by reading CR3 and translating
/// the physical address to virtual via the bootloader offset.
pub unsafe fn active_level4_table() -> &'static mut PageTable {
    let (level4_table_frame, _) = Cr3::read();
    let phys = level4_table_frame.start_address().as_u64();
    let virt = phys_to_virt(phys);
    &mut *(virt as *mut PageTable)
}

/// Map a virtual page to a physical frame, dynamically allocating
/// intermediate page tables from the frame allocator as needed.
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
    if l4_table[l4_index].is_unused() {
        let frame_phys = alloc_zeroed_frame()?;
        l4_table[l4_index].set_addr(
            PhysAddr::new(frame_phys),
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE,
        );
    }
    let l3_table = &mut *(phys_to_virt(l4_table[l4_index].addr().as_u64()) as *mut PageTable);

    // 2. Get or create Level 2 table
    if l3_table[l3_index].is_unused() {
        let frame_phys = alloc_zeroed_frame()?;
        l3_table[l3_index].set_addr(
            PhysAddr::new(frame_phys),
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE,
        );
    }
    let l2_table = &mut *(phys_to_virt(l3_table[l3_index].addr().as_u64()) as *mut PageTable);

    // 3. Get or create Level 1 table
    if l2_table[l2_index].is_unused() {
        let frame_phys = alloc_zeroed_frame()?;
        l2_table[l2_index].set_addr(
            PhysAddr::new(frame_phys),
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE,
        );
    }
    let l1_table = &mut *(phys_to_virt(l2_table[l2_index].addr().as_u64()) as *mut PageTable);

    // 4. Map virtual page to physical frame in Level 1 table
    l1_table[l1_index].set_addr(
        PhysAddr::new(phys_frame as u64),
        flags,
    );

    // Flush TLB for this virtual address
    x86_64::instructions::tlb::flush(x86_64::VirtAddr::new(virt_page as u64));

    Ok(())
}

/// Walk the entire 4-level page table hierarchy and set the
/// `USER_ACCESSIBLE` flag on every present entry at every level.
///
/// This is the "quick-and-dirty" approach so that Ring 3 tasks (whose code
/// still lives in the kernel binary) can actually execute without #PF.
/// A proper OS would load user ELFs into dedicated user-mode pages instead.
pub unsafe fn mark_all_user_accessible() {
    let l4 = active_level4_table();

    for i4 in 0..512 {
        let flags4 = l4[i4].flags();
        if !flags4.contains(PageTableFlags::PRESENT) {
            continue;
        }
        let addr4 = l4[i4].addr();
        l4[i4].set_addr(addr4, flags4 | PageTableFlags::USER_ACCESSIBLE);

        let l3 = &mut *(phys_to_virt(addr4.as_u64()) as *mut PageTable);
        for i3 in 0..512 {
            let flags3 = l3[i3].flags();
            if !flags3.contains(PageTableFlags::PRESENT) {
                continue;
            }
            let addr3 = l3[i3].addr();
            l3[i3].set_addr(addr3, flags3 | PageTableFlags::USER_ACCESSIBLE);

            // 1 GiB huge page — no L2 table beneath
            if flags3.contains(PageTableFlags::HUGE_PAGE) {
                continue;
            }

            let l2 = &mut *(phys_to_virt(addr3.as_u64()) as *mut PageTable);
            for i2 in 0..512 {
                let flags2 = l2[i2].flags();
                if !flags2.contains(PageTableFlags::PRESENT) {
                    continue;
                }
                let addr2 = l2[i2].addr();
                l2[i2].set_addr(addr2, flags2 | PageTableFlags::USER_ACCESSIBLE);

                // 2 MiB huge page — no L1 table beneath
                if flags2.contains(PageTableFlags::HUGE_PAGE) {
                    continue;
                }

                let l1 = &mut *(phys_to_virt(addr2.as_u64()) as *mut PageTable);
                for i1 in 0..512 {
                    let flags1 = l1[i1].flags();
                    if !flags1.contains(PageTableFlags::PRESENT) {
                        continue;
                    }
                    let addr1 = l1[i1].addr();
                    l1[i1].set_addr(addr1, flags1 | PageTableFlags::USER_ACCESSIBLE);
                }
            }
        }
    }

    x86_64::instructions::tlb::flush_all();
}
