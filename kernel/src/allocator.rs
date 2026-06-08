use core::alloc::{GlobalAlloc, Layout};
use core::ptr;
use spin::Mutex;

struct FreeNode {
    size: usize,
    next: *mut FreeNode,
}

unsafe impl Send for FreeNode {}
unsafe impl Sync for FreeNode {}

pub struct HeapAllocator {
    head: Mutex<FreeNode>,
    initialized: Mutex<bool>,
}

impl HeapAllocator {
    pub const fn new() -> Self {
        Self {
            head: Mutex::new(FreeNode {
                size: 0,
                next: ptr::null_mut(),
            }),
            initialized: Mutex::new(false),
        }
    }

    pub unsafe fn init(&self, start: usize, size: usize) {
        let mut head = self.head.lock();
        let mut init = self.initialized.lock();
        if *init {
            return;
        }

        let first_node = start as *mut FreeNode;
        ptr::write(first_node, FreeNode {
            size: size - core::mem::size_of::<FreeNode>(),
            next: ptr::null_mut(),
        });

        head.next = first_node;
        *init = true;
    }
}

fn align_up(addr: usize, align: usize) -> usize {
    (addr + align - 1) & !(align - 1)
}

unsafe impl GlobalAlloc for HeapAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let mut head = self.head.lock();
        
        let size = align_up(layout.size(), 16);
        let align = align_up(layout.align(), 16);
        
        let mut prev = &raw mut *head;
        let mut curr = head.next;

        while !curr.is_null() {
            let node = &mut *curr;
            let alloc_start = align_up(curr as usize + core::mem::size_of::<FreeNode>(), align);
            let needed_space = alloc_start - curr as usize + size;

            if node.size + core::mem::size_of::<FreeNode>() >= needed_space {
                let next_node = node.next;
                let remaining = (node.size + core::mem::size_of::<FreeNode>()) - needed_space;

                if remaining >= core::mem::size_of::<FreeNode>() + 16 {
                    let new_node_ptr = (alloc_start + size) as *mut FreeNode;
                    ptr::write(new_node_ptr, FreeNode {
                        size: remaining - core::mem::size_of::<FreeNode>(),
                        next: next_node,
                    });
                    (*prev).next = new_node_ptr;
                } else {
                    (*prev).next = next_node;
                }

                unsafe {
                    USED_MEMORY += layout.size();
                }
                return alloc_start as *mut u8;
            }

            prev = &raw mut *curr;
            curr = node.next;
        }

        ptr::null_mut()
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let mut head = self.head.lock();
        let node_ptr = (ptr as usize - core::mem::size_of::<FreeNode>()) as *mut FreeNode;
        let size = align_up(layout.size(), 16);

        ptr::write(node_ptr, FreeNode {
            size,
            next: head.next,
        });

        head.next = node_ptr;

        unsafe {
            USED_MEMORY -= layout.size();
        }
    }
}

#[global_allocator]
pub static ALLOCATOR: HeapAllocator = HeapAllocator::new();

#[alloc_error_handler]
fn alloc_error_handler(layout: core::alloc::Layout) -> ! {
    panic!("Allocation error: {:?}", layout);
}

pub static mut USED_MEMORY: usize = 0;
