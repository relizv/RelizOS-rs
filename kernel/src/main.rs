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

static mut USER_STACK_ATA: [u8; 4096] = [0; 4096];
static mut KERNEL_STACK_ATA: [u8; 4096] = [0; 4096];

static mut USER_STACK_KBD: [u8; 4096] = [0; 4096];
static mut KERNEL_STACK_KBD: [u8; 4096] = [0; 4096];

/// Convert PS/2 scancode to ASCII characters
fn scancode_to_ascii(scancode: u8) -> Option<char> {
    match scancode {
        0x1E => Some('a'), 0x30 => Some('b'), 0x2E => Some('c'), 0x20 => Some('d'),
        0x12 => Some('e'), 0x21 => Some('f'), 0x22 => Some('g'), 0x23 => Some('h'),
        0x17 => Some('i'), 0x24 => Some('j'), 0x25 => Some('k'), 0x26 => Some('l'),
        0x32 => Some('m'), 0x31 => Some('n'), 0x18 => Some('o'), 0x19 => Some('p'),
        0x10 => Some('q'), 0x13 => Some('r'), 0x1F => Some('s'), 0x14 => Some('t'),
        0x16 => Some('u'), 0x2F => Some('v'), 0x11 => Some('w'), 0x2D => Some('x'),
        0x15 => Some('y'), 0x2C => Some('z'),
        0x02 => Some('1'), 0x03 => Some('2'), 0x04 => Some('3'), 0x05 => Some('4'),
        0x06 => Some('5'), 0x07 => Some('6'), 0x08 => Some('7'), 0x09 => Some('8'),
        0x0A => Some('9'), 0x0B => Some('0'),
        0x1C => Some('\n'),
        0x39 => Some(' '),
        0x0E => Some('\x08'),
        0x34 => Some('.'),
        0x0C => Some('-'),
        0x35 => Some('/'),
        _ => None,
    }
}

