#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]
#![feature(alloc_error_handler)]

extern crate alloc;

pub mod gop;
pub mod ata;
pub mod relizfs;
pub mod task;
pub mod interrupts;
pub mod gdt;
pub mod syscall;
pub mod allocator;
pub mod paging;
pub mod shell;
pub mod gui;

use bootloader_api::{entry_point, BootInfo};
use core::panic::PanicInfo;
use relizfs::RelizFsReader;

entry_point!(kernel_main);

// Pre-allocate 16 MiB heap memory buffer
static mut HEAP_MEM: [u8; 16 * 1024 * 1024] = [0; 16 * 1024 * 1024];

// Pre-allocate 4 KiB stacks in the BSS segment for task execution
static mut USER_STACK_ALPHA: [u8; 4096] = [0; 4096];
static mut KERNEL_STACK_ALPHA: [u8; 4096] = [0; 4096];

static mut USER_STACK_BETA: [u8; 4096] = [0; 4096];
static mut KERNEL_STACK_BETA: [u8; 4096] = [0; 4096];

static mut USER_STACK_ATA: [u8; 4096] = [0; 4096];
static mut KERNEL_STACK_ATA: [u8; 4096] = [0; 4096];

static mut USER_STACK_KBD: [u8; 4096] = [0; 4096];
static mut KERNEL_STACK_KBD: [u8; 4096] = [0; 4096];

static mut USER_STACK_GUI: [u8; 4096] = [0; 4096];
static mut KERNEL_STACK_GUI: [u8; 4096] = [0; 4096];

/// User Space Input Server - Executes in Ring 3 with IOPL = 3!
/// Polls the PS/2 controller, initializing and parsing keyboard and mouse packet streams, sending to GUI Server (5) via IPC.
fn task_input_server() -> ! {
    use task::Message;
    
    // Command register (0x64) and data register (0x60)
    let mut status_port = x86_64::instructions::port::Port::<u8>::new(0x64);
    let mut data_port = x86_64::instructions::port::Port::<u8>::new(0x60);
    
    fn wait_write(status_port: &mut x86_64::instructions::port::Port<u8>) {
        let mut timeout = 100000;
        while timeout > 0 {
            if unsafe { status_port.read() & 0x02 } == 0 {
                break;
            }
            timeout -= 1;
        }
    }
    
    fn wait_read(status_port: &mut x86_64::instructions::port::Port<u8>) {
        let mut timeout = 100000;
        while timeout > 0 {
            if unsafe { status_port.read() & 0x01 } != 0 {
                break;
            }
            timeout -= 1;
        }
    }

    unsafe {
        // 1. Enable auxiliary mouse device
        wait_write(&mut status_port);
        status_port.write(0xA8);
        
        // 2. Enable mouse interrupts in controller configuration byte
        wait_write(&mut status_port);
        status_port.write(0x20); // Get config byte
        wait_read(&mut status_port);
        let mut config = data_port.read();
        config |= 0x02; // Enable IRQ 12
        config &= !0x20; // Enable mouse clocks
        
        wait_write(&mut status_port);
        status_port.write(0x60); // Set config byte
        wait_write(&mut status_port);
        data_port.write(config);
        
        // 3. Set mouse to defaults
        wait_write(&mut status_port);
        status_port.write(0xD4);
        wait_write(&mut status_port);
        data_port.write(0xF6);
        wait_read(&mut status_port);
        let _ = data_port.read(); // ACK (0xFA)
        
        // 4. Enable packet streaming
        wait_write(&mut status_port);
        status_port.write(0xD4);
        wait_write(&mut status_port);
        data_port.write(0xF4);
        wait_read(&mut status_port);
        let _ = data_port.read(); // ACK (0xFA)
    }

    let mut last_scancode = 0;
    let mut mouse_cycle = 0;
    let mut mouse_packet = [0u8; 3];

    loop {
        let status = unsafe { status_port.read() };
        if (status & 0x01) != 0 {
            if (status & 0x20) != 0 {
                // Mouse byte
                let b = unsafe { data_port.read() };
                if mouse_cycle == 0 && (b & 0x08) == 0 {
                    // Out of sync
                    continue;
                }
                mouse_packet[mouse_cycle] = b;
                mouse_cycle += 1;
                
                if mouse_cycle == 3 {
                    mouse_cycle = 0;
                    let flags = mouse_packet[0];
                    let left_click = (flags & 0x01) != 0;
                    let right_click = (flags & 0x02) != 0;
                    let mut dx = mouse_packet[1] as i32;
                    let mut dy = mouse_packet[2] as i32;
                    
                    if (flags & 0x10) != 0 {
                        dx |= !0xFF;
                    }
                    if (flags & 0x20) != 0 {
                        dy |= !0xFF;
                    }
                    
                    // Mouse packet event message (type 30)
                    let mouse_msg = Message {
                        sender: 4,
                        msg_type: 30, // MSG_MOUSE_EVENT
                        arg1: dx as u64,
                        arg2: dy as u64,
                        arg3: left_click as u64,
                        arg4: right_click as u64,
                    };
                    
                    unsafe {
                        core::arch::asm!(
                            "syscall",
                            in("rax") 3u64, // Send
                            in("rdi") 5u64, // Dest: GUI Server (Task 5)
                            in("rsi") &mouse_msg as *const Message as u64,
                            out("rcx") _, out("r11") _,
                        );
                    }
                }
            } else {
                // Keyboard byte
                let scancode = unsafe { data_port.read() };
                if (scancode & 0x80) == 0 && scancode != last_scancode {
                    last_scancode = scancode;
                    if let Some(c) = shell::scancode_to_ascii(scancode) {
                        let msg = Message {
                            sender: 4,
                            msg_type: 20, // MSG_KEY_EVENT
                            arg1: c as u64,
                            arg2: 0, arg3: 0, arg4: 0,
                        };
                        
                        unsafe {
                            core::arch::asm!(
                                "syscall",
                                in("rax") 3u64, // Send
                                in("rdi") 5u64, // Dest: GUI Server (Task 5)
                                in("rsi") &msg as *const Message as u64,
                                out("rcx") _, out("r11") _,
                            );
                        }
                    }
                } else if (scancode & 0x80) != 0 {
                    last_scancode = 0;
                }
            }
        }

        // Yield to prevent pegging CPU
        unsafe {
            core::arch::asm!(
                "syscall",
                in("rax") 2u64, // Yield
                out("rcx") _, out("r11") _,
            );
        }
    }
}

