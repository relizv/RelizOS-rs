use alloc::vec::Vec;

#[derive(Clone, Copy, Debug)]
pub struct WindowInfo {
    pub id: u64,
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

pub struct LayoutManager {
    pub screen_w: usize,
    pub screen_h: usize,
    pub dock_h: usize,
    pub windows: Vec<WindowInfo>,
}

impl LayoutManager {
    pub fn new(screen_w: usize, screen_h: usize) -> Self {
        Self {
            screen_w,
            screen_h,
            dock_h: 60, // 60 pixels for the centered Windows 10X dock
            windows: Vec::new(),
        }
    }

    /// Register a new window ID
    pub fn add_window(&mut self, id: u64) {
        // Prevent duplicate IDs
        if !self.windows.iter().any(|w| w.id == id) {
            self.windows.push(WindowInfo {
                id,
                x: 0, y: 0, w: 0, h: 0,
            });
            self.recalculate();
        }
    }

    /// Unregister a window ID
    pub fn remove_window(&mut self, id: u64) {
        if let Some(pos) = self.windows.iter().position(|w| w.id == id) {
            self.windows.remove(pos);
            self.recalculate();
        }
    }

    /// Dynamically recalculate window tiling geometries using a Master/Stack layout with 10px gaps
    pub fn recalculate(&mut self) {
        let n = self.windows.len();
        if n == 0 {
            return;
        }

        let gaps = 10;
        let dock_space = self.dock_h;
        let usable_h = self.screen_h - dock_space;

        if n == 1 {
            // Single window takes the full workspace
            self.windows[0].x = gaps;
            self.windows[0].y = gaps;
            self.windows[0].w = self.screen_w - gaps * 2;
            self.windows[0].h = usable_h - gaps * 2;
        } else {
            // Master & Stack Tiling Layout (N >= 2)
            // Left half (Master): Window 0
            let master_w = self.screen_w / 2 - gaps - gaps / 2;
            self.windows[0].x = gaps;
            self.windows[0].y = gaps;
            self.windows[0].w = master_w;
            self.windows[0].h = usable_h - gaps * 2;

            // Right half (Stack): Windows 1..N-1 stacked vertically
            let stack_x = self.screen_w / 2 + gaps / 2;
            let stack_w = self.screen_w / 2 - gaps - gaps / 2;
            
            let stack_count = n - 1;
            // Distribute remaining height and subtract internal gaps
            let total_gaps_height = (stack_count - 1) * gaps;
            let total_usable_stack_h = (usable_h - gaps * 2) - total_gaps_height;
            let cell_h = total_usable_stack_h / stack_count;

            for i in 1..n {
                self.windows[i].x = stack_x;
                self.windows[i].y = gaps + (i - 1) * (cell_h + gaps);
                self.windows[i].w = stack_w;
                self.windows[i].h = cell_h;
            }
        }
    }

    /// Get coordinates of the centered dock
    pub fn get_dock_rect(&self) -> (usize, usize, usize, usize) {
        let dock_w = core::cmp::min(450, self.screen_w - 40); // Centered, max width 450px
        let dock_x = (self.screen_w - dock_w) / 2;
        let dock_y = self.screen_h - self.dock_h + 10; // Floating dock effect (10px padding from bottom)
        let dock_h = self.dock_h - 20; // Dock actual height is 40px
        (dock_x, dock_y, dock_w, dock_h)
    }
}
