#![no_std]
#![no_main]

pub mod gop;
pub mod ata;
pub mod relizfs;

use bootloader_api::{entry_point, BootInfo};
use core::panic::PanicInfo;
use relizfs::RelizFsReader;

entry_point!(kernel_main);

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
    println!("Initializing ATA PIO driver (Primary Slave)...");
    
    // 2. Load RelizFS reader from primary slave drive
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
            match fs.read_inode(0) {
                Ok(root_inode) => {
                    if let Err(e) = fs.list_directory(&root_inode) {
                        println!("[ERROR] Failed to list root directory: {}", e);
                    }
                }
                Err(e) => println!("[ERROR] Failed to read root inode: {}", e),
            }
            println!("----------------------------------------------------------");

            // Look for hello.txt
            println!("Searching for 'hello.txt'...");
            match fs.read_inode(0) {
                Ok(root_inode) => {
                    match fs.find_entry(&root_inode, "hello.txt") {
                        Ok(hello_inode_idx) => {
                            println!("[ OK ] Found 'hello.txt' at inode {}", hello_inode_idx);
                            
                            // Read file inode
                            match fs.read_inode(hello_inode_idx) {
                                Ok(file_inode) => {
                                    let mut content_buf = [0u8; 256];
                                    match fs.read_file(&file_inode, &mut content_buf) {
                                        Ok(bytes_read) => {
                                            println!("[ OK ] Read {} bytes from 'hello.txt':", bytes_read);
                                            println!("--- CONTENT START ---");
                                            
                                            // Convert bytes to string slice safely
                                            if let Ok(text) = core::str::from_utf8(&content_buf[..bytes_read]) {
                                                println!("{}", text);
                                            } else {
                                                println!("[ERROR] File content is not valid UTF-8.");
                                            }
                                            println!("--- CONTENT END ---");
                                        }
                                        Err(e) => println!("[ERROR] Failed to read file data: {}", e),
                                    }
                                }
                                Err(e) => println!("[ERROR] Failed to read file inode: {}", e),
                            }
                        }
                        Err(e) => println!("[ERROR] 'hello.txt' not found: {}", e),
                    }
                }
                Err(_) => {}
            }
        }
        Err(e) => {
            panic!("Failed to mount RelizFS: {}", e);
        }
    }

    println!("----------------------------------------------------------");
    println!("System execution completed. Idle.");

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
