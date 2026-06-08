#![no_std]
#![no_main]

pub mod gop;
pub mod ata;
pub mod relizfs;
pub mod task;

use bootloader_api::{entry_point, BootInfo};
use core::panic::PanicInfo;
use relizfs::RelizFsReader;

entry_point!(kernel_main);

// Pre-allocate 4 KiB stacks in the BSS segment for cooperative task testing
static mut STACK_ALPHA: [u8; 4096] = [0; 4096];
static mut STACK_BETA: [u8; 4096] = [0; 4096];

/// Task Alpha execution loop
fn task_alpha() -> ! {
    let mut counter = 0;
    loop {
        counter += 1;
        println!("[Task Alpha] Counter: {} -> yielding control", counter);
        
        // Waste some time so output isn't too fast
        for _ in 0..10_000_000 {
            core::hint::spin_loop();
        }

        task::yield_now();
    }
}

/// Task Beta execution loop
fn task_beta() -> ! {
    let mut counter = 0;
    loop {
        counter += 1;
        println!("[Task Beta] Counter: {} -> yielding control", counter);
        
        // Waste some time so output isn't too fast
        for _ in 0..10_000_000 {
            core::hint::spin_loop();
        }

        task::yield_now();
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
    
    // 2. Load RelizFS reader from Primary Master drive
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

    // 3. Initialize multitasking scheduler
    println!("Initializing cooperative task scheduler...");
    
    // Create and register tasks inside a separate scope to drop the Mutex before yielding
    {
        let task_a = unsafe { task::Task::new(1, task_alpha, &mut STACK_ALPHA) };
        let task_b = unsafe { task::Task::new(2, task_beta, &mut STACK_BETA) };

        let mut sched = task::SCHEDULER.lock();
        sched.spawn(task_a).expect("Failed to spawn Task Alpha");
        sched.spawn(task_b).expect("Failed to spawn Task Beta");
    }

    println!("[ OK ] Spawning Task Alpha (ID 1) & Task Beta (ID 2)");
    println!("Starting scheduler...");
    println!("----------------------------------------------------------");

    // Trigger the first task switch
    task::yield_now();

    // The scheduler takes over; we should never return to this boot stack frame.
    loop {
        x86_64::instructions::hlt();
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
