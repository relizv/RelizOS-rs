use alloc::vec::Vec;
use crate::gui::layout::LayoutManager;
use crate::gui::renderer::{
    draw_rect, draw_rounded_rect, draw_translucent_rounded_rect,
    draw_shadow, draw_mouse_cursor, draw_string, draw_filled_circle,
};

#[repr(C)]
pub struct BlitRequest {
    pub buffer_ptr: *const u32,
    pub src_stride: usize,
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

pub struct Compositor {
    pub width: usize,
    pub height: usize,
    pub buffer: Vec<u32>,
    pub dirty_x0: usize,
    pub dirty_y0: usize,
    pub dirty_x1: usize,
    pub dirty_y1: usize,
}

impl Compositor {
    pub fn new(width: usize, height: usize) -> Self {
        let size = width * height;
        let mut buffer = Vec::with_capacity(size);
        buffer.resize(size, 0);

        Self {
            width,
            height,
            buffer,
            // Initialize dirty bounds to full screen for the initial frame
            dirty_x0: 0,
            dirty_y0: 0,
            dirty_x1: width,
            dirty_y1: height,
        }
    }

    /// Mark a rectangular region as dirty
    pub fn mark_dirty(&mut self, x: usize, y: usize, w: usize, h: usize) {
        let x1 = core::cmp::min(x + w, self.width);
        let y1 = core::cmp::min(y + h, self.height);

        self.dirty_x0 = core::cmp::min(self.dirty_x0, x);
        self.dirty_y0 = core::cmp::min(self.dirty_y0, y);
        self.dirty_x1 = core::cmp::max(self.dirty_x1, x1);
        self.dirty_y1 = core::cmp::max(self.dirty_y1, y1);
    }