/// User Space ATA Server - Executes in Ring 3 with IOPL = 3!
/// Serves ATA read sector requests over synchronous IPC from other tasks.
fn task_ata_server() -> ! {
    use task::Message;
    
    let mut msg = Message {
        sender: 0,
        msg_type: 0,
        arg1: 0,
        arg2: 0,
        arg3: 0,
        arg4: 0,
    };
    
    let mut buffer = [0u8; 512];

    loop {
        // Wait for a message from ANY task (filter = 0)
        unsafe {
            core::arch::asm!(
                "syscall",
                in("rax") 4u64,     // Syscall 4: Recv
                in("rdi") 0u64,     // 0 = Receive from ANY
                in("rsi") &mut msg as *mut Message as u64,
                out("rcx") _,
                out("r11") _,
            );
        }

        let client = msg.sender;

        if msg.msg_type == 1 { // MSG_ATA_READ
            let lba = msg.arg1 as u32;
            let client_buf_ptr = msg.arg2 as *mut [u8; 512];

            // Perform port I/O disk read directly using the ATA driver
            let read_res = crate::ata::ATA_DRIVE.lock().read_sector(0, lba, &mut buffer);

            if read_res.is_ok() {
                // Copy the read data to the client's buffer
                unsafe {
                    core::ptr::copy_nonoverlapping(buffer.as_ptr(), client_buf_ptr as *mut u8, 512);
                }
                
                // Send MSG_ATA_OK response
                let response = Message {
                    sender: 3, // Our ID is 3
                    msg_type: 0, // MSG_ATA_OK
                    arg1: 0, arg2: 0, arg3: 0, arg4: 0,
                };
                unsafe {
                    core::arch::asm!(
                        "syscall",
                        in("rax") 3u64, // Syscall 3: Send
                        in("rdi") client as u64,
                        in("rsi") &response as *const Message as u64,
                        out("rcx") _,
                        out("r11") _,
                    );
                }
            } else {
                // Send MSG_ATA_ERROR response
                let response = Message {
                    sender: 3,
                    msg_type: 999, // MSG_ATA_ERROR
                    arg1: 0, arg2: 0, arg3: 0, arg4: 0,
                };
                unsafe {
                    core::arch::asm!(
                        "syscall",
                        in("rax") 3u64, // Syscall 3: Send
                        in("rdi") client as u64,
                        in("rsi") &response as *const Message as u64,
                        out("rcx") _,
                        out("r11") _,
                    );
                }
            }
        }
    }
}

