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
        // Wait for input/draw/IPC events from ANY task
        unsafe {
            core::arch::asm!(
                "syscall",
                in("rax") 4u64,     // Syscall 4: Recv
                in("rdi") 0u64,     // 0 = Receive from ANY
                in("rsi") &mut msg as *mut Message as u64,
                out("rcx") _, out("r11") _,
            );
        }

        let sender = msg.sender;

        match msg.msg_type {
            20 => {
                // MSG_KEY_EVENT: arg1 is char
                let c = msg.arg1 as u8 as char;
                
                // Route keyboard character to current active window
                // In our microkernel setup, the Shell (Task 1) is our main interactive target.
                let forward_msg = Message {
                    sender: 5, // GUI Server ID
                    msg_type: 20, // MSG_KEY_EVENT
                    arg1: c as u64,
                    arg2: 0, arg3: 0, arg4: 0,
                };
                unsafe {
                    core::arch::asm!(
                        "syscall",
                        in("rax") 3u64, // Send
                        in("rdi") 1u64, // Shell ID (1)
                        in("rsi") &forward_msg as *const Message as u64,
                        out("rcx") _, out("r11") _,
                    );
                }
            }
            30 => {
                // MSG_MOUSE_EVENT: arg1 = dx, arg2 = dy, arg3 = left_click, arg4 = right_click
                let dx = msg.arg1 as i32;
                let dy = msg.arg2 as i32;
                let left_click = msg.arg3 != 0;
                
                // Track old cursor coordinates to mark dirty rectangles
                let old_mx = mouse_x;
                let old_my = mouse_y;

                // Update mouse position with screen boundary constraints
                let mut nx = mouse_x as isize + dx as isize;
                let mut ny = mouse_y as isize - dy as isize; // PS/2 y delta is inverted

                if nx < 0 { nx = 0; }
                if nx >= width as isize { nx = width as isize - 1; }
                if ny < 0 { ny = 0; }
                if ny >= height as isize { ny = height as isize - 1; }

                mouse_x = nx as usize;
                mouse_y = ny as usize;

                // Support interactive clicking on bottom dock icons
                if left_click {
                    let (dock_x, dock_y, dock_w, dock_h) = layout_manager.get_dock_rect();
                    let icon_count = 5;
                    let icon_spacing = 50;
                    let start_icon_x = dock_x + (dock_w - (icon_count * icon_spacing)) / 2;
                    let icon_y = dock_y + (dock_h - 24) / 2;
                    
                    for i in 0..icon_count {
                        let icon_x = start_icon_x + i * icon_spacing;
                        // Click inside the 24x24 icon boundary
                        if mouse_x >= icon_x && mouse_x < icon_x + 24 &&
                           mouse_y >= icon_y && mouse_y < icon_y + 24 {
                            match i {
                                0 => {
                                    // Toggle Terminal Shell (ID 1)
                                    if layout_manager.windows.iter().any(|w| w.id == 1) {
                                        layout_manager.remove_window(1);
                                    } else {
                                        layout_manager.add_window(1);
                                    }
                                    compositor.mark_dirty(0, 0, width, height);
                                }
                                1 => {
                                    // Toggle File Manager (ID 2)
                                    if layout_manager.windows.iter().any(|w| w.id == 2) {
                                        layout_manager.remove_window(2);
                                    } else {
                                        layout_manager.add_window(2);
                                    }
                                    compositor.mark_dirty(0, 0, width, height);
                                }
                                2 => {
                                    // Toggle System Monitor (ID 3)
                                    if layout_manager.windows.iter().any(|w| w.id == 3) {
                                        layout_manager.remove_window(3);
                                    } else {
                                        layout_manager.add_window(3);
                                    }
                                    compositor.mark_dirty(0, 0, width, height);
                                }
                                4 => {
                                    // Launch Gfx Demo (Nyan Cat!)
                                    unsafe {
                                        core::arch::asm!(
                                            "syscall",
                                            in("rax") 11u64,
                                            out("rcx") _, out("r11") _,
                                        );
                                    }
                                    compositor.mark_dirty(0, 0, width, height);
                                }
                                _ => {}
                            }
                            break;
                        }
                    }
                }

                // Mark old and new mouse positions as dirty
                compositor.mark_dirty(old_mx, old_my, 20, 20);
                compositor.mark_dirty(mouse_x, mouse_y, 20, 20);
            }
            40 => {
                // MSG_CREATE_WINDOW: arg1 = window_id
                let window_id = msg.arg1;
                layout_manager.add_window(window_id);
                
                // Mark whole screen dirty to recomposite the layout division
                compositor.mark_dirty(0, 0, width, height);

                // ACK success to client
                let ack = Message {
                    sender: 5,
                    msg_type: 0, // MSG_OK
                    arg1: 0, arg2: 0, arg3: 0, arg4: 0,
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

        // 3. Composite and render the desktop background, taskbar, layout windows, and mouse
        compositor.composite(&layout_manager, mouse_x, mouse_y);
        
        // 4. Blit the dirty areas to physical GOP framebuffer
        compositor.blit_dirty();

        // Yield CPU to let other tasks draw
        unsafe {
            core::arch::asm!(
                "syscall",
                in("rax") 2u64, // Yield
                out("rcx") _, out("r11") _,
            );
        }
    }
}
