pub mod compositor;
pub mod renderer;
pub mod layout;

use crate::task::Message;
use compositor::Compositor;
use layout::LayoutManager;

/// GUI server task executing in Ring 3
pub fn task_gui_server() -> ! {
    // 1. Query GOP resolution via Syscall 8
    let mut gop_buf = [0usize; 4]; // width, height, stride, bytes_per_pixel
    let mut ret: usize = 0;
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") 8u64, // Syscall 8: Get GOP Info
            in("rdi") gop_buf.as_mut_ptr() as u64,
            out("rcx") _, out("r11") _,
            lateout("rax") ret,
        );
    }

    let (width, height) = if ret == 0 {
        (gop_buf[0], gop_buf[1])
    } else {
        (1024, 768) // Fallback default
    };

    // 2. Initialize graphics components
    let mut compositor = Compositor::new(width, height);
    let mut layout_manager = LayoutManager::new(width, height);
    
    // Add default mock windows for tiling and centered dock demo
    layout_manager.add_window(1); // Terminal Shell
    layout_manager.add_window(2); // File Manager
    layout_manager.add_window(3); // System Monitor
    compositor.mark_dirty(0, 0, width, height);
    
    // Position mouse cursor initially in center
    let mut mouse_x = width / 2;
    let mut mouse_y = height / 2;

    let mut msg = Message {
        sender: 0,
        msg_type: 0,
        arg1: 0,
        arg2: 0,
        arg3: 0,
        arg4: 0,
    };

    loop {
        // 1. Drain all pending messages using non-blocking Try Recv
        loop {
            let ret: usize;
            unsafe {
                core::arch::asm!(
                    "syscall",
                    in("rax") 14u64,    // Syscall 14: Try Recv (non-blocking)
                    in("rdi") 0u64,     // 0 = from ANY sender
                    in("rsi") &mut msg as *mut Message as u64,
                    lateout("rax") ret,
                    out("rcx") _, out("r11") _,
                );
            }
            if ret != 0 { break; } // no more pending messages

            let sender = msg.sender;

            match msg.msg_type {
                20 => {
                    // MSG_KEY_EVENT: arg1 is char
                    let c = msg.arg1 as u8 as char;
                    
                    // Route keyboard character to current active window
                    if c == '\t' {
                        // Tab key cycles through windows
                        layout_manager.cycle_active_window();
                        compositor.mark_dirty(0, 0, width, height);
                    } else if let Some(active_window) = layout_manager.get_active_window() {
                        // Forward key event to the owner task of the active window
                        let forward_msg = Message {
                            sender: 5, // From GUI server
                            msg_type: 20,
                            arg1: c as u64,
                            arg2: 0, arg3: 0, arg4: 0,
                        };
                        unsafe {
                            core::arch::asm!(
                                "syscall",
                                in("rax") 3u64,
                                in("rdi") active_window.owner_task as u64,
                                in("rsi") &forward_msg as *const Message as u64,
                                out("rcx") _, out("r11") _,
                            );
                        }
                    }
                }
                30 => {
                    // MSG_MOUSE_EVENT: arg1 = dx, arg2 = dy, arg3 = left_click, arg4 = right_click
                    let dx = msg.arg1 as i64;
                    let dy = msg.arg2 as i64;
                    let left_click = msg.arg3 != 0;
                    let _right_click = msg.arg4 != 0;

                    // Track old position for dirty rectangle
                    let old_mx = mouse_x;
                    let old_my = mouse_y;

                    // Update mouse position with bounds clamping
                    let new_x = mouse_x as i64 + dx;
                    let new_y = mouse_y as i64 - dy; // PS/2 Y is inverted
                    mouse_x = new_x.clamp(0, (width as i64) - 1) as usize;
                    mouse_y = new_y.clamp(0, (height as i64) - 1) as usize;

                    // Mark old and new mouse positions as dirty
                    compositor.mark_dirty(old_mx, old_my, 20, 20);
                    compositor.mark_dirty(mouse_x, mouse_y, 20, 20);

                    // Handle click events - check if mouse is over a window titlebar
                    if left_click {
                        // Check dock clicks (bottom bar)
                        let dock_height = 48;
                        let dock_y = height.saturating_sub(dock_height);
                        
                        if mouse_y >= dock_y {
                            // Clicked on dock area - could launch apps
                            // For now, check the nyan cat icon position
                            let icon_spacing = 64;
                            let icons_start_x = width / 2 - icon_spacing;
                            
                            // Check if clicked on nyan cat icon (second icon)
                            let nyan_x = icons_start_x + icon_spacing;
                            if mouse_x >= nyan_x && mouse_x < nyan_x + 40 {
                                // Launch nyan cat demo via Syscall 11
                                unsafe {
                                    core::arch::asm!(
                                        "syscall",
                                        in("rax") 11u64,
                                        out("rcx") _, out("r11") _,
                                    );
                                }
                                compositor.mark_dirty(0, 0, width, height);
                            }
                        } else {
                            // Check window titlebar clicks for focus/toggle
                            for i in 0..layout_manager.window_count() {
                                if let Some(win) = layout_manager.get_window(i) {
                                    // Check if click is in titlebar area (top 30px of window)
                                    if mouse_x >= win.x && mouse_x < win.x + win.width
                                        && mouse_y >= win.y && mouse_y < win.y + 30 {
                                        // Check close button (rightmost 20px of titlebar)
                                        if mouse_x >= win.x + win.width - 25 {
                                            layout_manager.toggle_window(i);
                                        } else {
                                            layout_manager.set_active(i);
                                        }
                                        compositor.mark_dirty(0, 0, width, height);
                                        break;
                                    }
                                    
                                    // Check if click is inside the window body
                                    if !win.minimized 
                                        && mouse_x >= win.x && mouse_x < win.x + win.width
                                        && mouse_y >= win.y && mouse_y < win.y + win.height {
                                        layout_manager.set_active(i);
                                        compositor.mark_dirty(0, 0, width, height);
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
                40 => {
                    // MSG_CREATE_WINDOW: arg1 = title_ptr, arg2 = title_len, arg3 = width, arg4 = height
                    let title_ptr = msg.arg1 as *const u8;
                    let title_len = msg.arg2 as usize;
                    let win_w = msg.arg3 as usize;
                    let win_h = msg.arg4 as usize;
                    
                    let title_slice = unsafe { core::slice::from_raw_parts(title_ptr, title_len.min(32)) };
                    let mut title_buf = [0u8; 32];
                    title_buf[..title_slice.len()].copy_from_slice(title_slice);
                    
                    let window_id = layout_manager.add_window(
                        &title_buf[..title_slice.len()],
                        win_w, win_h,
                        sender,
                        width, height
                    );
                    compositor.mark_dirty(0, 0, width, height);
                    
                    // Send acknowledgment with window_id
                    let ack = Message {
                        sender: 5,
                        msg_type: 40,
                        arg1: window_id,
                        arg2: 0, arg3: 0, arg4: 0,
                    };
                    unsafe {
                        core::arch::asm!(
                            "syscall",
                            in("rax") 3u64,
                            in("rdi") sender as u64,
                            in("rsi") &ack as *const Message as u64,
                            out("rcx") _, out("r11") _,
                        );
                    }
                }
                41 => {
                    // MSG_CLOSE_WINDOW: arg1 = window_id
                    let window_id = msg.arg1;
                    layout_manager.remove_window(window_id);
                    compositor.mark_dirty(0, 0, width, height);
                }
                _ => {}
            }
        }

        // 2. Always mark full screen dirty for periodic refresh (updates stats, clock, etc.)
        compositor.mark_dirty(0, 0, width, height);

        // 3. Composite and render the desktop
        compositor.composite(&layout_manager, mouse_x, mouse_y);

        // 4. Blit to physical framebuffer
        compositor.blit_dirty();

        // 5. Yield CPU
        unsafe {
            core::arch::asm!(
                "syscall",
                in("rax") 2u64, // Yield
                out("rcx") _, out("r11") _,
            );
        }
    }
}

