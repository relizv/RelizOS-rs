use crate::task::Message;
use shared::Superblock;

/// Convert PS/2 scancode to ASCII characters
pub fn scancode_to_ascii(scancode: u8) -> Option<char> {
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

/// Helper to format integers into a byte buffer
fn format_num(mut val: usize, buf: &mut [u8]) -> usize {
    let mut idx = buf.len();
    if val == 0 {
        idx -= 1;
        buf[idx] = b'0';
        return idx;
    }
    while val > 0 {
        idx -= 1;
        buf[idx] = b'0' + (val % 10) as u8;
        val /= 10;
    }
    idx
}

/// Print a string slice using Syscall 1 (Print String)
pub fn print_str(s: &str) {
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") 1u64,
            in("rdi") s.as_ptr() as u64,
            in("rsi") s.len() as u64,
            out("rcx") _, out("r11") _,
        );
    }
}

/// User Space Interactive Shell - Executes in Ring 3!
pub fn task_shell() -> ! {
    // Print startup message
    let welcome = "\n==========================================================\n\
                     Welcome to RelizOS Interactive Shell!                     \n\
                   ==========================================================\n\
                   Type 'help' to see list of available commands.\n\n";
    print_str(welcome);

    let prompt = "relizos> ";
    print_str(prompt);

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
                print_str("\n");

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
                                               clear          - Clear the screen\n\
                                               free           - Show heap allocator memory stats\n\
                                               sysinfo        - Display active system tasks and info\n\
                                               dump <lba>     - Hex/ASCII dump of specified disk sector\n\
                                               echo <text>    - Print back the entered text\n\
                                               uptime         - Show system uptime in seconds\n\
                                               gopinfo        - Display graphics parameters\n\
                                               gfxdemo        - Launch Nyan Cat space graphics demo!\n\
                                               diskinfo       - Show disk partition and RelizFS info\n\
                                               reboot         - Reboot the machine\n\
                                               shutdown       - Shutdown the machine\n";
                            print_str(help_menu);
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
                                print_str("Error reading directory listing!\n");
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
                                    print_str("\n");
                                } else {
                                    print_str("File not found!\n");
                                }
                            }
                        } else if trimmed == "clear" {
                            print_str("\x0C");
                        } else if trimmed == "free" {
                            let mut used_mem: usize = 0;
                            unsafe {
                                core::arch::asm!(
                                    "syscall",
                                    in("rax") 5u64, // Syscall 5: Get Used Memory
                                    out("rcx") _, out("r11") _,
                                    lateout("rax") used_mem,
                                );
                            }
                            
                            let total = 1024 * 1024;
                            let free = total - used_mem;
                            
                            let mut buf_total = [0u8; 20];
                            let mut buf_used = [0u8; 20];
                            let mut buf_free = [0u8; 20];
                            
                            let idx_total = format_num(total, &mut buf_total);
                            let idx_used = format_num(used_mem, &mut buf_used);
                            let idx_free = format_num(free, &mut buf_free);
                            
                            let mut output = [0u8; 128];
                            let mut out_len = 0;
                            let mut append_out = |s: &str| {
                                let bytes = s.as_bytes();
                                let len = core::cmp::min(bytes.len(), output.len() - out_len);
                                output[out_len..out_len+len].copy_from_slice(&bytes[..len]);
                                out_len += len;
                            };
                            
                            // Check that USED_MEMORY works
                            append_out("Heap Memory Stats:\n  Total size: ");
                            append_out(core::str::from_utf8(&buf_total[idx_total..]).unwrap_or(""));
                            append_out(" bytes\n  Used size:  ");
                            append_out(core::str::from_utf8(&buf_used[idx_used..]).unwrap_or(""));
                            append_out(" bytes\n  Free size:  ");
                            append_out(core::str::from_utf8(&buf_free[idx_free..]).unwrap_or(""));
                            append_out(" bytes\n");
                            
                            unsafe {
                                core::arch::asm!(
                                    "syscall",
                                    in("rax") 1u64,
                                    in("rdi") output.as_ptr() as u64,
                                    in("rsi") out_len as u64,
                                    out("rcx") _, out("r11") _,
                                );
                            }
                        } else if trimmed == "sysinfo" {
                            let mut task_buf = [0u8; 512];
                            let mut copied_bytes: usize = 0;
                            unsafe {
                                core::arch::asm!(
                                    "syscall",
                                    in("rax") 6u64, // Syscall 6: Get Task Info
                                    in("rdi") task_buf.as_mut_ptr() as u64,
                                    in("rsi") task_buf.len() as u64,
                                    out("rcx") _, out("r11") _,
                                    lateout("rax") copied_bytes,
                                );
                            }
                            
                            let sys_meta = "System Information:\n  OS:           RelizOS-Rust v0.1.0\n  Architecture: x86_64 UEFI\n  Heap Limit:   1024 KiB\n\nActive Tasks:\n";
                            print_str(sys_meta);
                            unsafe {
                                core::arch::asm!(
                                    "syscall",
                                    in("rax") 1u64,
                                    in("rdi") task_buf.as_ptr() as u64,
                                    in("rsi") copied_bytes as u64,
                                    out("rcx") _, out("r11") _,
                                );
                            }
                        } else if trimmed.starts_with("dump ") {
                            let lba_str = trimmed.strip_prefix("dump ").unwrap_or("").trim();
                            let mut lba = 0;
                            for b in lba_str.bytes() {
                                if b >= b'0' && b <= b'9' {
                                    lba = lba * 10 + (b - b'0') as u32;
                                } else {
                                    break;
                                }
                            }
                            
                            let mut sector_buf = [0u8; 512];
                            let req = Message {
                                sender: 1,
                                msg_type: 1, // MSG_ATA_READ
                                arg1: lba as u64,
                                arg2: sector_buf.as_mut_ptr() as u64,
                                arg3: 0, arg4: 0,
                            };
                            let mut resp = Message {
                                sender: 0, msg_type: 0, arg1: 0, arg2: 0, arg3: 0, arg4: 0,
                            };
                            
                            unsafe {
                                core::arch::asm!(
                                    "syscall",
                                    in("rax") 3u64, // Send to ATA (3)
                                    in("rdi") 3u64,
                                    in("rsi") &req as *const Message as u64,
                                    out("rcx") _, out("r11") _,
                                );
                                core::arch::asm!(
                                    "syscall",
                                    in("rax") 4u64, // Recv response
                                    in("rdi") 3u64,
                                    in("rsi") &mut resp as *mut Message as u64,
                                    out("rcx") _, out("r11") _,
                                );
                            }
                            
                            if resp.msg_type == 0 {
                                for line in 0..32 {
                                    let mut line_buf = [0u8; 80];
                                    let mut cursor = 0;
                                    let offset = line * 16;
                                    let hex_chars = b"0123456789ABCDEF";
                                    let offset_str = [
                                        b'0',
                                        hex_chars[(offset >> 8) & 0xF],
                                        hex_chars[(offset >> 4) & 0xF],
                                        hex_chars[offset & 0xF],
                                        b':', b' ',
                                    ];
                                    line_buf[cursor..cursor+6].copy_from_slice(&offset_str);
                                    cursor += 6;
                                    
                                    for i in 0..16 {
                                        let b = sector_buf[offset + i];
                                        let byte_str = [
                                            hex_chars[(b >> 4) as usize],
                                            hex_chars[(b & 0xF) as usize],
                                            b' ',
                                        ];
                                        line_buf[cursor..cursor+3].copy_from_slice(&byte_str);
                                        cursor += 3;
                                    }
                                    
                                    line_buf[cursor..cursor+2].copy_from_slice(b" |");
                                    cursor += 2;
                                    
                                    for i in 0..16 {
                                        let b = sector_buf[offset + i];
                                        let c = if b >= 32 && b <= 126 { b } else { b'.' };
                                        line_buf[cursor] = c;
                                        cursor += 1;
                                    }
                                    
                                    line_buf[cursor..cursor+2].copy_from_slice(b"|\n");
                                    cursor += 2;
                                    
                                    unsafe {
                                        core::arch::asm!(
                                            "syscall",
                                            in("rax") 1u64,
                                            in("rdi") line_buf.as_ptr() as u64,
                                            in("rsi") cursor as u64,
                                            out("rcx") _, out("r11") _,
                                        );
                                    }
                                }
                            } else {
                                print_str("Error reading sector from disk!\n");
                            }
                        } else if trimmed.starts_with("echo ") {
                            let text = trimmed.strip_prefix("echo ").unwrap_or("");
                            print_str(text);
                            print_str("\n");
                        } else if trimmed == "uptime" {
                            let mut ticks: usize = 0;
                            unsafe {
                                core::arch::asm!(
                                    "syscall",
                                    in("rax") 7u64, // Syscall 7: Get Uptime Ticks
                                    out("rcx") _, out("r11") _,
                                    lateout("rax") ticks,
                                );
                            }
                            // Assuming typical PIT rate of ~18.2 Hz
                            let seconds = ticks / 18;
                            
                            let mut buf_sec = [0u8; 20];
                            let mut buf_ticks = [0u8; 20];
                            let idx_sec = format_num(seconds, &mut buf_sec);
                            let idx_ticks = format_num(ticks, &mut buf_ticks);
                            
                            print_str("System Uptime: ");
                            print_str(core::str::from_utf8(&buf_sec[idx_sec..]).unwrap_or(""));
                            print_str(" seconds (");
                            print_str(core::str::from_utf8(&buf_ticks[idx_ticks..]).unwrap_or(""));
                            print_str(" raw ticks)\n");
                        } else if trimmed == "gopinfo" {
                            let mut info_buf = [0usize; 4]; // width, height, stride, bytes_per_pixel
                            let mut ret: usize = 0;
                            unsafe {
                                core::arch::asm!(
                                    "syscall",
                                    in("rax") 8u64, // Syscall 8: Get GOP Info
                                    in("rdi") info_buf.as_mut_ptr() as u64,
                                    out("rcx") _, out("r11") _,
                                    lateout("rax") ret,
                                );
                            }
                            if ret == 0 {
                                let mut buf_w = [0u8; 20];
                                let mut buf_h = [0u8; 20];
                                let mut buf_s = [0u8; 20];
                                let mut buf_bpp = [0u8; 20];
                                
                                let idx_w = format_num(info_buf[0], &mut buf_w);
                                let idx_h = format_num(info_buf[1], &mut buf_h);
                                let idx_s = format_num(info_buf[2], &mut buf_s);
                                let idx_bpp = format_num(info_buf[3], &mut buf_bpp);
                                
                                print_str("Graphics GOP Settings:\n  Resolution:      ");
                                print_str(core::str::from_utf8(&buf_w[idx_w..]).unwrap_or(""));
                                print_str("x");
                                print_str(core::str::from_utf8(&buf_h[idx_h..]).unwrap_or(""));
                                print_str("\n  Scanline Stride: ");
                                print_str(core::str::from_utf8(&buf_s[idx_s..]).unwrap_or(""));
                                print_str(" pixels\n  Bytes per Pixel: ");
                                print_str(core::str::from_utf8(&buf_bpp[idx_bpp..]).unwrap_or(""));
                                print_str("\n");
                            } else {
                                print_str("Failed to query GOP graphics info!\n");
                            }
                        } else if trimmed == "gfxdemo" {
                            print_str("Launching Nyan Cat Graphics Demo... (Enjoy!)\n");
                            unsafe {
                                core::arch::asm!(
                                    "syscall",
                                    in("rax") 11u64, // Syscall 11: Run Gfx Demo
                                    out("rcx") _, out("r11") _,
                                );
                            }
                        } else if trimmed == "diskinfo" {
                            // Let's read MBR (Sector 0) and Superblock (Sector 40002) via ATA server IPC
                            let mut mbr_sector = [0u8; 512];
                            let req_mbr = Message {
                                sender: 1,
                                msg_type: 1, // MSG_ATA_READ
                                arg1: 0, // MBR LBA
                                arg2: mbr_sector.as_mut_ptr() as u64,
                                arg3: 0, arg4: 0,
                            };
                            let mut resp_mbr = Message {
                                sender: 0, msg_type: 0, arg1: 0, arg2: 0, arg3: 0, arg4: 0,
                            };
                            
                            unsafe {
                                core::arch::asm!(
                                    "syscall",
                                    in("rax") 3u64, // Send to ATA (3)
                                    in("rdi") 3u64,
                                    in("rsi") &req_mbr as *const Message as u64,
                                    out("rcx") _, out("r11") _,
                                );
                                core::arch::asm!(
                                    "syscall",
                                    in("rax") 4u64, // Recv response
                                    in("rdi") 3u64,
                                    in("rsi") &mut resp_mbr as *mut Message as u64,
                                    out("rcx") _, out("r11") _,
                                );
                            }
                            
                            let mut sb_sector = [0u8; 512];
                            let req_sb = Message {
                                sender: 1,
                                msg_type: 1, // MSG_ATA_READ
                                arg1: 40002, // Superblock LBA
                                arg2: sb_sector.as_mut_ptr() as u64,
                                arg3: 0, arg4: 0,
                            };
                            let mut resp_sb = Message {
                                sender: 0, msg_type: 0, arg1: 0, arg2: 0, arg3: 0, arg4: 0,
                            };
                            
                            unsafe {
                                core::arch::asm!(
                                    "syscall",
                                    in("rax") 3u64,
                                    in("rdi") 3u64,
                                    in("rsi") &req_sb as *const Message as u64,
                                    out("rcx") _, out("r11") _,
                                );
                                core::arch::asm!(
                                    "syscall",
                                    in("rax") 4u64,
                                    in("rdi") 3u64,
                                    in("rsi") &mut resp_sb as *mut Message as u64,
                                    out("rcx") _, out("r11") _,
                                );
                            }
                            
                            if resp_mbr.msg_type == 0 && resp_sb.msg_type == 0 {
                                // Extract partition start from MBR entry 1 (offset 446)
                                let partition_start = unsafe {
                                    let ptr = mbr_sector.as_ptr().add(446 + 8) as *const u32;
                                    core::ptr::read_unaligned(ptr)
                                };
                                let partition_sectors = unsafe {
                                    let ptr = mbr_sector.as_ptr().add(446 + 12) as *const u32;
                                    core::ptr::read_unaligned(ptr)
                                };
                                
                                let sb = unsafe { *(sb_sector.as_ptr() as *const Superblock) };
                                
                                let mut buf_start = [0u8; 20];
                                let mut buf_sect = [0u8; 20];
                                let mut buf_blocks = [0u8; 20];
                                let mut buf_inodes = [0u8; 20];
                                
                                let idx_start = format_num(partition_start as usize, &mut buf_start);
                                let idx_sect = format_num(partition_sectors as usize, &mut buf_sect);
                                let idx_blocks = format_num(sb.total_blocks as usize, &mut buf_blocks);
                                let idx_inodes = format_num(sb.inode_count as usize, &mut buf_inodes);
                                
                                print_str("Primary Disk Storage Info:\n");
                                print_str("  ATA Drive Channel: Primary Master (0)\n");
                                print_str("  Partition 1 LBA:   ");
                                print_str(core::str::from_utf8(&buf_start[idx_start..]).unwrap_or(""));
                                print_str("\n  Partition Sectors: ");
                                print_str(core::str::from_utf8(&buf_sect[idx_sect..]).unwrap_or(""));
                                print_str("\n\nMount volume: RelizFS File System\n");
                                print_str("  Superblock Magic:  0x");
                                
                                // Hex output for magic
                                let hex_chars = b"0123456789ABCDEF";
                                let mut magic_str = [0u8; 16];
                                for i in 0..8 {
                                    let b = ((sb.magic >> (56 - i * 8)) & 0xFF) as u8;
                                    magic_str[i * 2] = hex_chars[(b >> 4) as usize];
                                    magic_str[i * 2 + 1] = hex_chars[(b & 0xF) as usize];
                                }
                                print_str(core::str::from_utf8(&magic_str).unwrap_or(""));
                                print_str("\n  Total FS Blocks:   ");
                                print_str(core::str::from_utf8(&buf_blocks[idx_blocks..]).unwrap_or(""));
                                print_str("\n  Total FS Inodes:   ");
                                print_str(core::str::from_utf8(&buf_inodes[idx_inodes..]).unwrap_or(""));
                                print_str("\n");
                            } else {
                                print_str("Failed to query drive parameters via ATA Server IPC!\n");
                            }
                        } else if trimmed == "reboot" {
                            print_str("Rebooting system...\n");
                            unsafe {
                                core::arch::asm!(
                                    "syscall",
                                    in("rax") 9u64, // Syscall 9: Reboot
                                    out("rcx") _, out("r11") _,
                                );
                            }
                        } else if trimmed == "shutdown" {
                            print_str("Shutting down RelizOS-Rust...\n");
                            unsafe {
                                core::arch::asm!(
                                    "syscall",
                                    in("rax") 10u64, // Syscall 10: Shutdown
                                    out("rcx") _, out("r11") _,
                                );
                            }
                        } else {
                            print_str("Unknown command. Type 'help' for options.\n");
                        }
                    }
                }
                
                cmd_len = 0;
                print_str(prompt);
            } else if c == '\x08' {
                if cmd_len > 0 {
                    cmd_len -= 1;
                    print_str("\x08");
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
