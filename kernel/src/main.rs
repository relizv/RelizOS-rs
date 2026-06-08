#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]

pub mod gop;
pub mod ata;
pub mod relizfs;
pub mod task;
pub mod interrupts;
pub mod gdt;
pub mod syscall;

use bootloader_api::{entry_point, BootInfo};
use core::panic::PanicInfo;
use relizfs::RelizFsReader;

entry_point!(kernel_main);

// Pre-allocate 4 KiB stacks in the BSS segment for task execution
static mut USER_STACK_ALPHA: [u8; 4096] = [0; 4096];
static mut KERNEL_STACK_ALPHA: [u8; 4096] = [0; 4096];

static mut USER_STACK_BETA: [u8; 4096] = [0; 4096];
static mut KERNEL_STACK_BETA: [u8; 4096] = [0; 4096];

/// User Space Task Alpha - Executes in Ring 3!
/// Prints messages and yields using the raw CPU `syscall` instruction.
fn task_user_alpha() -> ! {
    let msg = "[User Space Task Alpha] Hello from Ring 3 via syscall!\n";
    let msg_ptr = msg.as_ptr() as u64;
    let msg_len = msg.len() as u64;
    
    loop {
        // Invoke Syscall 1 (Print String)
        unsafe {
            core::arch::asm!(
                "syscall",
                in("rax") 1u64,     // Syscall number 1
                in("rdi") msg_ptr,  // Arg 1: buffer pointer
                in("rsi") msg_len,  // Arg 2: buffer length
                out("rcx") _,       // Overwritten by CPU with user RIP
                out("r11") _,       // Overwritten by CPU with user RFLAGS
            );
        }
        
        // Spin to slow down logging
        for _ in 0..40_000_000 {
            core::hint::spin_loop();
        }

        // Invoke Syscall 2 (Yield)
        unsafe {
            core::arch::asm!(
                "syscall",
                in("rax") 2u64,     // Syscall number 2
                out("rcx") _,
                out("r11") _,
            );
        }
    }
}

/// User Space Task Beta - Executes in Ring 3!
/// Prints messages and yields using the raw CPU `syscall` instruction.
fn task_user_beta() -> ! {
    let msg = "[User Space Task Beta] Hello from Ring 3 via syscall!\n";
    let msg_ptr = msg.as_ptr() as u64;
    let msg_len = msg.len() as u64;
    
    loop {
        // Invoke Syscall 1 (Print String)
        unsafe {
            core::arch::asm!(
                "syscall",
                in("rax") 1u64,     // Syscall number 1
                in("rdi") msg_ptr,  // Arg 1: buffer pointer
                in("rsi") msg_len,  // Arg 2: buffer length
                out("rcx") _,
                out("r11") _,
            );
        }
        
        // Spin to slow down logging
        for _ in 0..40_000_000 {
            core::hint::spin_loop();
        }

        // Invoke Syscall 2 (Yield)
        unsafe {
            core::arch::asm!(
                "syscall",
                in("rax") 2u64,     // Syscall number 2
                out("rcx") _,
                out("r11") _,
            );
        }
    }
}

fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
    // 1. Initialize GOP writer
    if let Some(framebuffer) = boot_info.framebuffer.as_mut() {
        let info = framebuffer.info();
        let buffer = framebuffer.buffer_mut();
        
        let writer = gop::FrameBufferWriter::new(buffer, info);
        *gop::WRITER.lock() = Some(writer);
    }

    // Print booting banner
    println!("==========================================================");
    println!("              RelizOS-Rust v0.1.0 Booting                 ");
    println!("==========================================================");
    println!("[ OK ] CPU Mode: x86_64 Long Mode (64-bit)");
    println!("[ OK ] Firmware: UEFI Boot Services Active");
    println!("[ OK ] GOP: Graphics Framebuffer mapping successful");
    println!("----------------------------------------------------------");
    
    // 2. Initialize GDT, TSS and Syscall extensions
    println!("Initializing GDT and Task State Segment (TSS)...");
    gdt::init();
    println!("[ OK ] GDT loaded. TSS loaded. RSP0 privilege stack mapped.");

    println!("Initializing fast system calls (syscall/sysret)...");
    syscall::init();
    println!("[ OK ] STAR, LSTAR, FMASK registers mapped. Syscall active.");

    // 3. Load RelizFS reader from Primary Master drive
    println!("Mounting RelizFS file system...");
    match RelizFsReader::init() {
        Ok(fs) => {
            let sb = fs.superblock();
            let magic = sb.magic;
            let total_blocks = sb.total_blocks;
            let inode_count = sb.inode_count;
            let data_blocks_start = sb.data_blocks_start;
            println!("[ OK ] RelizFS mounted successfully!");
            println!("       FS Magic:       0x{:X}", magic);
            println!("       Total Blocks:   {}", total_blocks);
            println!("       Inode Count:    {}", inode_count);
            println!("       Data Blocks @:  sector {}", data_blocks_start);
            println!("----------------------------------------------------------");
            
            // Read root directory (inode 0)
            println!("Root Directory [/] Listing:");
            if let Ok(root_inode) = fs.read_inode(0) {
                let _ = fs.list_directory(&root_inode);
            }
        }
        Err(e) => {
            println!("[ERROR] Failed to mount RelizFS: {}", e);
        }
    }
    println!("----------------------------------------------------------");

    // 4. Initialize multitasking scheduler state
    println!("Initializing task scheduler...");
    
    // Create and register Ring 3 tasks inside a separate scope
    {
        let task_a = unsafe { task::Task::new_user(1, task_user_alpha, &mut USER_STACK_ALPHA, &mut KERNEL_STACK_ALPHA) };
        let task_b = unsafe { task::Task::new_user(2, task_user_beta, &mut USER_STACK_BETA, &mut KERNEL_STACK_BETA) };

        let mut sched = task::SCHEDULER.lock();
        sched.spawn(task_a).expect("Failed to spawn Task Alpha");
        sched.spawn(task_b).expect("Failed to spawn Task Beta");
    }
    println!("[ OK ] Spawning Task Alpha (ID 1) & Task Beta (ID 2) in Ring 3");

    // 5. Load IDT and start hardware timer interrupts
    println!("Initializing IDT and PIC timer interrupts...");
    interrupts::init();
    println!("[ OK ] IDT loaded. PIC remapped to vector 0x20.");
    println!("Starting preemptive scheduler...");
    println!("----------------------------------------------------------");

    // Start the first task (this will jump to task_user_alpha in user space and automatically enable interrupts)
    unsafe {
        task::start_first_task();
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    if let Some(ref mut writer) = *gop::WRITER.lock() {
        writer.set_colors((255, 120, 120), (50, 5, 5));
        writer.clear();
    }
    println!("==========================================================");
    println!("                  !!! KERNEL PANIC !!!                    ");
    println!("==========================================================");
    println!("");
    println!("{}", info);
    println!("");
    println!("----------------------------------------------------------");
    println!("System halted. Please restart your system.");

    loop {
        x86_64::instructions::hlt();
    }
}