    /// Composite all desktop layers into the backbuffer
    pub fn composite(&mut self, layout: &LayoutManager, mouse_x: usize, mouse_y: usize) {
        let stride = self.width;

        // 1. Draw a beautiful dark mode gradient background
        for y in 0..self.height {
            // Cool dark blue-purple to deep gray/charcoal linear gradient
            let r = 15 + (11 * y) / self.height;
            let g = 10 + (17 * y) / self.height;
            let b = 28 + (10 * y) / self.height;
            let color = ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
            
            let row_offset = y * stride;
            for x in 0..self.width {
                self.buffer[row_offset + x] = color;
            }
        }

        // 2. Draw Tiled Windows
        for w in &layout.windows {
            // Draw window drop shadow
            draw_shadow(&mut self.buffer, stride, w.x, w.y, w.w, w.h, 12);

            // Draw window background container
            draw_rounded_rect(&mut self.buffer, stride, w.x, w.y, w.w, w.h, 8, 0x1E1E2E);

            // Draw window title bar (header)
            draw_rounded_rect(&mut self.buffer, stride, w.x, w.y, w.w, 28, 8, 0x181825);
            // Overwrite bottom rounded part of the header to make it flat where it meets the content
            draw_rect(&mut self.buffer, stride, w.x, w.y + 8, w.w, 20, 0x181825);

            // Draw Window Control Buttons (macOS/premium style: Red, Yellow, Green dots)
            draw_filled_circle(&mut self.buffer, stride, w.x + 16, w.y + 14, 4, 0xFF5F56);
            draw_filled_circle(&mut self.buffer, stride, w.x + 28, w.y + 14, 4, 0xFFBD2E);
            draw_filled_circle(&mut self.buffer, stride, w.x + 40, w.y + 14, 4, 0x27C93F);

            // Draw Window Title
            let title = match w.id {
                1 => "Terminal Shell",
                2 => "File Manager",
                3 => "System Monitor",
                _ => "Application Window",
            };
            draw_string(&mut self.buffer, stride, w.x + 56, w.y + 10, title, 0xCDD6F4);

            // Draw simulated client window contents
            match w.id {
                1 => {
                    // Shell simulated text
                    draw_string(&mut self.buffer, stride, w.x + 15, w.y + 40, "guest@relizos:~$ neofetch", 0xA6E3A1);
                    draw_string(&mut self.buffer, stride, w.x + 15, w.y + 60, "OS: RelizOS x86_64", 0xCDD6F4);
                    draw_string(&mut self.buffer, stride, w.x + 15, w.y + 75, "Kernel: relizos-rust 0.1.0", 0xCDD6F4);
                    draw_string(&mut self.buffer, stride, w.x + 15, w.y + 90, "Uptime: 42 seconds", 0xCDD6F4);
                    draw_string(&mut self.buffer, stride, w.x + 15, w.y + 105, "Shell: relizsh", 0xCDD6F4);
                    draw_string(&mut self.buffer, stride, w.x + 15, w.y + 120, "Memory: 2.4 MB / 16.0 MB", 0xCDD6F4);
                }
                2 => {
                    // File manager simulated text
                    draw_string(&mut self.buffer, stride, w.x + 15, w.y + 40, "[DIR]  bin", 0x89B4FA);
                    draw_string(&mut self.buffer, stride, w.x + 15, w.y + 55, "[DIR]  dev", 0x89B4FA);
                    draw_string(&mut self.buffer, stride, w.x + 15, w.y + 70, "[DIR]  home", 0x89B4FA);
                    draw_string(&mut self.buffer, stride, w.x + 15, w.y + 85, "[DIR]  kernel", 0x89B4FA);
                    draw_string(&mut self.buffer, stride, w.x + 15, w.y + 100, "[FILE] README.txt", 0xF5C2E7);
                }
                3 => {
                    // System monitor simulated text
                    draw_string(&mut self.buffer, stride, w.x + 15, w.y + 40, "CPU Usage: [|||||||||         ] 45%", 0xF9E2AF);
                    draw_string(&mut self.buffer, stride, w.x + 15, w.y + 60, "RAM Usage: [||||||            ] 30%", 0x89B4FA);
                    draw_string(&mut self.buffer, stride, w.x + 15, w.y + 80, "Tasks active: 5", 0xCDD6F4);
                }
                _ => {
                    draw_string(&mut self.buffer, stride, w.x + 15, w.y + 40, "Tiled application view.", 0xA6ADC8);
                }
            }
        }

        // 3. Draw Windows 10X Centered Bottom Dock
        let (dock_x, dock_y, dock_w, dock_h) = layout.get_dock_rect();
        // Draw dock shadow
        draw_shadow(&mut self.buffer, stride, dock_x, dock_y, dock_w, dock_h, 8);
        // Draw glassy translucent dock background (using alpha=180)
        draw_translucent_rounded_rect(&mut self.buffer, stride, dock_x, dock_y, dock_w, dock_h, 8, 0x1E1E2E, 180);

        // Render dock shortcut icons
        let icon_count = 5;
        let icon_spacing = 50;
        let start_icon_x = dock_x + (dock_w - (icon_count * icon_spacing)) / 2;
        let icon_y = dock_y + (dock_h - 24) / 2;

        for i in 0..icon_count {
            let cx = start_icon_x + i * icon_spacing + 12;
            let cy = icon_y + 12;
            match i {
                0 => {
                    // Terminal (Shell) icon: dark slate square with white >
                    draw_rounded_rect(&mut self.buffer, stride, cx - 12, cy - 12, 24, 24, 4, 0x313244);
                    draw_string(&mut self.buffer, stride, cx - 4, cy - 4, ">", 0xA6E3A1);
                }
                1 => {
                    // File Manager icon: folder shape (yellow)
                    draw_rounded_rect(&mut self.buffer, stride, cx - 12, cy - 12, 24, 24, 4, 0xF9E2AF);
                    draw_rect(&mut self.buffer, stride, cx - 8, cy - 8, 16, 16, 0xCDD6F4);
                }
                2 => {
                    // Web browser icon: blue planet shape
                    draw_filled_circle(&mut self.buffer, stride, cx, cy, 10, 0x89B4FA);
                    draw_filled_circle(&mut self.buffer, stride, cx - 2, cy - 2, 4, 0xA6E3A1);
                }
                3 => {
                    // Settings icon: gear (gray circle)
                    draw_filled_circle(&mut self.buffer, stride, cx, cy, 10, 0x45475A);
                    draw_filled_circle(&mut self.buffer, stride, cx, cy, 4, 0x11111B);
                }
                4 => {
                    // Nyan Cat Demo icon: pink/toaster-pastry shape
                    draw_rounded_rect(&mut self.buffer, stride, cx - 12, cy - 12, 24, 24, 4, 0xF5C2E7);
                    draw_rect(&mut self.buffer, stride, cx - 8, cy - 8, 16, 16, 0xF38BA8);
                }
                _ => {}
            }
        }

        // 4. Draw Mouse Cursor
        draw_mouse_cursor(&mut self.buffer, stride, mouse_x, mouse_y);
    }

    /// Blit the dirty bounding box region to the physical framebuffer
    pub fn blit_dirty(&mut self) {
        if self.dirty_x1 <= self.dirty_x0 || self.dirty_y1 <= self.dirty_y0 {
            return;
        }

        let x = self.dirty_x0;
        let y = self.dirty_y0;
        let w = self.dirty_x1 - self.dirty_x0;
        let h = self.dirty_y1 - self.dirty_y0;

        let req = BlitRequest {
            buffer_ptr: self.buffer.as_ptr(),
            src_stride: self.width,
            x,
            y,
            w,
            h,
        };

        unsafe {
            core::arch::asm!(
                "syscall",
                in("rax") 12u64, // Syscall 12: Blit Rect
                in("rdi") &req as *const BlitRequest as u64,
                out("rcx") _, out("r11") _,
            );
        }

        // Reset dirty rect
        self.dirty_x0 = self.width;
        self.dirty_y0 = self.height;
        self.dirty_x1 = 0;
        self.dirty_y1 = 0;
    }
}