/// User Space FS Server - Executes in Ring 3!
/// Parses filesystem partition layout over disk read messages.
fn task_fs_server() -> ! {
    use task::Message;
    use shared::{Superblock, Inode, DirectoryEntry};
    use core::ptr;

    // Local helper to read a sector via the ATA Server over IPC
    fn fs_read_sector(lba: u32, buf_ptr: *mut [u8; 512]) -> Result<(), &'static str> {
        let req = Message {
            sender: 2, // FS ID is 2
            msg_type: 1, // MSG_ATA_READ
            arg1: lba as u64,
            arg2: buf_ptr as u64,
            arg3: 0, arg4: 0,
        };
        let mut resp = Message {
            sender: 0, msg_type: 0, arg1: 0, arg2: 0, arg3: 0, arg4: 0,
        };
        
        unsafe {
            core::arch::asm!(
                "syscall",
                in("rax") 3u64, // Send
                in("rdi") 3u64, // Dest: ATA Server (3)
                in("rsi") &req as *const Message as u64,
                out("rcx") _, out("r11") _,
            );
            core::arch::asm!(
                "syscall",
                in("rax") 4u64, // Recv
                in("rdi") 3u64, // Filter: only ATA Server (3)
                in("rsi") &mut resp as *mut Message as u64,
                out("rcx") _, out("r11") _,
            );
        }
        
        if resp.msg_type == 0 {
            Ok(())
        } else {
            Err("ATA read sector failed")
        }
    }

    let mut msg = Message {
        sender: 0, msg_type: 0, arg1: 0, arg2: 0, arg3: 0, arg4: 0,
    };

    loop {
        // Wait for an IPC message from ANY task (filter = 0)
        unsafe {
            core::arch::asm!(
                "syscall",
                in("rax") 4u64,     // Syscall 4: Recv
                in("rdi") 0u64,     // 0 = Receive from ANY
                in("rsi") &mut msg as *mut Message as u64,
                out("rcx") _, out("r11") _,
            );
        }

        let client = msg.sender;

        if msg.msg_type == 10 { // MSG_FS_LIST
            let client_buf = msg.arg1 as *mut u8;
            let max_len = msg.arg2 as usize;
            
            let mut list_res = [0u8; 1024];
            let mut cursor = 0;
            
            let mut append = |s: &str| {
                let bytes = s.as_bytes();
                let len = core::cmp::min(bytes.len(), list_res.len() - cursor);
                list_res[cursor..cursor+len].copy_from_slice(&bytes[..len]);
                cursor += len;
            };

            // Read superblock
            let mut sector = [0u8; 512];
            if fs_read_sector(40000 + 2, &mut sector).is_ok() {
                let sb = unsafe { *(sector.as_ptr() as *const Superblock) };
                
                // Read root inode (0)
                let target_sector = sb.inode_table_start;
                let mut sector_buf = [0u8; 512];
                if fs_read_sector(40000 + target_sector, &mut sector_buf).is_ok() {
                    let root_inode = unsafe { *(sector_buf.as_ptr() as *const Inode) };
                    let root_block = root_inode.direct_blocks[0];
                    
                    let mut dir_sector = [0u8; 512];
                    if fs_read_sector(40000 + root_block, &mut dir_sector).is_ok() {
                        // Loop through 16 entries (each is 32 bytes)
                        for i in 0..16 {
                            let entry_ptr = unsafe { (dir_sector.as_ptr() as *const DirectoryEntry).add(i) };
                            let entry = unsafe { ptr::read_unaligned(entry_ptr) };
                            
                            if entry.inode_num != 0 && entry.name_len > 0 {
                                let len = entry.name_len as usize;
                                let active_len = core::cmp::min(len, 27);
                                if let Ok(name) = core::str::from_utf8(&entry.name[..active_len]) {
                                    let byte_offset = (entry.inode_num as usize) * core::mem::size_of::<Inode>();
                                    let s_offset = byte_offset / 512;
                                    let b_index = byte_offset % 512;
                                    let inode_sector = sb.inode_table_start + (s_offset as u32);
                                    
                                    let mut inode_buf = [0u8; 512];
                                    if fs_read_sector(40000 + inode_sector, &mut inode_buf).is_ok() {
                                        let ent_inode = unsafe {
                                            let ptr = inode_buf.as_ptr().add(b_index) as *const Inode;
                                            ptr.read_unaligned()
                                        };
                                        if ent_inode.is_directory() {
                                            append("[DIR]  ");
                                        } else {
                                            append("[FILE] ");
                                        }
                                        append(name);
                                        if ent_inode.is_file() {
                                            append(" (");
                                            let mut size_buf = [0u8; 20];
                                            let mut size = ent_inode.size;
                                            let mut s_idx = 20;
                                            if size == 0 {
                                                s_idx -= 1;
                                                size_buf[s_idx] = b'0';
                                            } else {
                                                while size > 0 {
                                                    s_idx -= 1;
                                                    size_buf[s_idx] = b'0' + (size % 10) as u8;
                                                    size /= 10;
                                                }
                                            }
                                            if let Ok(size_str) = core::str::from_utf8(&size_buf[s_idx..]) {
                                                append(size_str);
                                            }
                                            append(" bytes)");
                                        }
                                        append("\n");
                                    }
                                }
                            }
                        }
                    }
                }
            }
            
            if cursor < list_res.len() {
                list_res[cursor] = 0;
                cursor += 1;
            }
            
            unsafe {
                core::ptr::copy_nonoverlapping(list_res.as_ptr(), client_buf, core::cmp::min(cursor, max_len));
            }
            
            let response = Message {
                sender: 2, msg_type: 0, arg1: 0, arg2: 0, arg3: 0, arg4: 0,
            };
            unsafe {
                core::arch::asm!(
                    "syscall",
                    in("rax") 3u64, // Send
                    in("rdi") client as u64,
                    in("rsi") &response as *const Message as u64,
                    out("rcx") _, out("r11") _,
                );
            }
        } else if msg.msg_type == 11 { // MSG_FS_READ
            let filename_ptr = msg.arg1 as *const u8;
            let client_buf = msg.arg2 as *mut u8;
            let max_len = msg.arg3 as usize;

            let mut filename_len = 0;
            unsafe {
                while *filename_ptr.add(filename_len) != 0 && filename_len < 32 {
                    filename_len += 1;
                }
            }
            let filename_slice = unsafe { core::slice::from_raw_parts(filename_ptr, filename_len) };
            let target_filename = core::str::from_utf8(filename_slice).unwrap_or("");

            let mut file_found = false;
            let mut bytes_copied = 0;

            let mut sector = [0u8; 512];
            if fs_read_sector(40000 + 2, &mut sector).is_ok() {
                let sb = unsafe { *(sector.as_ptr() as *const Superblock) };
                
                let target_sector = sb.inode_table_start;
                let mut sector_buf = [0u8; 512];
                if fs_read_sector(40000 + target_sector, &mut sector_buf).is_ok() {
                    let root_inode = unsafe { *(sector_buf.as_ptr() as *const Inode) };
                    let root_block = root_inode.direct_blocks[0];
                    
                    let mut dir_sector = [0u8; 512];
                    if fs_read_sector(40000 + root_block, &mut dir_sector).is_ok() {
                        for i in 0..16 {
                            let entry_ptr = unsafe { (dir_sector.as_ptr() as *const DirectoryEntry).add(i) };
                            let entry = unsafe { ptr::read_unaligned(entry_ptr) };
                            
                            if entry.inode_num != 0 && entry.name_len > 0 {
                                let len = entry.name_len as usize;
                                let active_len = core::cmp::min(len, 27);
                                if let Ok(name) = core::str::from_utf8(&entry.name[..active_len]) {
                                    if name == target_filename {
                                        let byte_offset = (entry.inode_num as usize) * core::mem::size_of::<Inode>();
                                        let s_offset = byte_offset / 512;
                                        let b_index = byte_offset % 512;
                                        let inode_sector = sb.inode_table_start + (s_offset as u32);
                                        
                                        let mut inode_buf = [0u8; 512];
                                        if fs_read_sector(40000 + inode_sector, &mut inode_buf).is_ok() {
                                            let file_inode = unsafe {
                                                let ptr = inode_buf.as_ptr().add(b_index) as *const Inode;
                                                ptr.read_unaligned()
                                            };
                                            
                                            let file_block = file_inode.direct_blocks[0];
                                            let mut data_sector = [0u8; 512];
                                            if fs_read_sector(40000 + file_block, &mut data_sector).is_ok() {
                                                file_found = true;
                                                let copy_size = core::cmp::min(file_inode.size as usize, max_len);
                                                unsafe {
                                                    core::ptr::copy_nonoverlapping(data_sector.as_ptr(), client_buf, copy_size);
                                                    if copy_size < max_len {
                                                        *client_buf.add(copy_size) = 0;
                                                    }
                                                }
                                                bytes_copied = copy_size;
                                            }
                                        }
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if file_found {
                let response = Message {
                    sender: 2, msg_type: 0,
                    arg1: bytes_copied as u64,
                    arg2: 0, arg3: 0, arg4: 0,
                };
                unsafe {
                    core::arch::asm!(
                        "syscall",
                        in("rax") 3u64,
                        in("rdi") client as u64,
                        in("rsi") &response as *const Message as u64,
                        out("rcx") _, out("r11") _,
                    );
                }
            } else {
                let response = Message {
                    sender: 2, msg_type: 999,
                    arg1: 0, arg2: 0, arg3: 0, arg4: 0,
                };
                unsafe {
                    core::arch::asm!(
                        "syscall",
                        in("rax") 3u64,
                        in("rdi") client as u64,
                        in("rsi") &response as *const Message as u64,
                        out("rcx") _, out("r11") _,
                    );
                }
            }
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

    // Initialize Heap Allocator
    println!("Initializing Heap Memory Allocator (16 MiB)...");
    unsafe {
        allocator::ALLOCATOR.init(&raw mut HEAP_MEM as usize, 16 * 1024 * 1024);
    }
    println!("[ OK ] Heap allocator initialized successfully.");

    // Test dynamic allocation
    {
        use alloc::vec::Vec;
        use alloc::boxed::Box;
        let mut v = Vec::new();
        v.push(42);
        v.push(1337);
        let b = Box::new(777);
        println!("[ OK ] Heap test successful: Box={:?}, Vec={:?}", b, v);
    }

    // Test VMM mapping
    println!("Testing Virtual Memory Manager (VMM)...");
    let test_virt_page = 0x_0000_7777_0000_0000usize;
    // Allocate a physical frame for our page mapping test
    let phys_frame = unsafe {
        let layout = core::alloc::Layout::from_size_align(4096, 4096).unwrap();
        alloc::alloc::alloc_zeroed(layout) as usize
    };
    
    unsafe {
        use x86_64::structures::paging::PageTableFlags;
        let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE;
        if paging::map_page(test_virt_page, phys_frame, flags).is_ok() {
            let ptr = test_virt_page as *mut u64;
            core::ptr::write(ptr, 0xDEADC0DE);
            let val = core::ptr::read(ptr);
            println!("[ OK ] VMM mapping successful! Mapped virt 0x{:X} -> phys 0x{:X}, verified value = 0x{:X}", test_virt_page, phys_frame, val);
        } else {
            println!("[ERROR] VMM page mapping failed!");
        }
    }
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
        // Spawning Task Alpha and Beta without I/O privilege, and ATA Server with IOPL = 3
        let task_a = unsafe { task::Task::new_user(1, shell::task_shell, &mut USER_STACK_ALPHA, &mut KERNEL_STACK_ALPHA, false) };
        let task_b = unsafe { task::Task::new_user(2, task_fs_server, &mut USER_STACK_BETA, &mut KERNEL_STACK_BETA, false) };
        let task_ata = unsafe { task::Task::new_user(3, task_ata_server, &mut USER_STACK_ATA, &mut KERNEL_STACK_ATA, true) };
        let task_input = unsafe { task::Task::new_user(4, task_input_server, &mut USER_STACK_KBD, &mut KERNEL_STACK_KBD, true) };
        let task_gui = unsafe { task::Task::new_user(5, gui::task_gui_server, &mut USER_STACK_GUI, &mut KERNEL_STACK_GUI, false) };

        let mut sched = task::SCHEDULER.lock();
        sched.spawn(task_a).expect("Failed to spawn Shell");
        sched.spawn(task_b).expect("Failed to spawn FS Server");
        sched.spawn(task_ata).expect("Failed to spawn ATA Server");
        sched.spawn(task_input).expect("Failed to spawn Input Server");
        sched.spawn(task_gui).expect("Failed to spawn GUI Server");
    }
    println!("[ OK ] Spawning Shell (1), FS Server (2), ATA Server (3), Input Server (4), GUI Server (5)");

    // 5. Load IDT and start hardware timer interrupts
    println!("Initializing IDT and PIC timer interrupts...");
    interrupts::init();
    println!("[ OK ] IDT loaded. PIC remapped to vector 0x20.");
    println!("Starting preemptive scheduler...");
    println!("----------------------------------------------------------");

    // Start the first task (this will jump to task_shell in user space and automatically enable interrupts)
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