/// User Space Keyboard Server - Executes in Ring 3 with IOPL = 3!
/// Polls the PS/2 controller status and data ports, decoding and pushing keypress events via IPC.
fn task_keyboard_server() -> ! {
    use task::Message;
    
    let mut status_port = x86_64::instructions::port::PortReadOnly::<u8>::new(0x64);
    let mut data_port = x86_64::instructions::port::PortReadOnly::<u8>::new(0x60);
    
    let mut last_scancode = 0;

    loop {
        let status = unsafe { status_port.read() };
        if (status & 0x01) != 0 {
            let scancode = unsafe { data_port.read() };
            
            // Only handle key press events (ignore key release, which has bit 7 set to 1)
            if (scancode & 0x80) == 0 && scancode != last_scancode {
                last_scancode = scancode;
                
                // Map scancode to ASCII
                if let Some(c) = scancode_to_ascii(scancode) {
                    let msg = Message {
                        sender: 4, // Keyboard ID is 4
                        msg_type: 20, // MSG_KEY_EVENT
                        arg1: c as u64,
                        arg2: 0, arg3: 0, arg4: 0,
                    };
                    
                    // Send to Shell Task (ID 1)
                    unsafe {
                        core::arch::asm!(
                            "syscall",
                            in("rax") 3u64, // Send
                            in("rdi") 1u64, // Dest: Shell (Task 1)
                            in("rsi") &msg as *const Message as u64,
                            out("rcx") _, out("r11") _,
                        );
                    }
                }
            } else if (scancode & 0x80) != 0 {
                // Key released
                last_scancode = 0;
            }
        }

        // Yield to prevent pegging 100% CPU on polling
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
    use shared::{Superblock, Inode, DirectoryEntry, RELIZFS_MAGIC, BLOCK_SIZE};
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

/// User Space Interactive Shell - Executes in Ring 3!
/// Renders command prompt, reads keyboard characters via IPC, and executes commands.
fn task_shell() -> ! {
    use task::Message;
    
    // Print startup message
    let welcome = "\n==========================================================\n\
                     Welcome to RelizOS Interactive Shell!                     \n\
                   ==========================================================\n\
                   Type 'help' to see list of available commands.\n\n";
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") 1u64,
            in("rdi") welcome.as_ptr() as u64,
            in("rsi") welcome.len() as u64,
            out("rcx") _, out("r11") _,
        );
    }

    let prompt = "relizos> ";
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") 1u64,
            in("rdi") prompt.as_ptr() as u64,
            in("rsi") prompt.len() as u64,
            out("rcx") _, out("r11") _,
        );
    }

    let mut cmd_buf = [0u8; 64];
    let mut cmd_len = 0;
    
    let mut msg = Message {
        sender: 0, msg_type: 0, arg1: 0, arg2: 0, arg3: 0, arg4: 0,
    };

    loop {
        // Wait for key press event from Keyboard Server (Task 4)
        unsafe {
            core::arch::asm!(
                "syscall",
                in("rax") 4u64,     // Syscall 4: Recv
                in("rdi") 0u64,     // 0 = Receive from ANY
                in("rsi") &mut msg as *mut Message as u64,
                out("rcx") _, out("r11") _,
            );
        }

        if msg.msg_type == 20 { // MSG_KEY_EVENT
            let c = msg.arg1 as u8 as char;
            
            if c == '\n' {
                // Print newline
                let nl = "\n";
                unsafe {
                    core::arch::asm!(
                        "syscall",
                        in("rax") 1u64,
                        in("rdi") nl.as_ptr() as u64,
                        in("rsi") nl.len() as u64,
                        out("rcx") _, out("r11") _,
                    );
                }

                // Process command
                if cmd_len > 0 {
                    let cmd_slice = &cmd_buf[..cmd_len];
                    if let Ok(cmd_str) = core::str::from_utf8(cmd_slice) {
                        let trimmed = cmd_str.trim();
                        if trimmed == "help" {
                            let help_menu = "Available commands:\n\
                                               help           - Show this help menu\n\
                                               ls             - List directory files\n\
                                               cat <filename> - Print file contents\n\
                                               clear          - Clear the screen\n";
                            unsafe {
                                core::arch::asm!(
                                    "syscall",
                                    in("rax") 1u64,
                                    in("rdi") help_menu.as_ptr() as u64,
                                    in("rsi") help_menu.len() as u64,
                                    out("rcx") _, out("r11") _,
                                );
                            }
                        } else if trimmed == "ls" {
                            let mut fs_buf = [0u8; 1024];
                            let req = Message {
                                sender: 1, // Our ID is 1
                                msg_type: 10, // MSG_FS_LIST
                                arg1: fs_buf.as_mut_ptr() as u64,
                                arg2: fs_buf.len() as u64,
                                arg3: 0, arg4: 0,
                            };
                            let mut resp = Message {
                                sender: 0, msg_type: 0, arg1: 0, arg2: 0, arg3: 0, arg4: 0,
                            };
                            unsafe {
                                core::arch::asm!(
                                    "syscall",
                                    in("rax") 3u64, // Send to FS Server (2)
                                    in("rdi") 2u64,
                                    in("rsi") &req as *const Message as u64,
                                    out("rcx") _, out("r11") _,
                                );
                                core::arch::asm!(
                                    "syscall",
                                    in("rax") 4u64, // Recv response
                                    in("rdi") 2u64,
                                    in("rsi") &mut resp as *mut Message as u64,
                                    out("rcx") _, out("r11") _,
                                );
                            }
                            if resp.msg_type == 0 {
                                let mut len = 0;
                                while fs_buf[len] != 0 && len < fs_buf.len() {
                                    len += 1;
                                }
                                unsafe {
                                    core::arch::asm!(
                                        "syscall",
                                        in("rax") 1u64,
                                        in("rdi") fs_buf.as_ptr() as u64,
                                        in("rsi") len as u64,
                                        out("rcx") _, out("r11") _,
                                    );
                                }
                            } else {
                                let err_msg = "Error reading directory listing!\n";
                                unsafe {
                                    core::arch::asm!(
                                        "syscall",
                                        in("rax") 1u64,
                                        in("rdi") err_msg.as_ptr() as u64,
                                        in("rsi") err_msg.len() as u64,
                                        out("rcx") _, out("r11") _,
                                    );
                                }
                            }
                        } else if trimmed.starts_with("cat ") {
                            let filename = trimmed.strip_prefix("cat ").unwrap_or("").trim();
                            if !filename.is_empty() {
                                let mut f_buf = [0u8; 32];
                                let f_bytes = filename.as_bytes();
                                let copy_len = core::cmp::min(f_bytes.len(), 31);
                                f_buf[..copy_len].copy_from_slice(&f_bytes[..copy_len]);
                                f_buf[copy_len] = 0;

                                let mut file_content = [0u8; 512];
                                let req = Message {
                                    sender: 1,
                                    msg_type: 11, // MSG_FS_READ
                                    arg1: f_buf.as_ptr() as u64,
                                    arg2: file_content.as_mut_ptr() as u64,
                                    arg3: file_content.len() as u64,
                                    arg4: 0,
                                };
                                let mut resp = Message {
                                    sender: 0, msg_type: 0, arg1: 0, arg2: 0, arg3: 0, arg4: 0,
                                };
                                unsafe {
                                    core::arch::asm!(
                                        "syscall",
                                        in("rax") 3u64, // Send to FS Server (2)
                                        in("rdi") 2u64,
                                        in("rsi") &req as *const Message as u64,
                                        out("rcx") _, out("r11") _,
                                    );
                                    core::arch::asm!(
                                        "syscall",
                                        in("rax") 4u64, // Recv response
                                        in("rdi") 2u64,
                                        in("rsi") &mut resp as *mut Message as u64,
                                        out("rcx") _, out("r11") _,
                                    );
                                }
                                if resp.msg_type == 0 {
                                    let read_bytes = resp.arg1 as usize;
                                    unsafe {
                                        core::arch::asm!(
                                            "syscall",
                                            in("rax") 1u64,
                                            in("rdi") file_content.as_ptr() as u64,
                                            in("rsi") read_bytes as u64,
                                            out("rcx") _, out("r11") _,
                                        );
                                    }
                                    let extra_nl = "\n";
                                    unsafe {
                                        core::arch::asm!(
                                            "syscall",
                                            in("rax") 1u64,
                                            in("rdi") extra_nl.as_ptr() as u64,
                                            in("rsi") extra_nl.len() as u64,
                                            out("rcx") _, out("r11") _,
                                        );
                                    }
                                } else {
                                    let err_msg = "File not found!\n";
                                    unsafe {
                                        core::arch::asm!(
                                            "syscall",
                                            in("rax") 1u64,
                                            in("rdi") err_msg.as_ptr() as u64,
                                            in("rsi") err_msg.len() as u64,
                                            out("rcx") _, out("r11") _,
                                        );
                                    }
                                }
                            }
                        } else if trimmed == "clear" {
                            let clear_cmd = "\x0C";
                            unsafe {
                                core::arch::asm!(
                                    "syscall",
                                    in("rax") 1u64,
                                    in("rdi") clear_cmd.as_ptr() as u64,
                                    in("rsi") clear_cmd.len() as u64,
                                    out("rcx") _, out("r11") _,
                                );
                            }
                        } else {
                            let unknown = "Unknown command. Type 'help' for options.\n";
                            unsafe {
                                core::arch::asm!(
                                    "syscall",
                                    in("rax") 1u64,
                                    in("rdi") unknown.as_ptr() as u64,
                                    in("rsi") unknown.len() as u64,
                                    out("rcx") _, out("r11") _,
                                );
                            }
                        }
                    }
                }
                
                cmd_len = 0;

                unsafe {
                    core::arch::asm!(
                        "syscall",
                        in("rax") 1u64,
                        in("rdi") prompt.as_ptr() as u64,
                        in("rsi") prompt.len() as u64,
                        out("rcx") _, out("r11") _,
                    );
                }
            } else if c == '\x08' {
                if cmd_len > 0 {
                    cmd_len -= 1;
                    let bs = "\x08";
                    unsafe {
                        core::arch::asm!(
                            "syscall",
                            in("rax") 1u64,
                            in("rdi") bs.as_ptr() as u64,
                            in("rsi") bs.len() as u64,
                            out("rcx") _, out("r11") _,
                        );
                    }
                }
            } else {
                if cmd_len < cmd_buf.len() {
                    cmd_buf[cmd_len] = c as u8;
                    cmd_len += 1;
                    
                    let char_str = [c as u8];
                    unsafe {
                        core::arch::asm!(
                            "syscall",
                            in("rax") 1u64,
                            in("rdi") char_str.as_ptr() as u64,
                            in("rsi") 1u64,
                            out("rcx") _, out("r11") _,
                        );
                    }
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
        let task_a = unsafe { task::Task::new_user(1, task_shell, &mut USER_STACK_ALPHA, &mut KERNEL_STACK_ALPHA, false) };
        let task_b = unsafe { task::Task::new_user(2, task_fs_server, &mut USER_STACK_BETA, &mut KERNEL_STACK_BETA, false) };
        let task_ata = unsafe { task::Task::new_user(3, task_ata_server, &mut USER_STACK_ATA, &mut KERNEL_STACK_ATA, true) };
        let task_kbd = unsafe { task::Task::new_user(4, task_keyboard_server, &mut USER_STACK_KBD, &mut KERNEL_STACK_KBD, true) };

        let mut sched = task::SCHEDULER.lock();
        sched.spawn(task_a).expect("Failed to spawn Shell");
        sched.spawn(task_b).expect("Failed to spawn FS Server");
        sched.spawn(task_ata).expect("Failed to spawn ATA Server");
        sched.spawn(task_kbd).expect("Failed to spawn Keyboard Server");
    }
    println!("[ OK ] Spawning Shell (1), FS Server (2), ATA Server (3), Keyboard Server (4)");

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
