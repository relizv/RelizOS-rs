/// Alpha blend a foreground color onto a background buffer pixel
#[inline]
pub fn blend_pixel(buffer: &mut [u32], idx: usize, color: u32, alpha: u8) {
    if idx >= buffer.len() {
        return;
    }
    if alpha == 255 {
        buffer[idx] = color;
    } else if alpha == 0 {
        return;
    } else {
        let bg = buffer[idx];
        let bg_r = (bg >> 16) & 0xFF;
        let bg_g = (bg >> 8) & 0xFF;
        let bg_b = bg & 0xFF;
        
        let fg_r = (color >> 16) & 0xFF;
        let fg_g = (color >> 8) & 0xFF;
        let fg_b = color & 0xFF;
        
        let a = alpha as u32;
        let r = ((fg_r * a + bg_r * (255 - a)) / 255) & 0xFF;
        let g = ((fg_g * a + bg_g * (255 - a)) / 255) & 0xFF;
        let b = ((fg_b * a + bg_b * (255 - a)) / 255) & 0xFF;
        
        buffer[idx] = (r << 16) | (g << 8) | b;
    }
}

/// Draw a solid rectangle
pub fn draw_rect(buffer: &mut [u32], stride: usize, x: usize, y: usize, w: usize, h: usize, color: u32) {
    let buf_len = buffer.len();
    for dy in 0..h {
        let py = y + dy;
        let row_offset = py * stride;
        for dx in 0..w {
            let px = x + dx;
            let idx = row_offset + px;
            if idx < buf_len {
                buffer[idx] = color;
            }
        }
    }
}

/// Draw a glassy translucent rectangle (using alpha blending)
pub fn draw_translucent_rect(buffer: &mut [u32], stride: usize, x: usize, y: usize, w: usize, h: usize, color: u32, alpha: u8) {
    let buf_len = buffer.len();
    for dy in 0..h {
        let py = y + dy;
        let row_offset = py * stride;
        for dx in 0..w {
            let px = x + dx;
            let idx = row_offset + px;
            if idx < buf_len {
                blend_pixel(buffer, idx, color, alpha);
            }
        }
    }
}

/// Draw a rectangle with rounded corners
pub fn draw_rounded_rect(
    buffer: &mut [u32],
    stride: usize,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    r: usize,
    color: u32,
) {
    let buf_len = buffer.len();
    let r_sq = (r * r) as isize;

    for dy in 0..h {
        let py = y + dy;
        let row_offset = py * stride;
        for dx in 0..w {
            let px = x + dx;
            let idx = row_offset + px;
            if idx >= buf_len {
                continue;
            }

            // Check if we are inside one of the four rounded corners
            let mut draw = true;
            
            // Top-left
            if dx < r && dy < r {
                let cx = r;
                let cy = r;
                let dist_sq = ((cx - dx) * (cx - dx) + (cy - dy) * (cy - dy)) as isize;
                if dist_sq > r_sq { draw = false; }
            }
            // Top-right
            else if dx >= w - r && dy < r {
                let cx = w - r - 1;
                let cy = r;
                let dist_sq = ((dx - cx) * (dx - cx) + (cy - dy) * (cy - dy)) as isize;
                if dist_sq > r_sq { draw = false; }
            }
            // Bottom-left
            else if dx < r && dy >= h - r {
                let cx = r;
                let cy = h - r - 1;
                let dist_sq = ((cx - dx) * (cx - dx) + (dy - cy) * (dy - cy)) as isize;
                if dist_sq > r_sq { draw = false; }
            }
            // Bottom-right
            else if dx >= w - r && dy >= h - r {
                let cx = w - r - 1;
                let cy = h - r - 1;
                let dist_sq = ((dx - cx) * (dx - cx) + (dy - cy) * (dy - cy)) as isize;
                if dist_sq > r_sq { draw = false; }
            }

            if draw {
                buffer[idx] = color;
            }
        }
    }
}

/// Draw a soft drop shadow around a window region using linear alpha gradients
pub fn draw_shadow(
    buffer: &mut [u32],
    stride: usize,
    wx: usize,
    wy: usize,
    ww: usize,
    wh: usize,
    shadow_sz: usize,
) {
    let buf_len = buffer.len();
    
    // Draw shadow on the 4 borders around the window
    for dy in 0..(wh + shadow_sz * 2) {
        let py = wy as isize - shadow_sz as isize + dy as isize;
        if py < 0 { continue; }
        let row_offset = (py as usize) * stride;

        for dx in 0..(ww + shadow_sz * 2) {
            let px = wx as isize - shadow_sz as isize + dx as isize;
            if px < 0 { continue; }
            let idx = row_offset + (px as usize);
            if idx >= buf_len { continue; }

            // Do not paint shadow under the window itself
            if px >= wx as isize && px < (wx + ww) as isize && py >= wy as isize && py < (wy + wh) as isize {
                continue;
            }

            // Calculate distance to nearest window edge to determine drop-off
            let mut dist_x = shadow_sz as isize;
            if px < wx as isize {
                dist_x = wx as isize - px;
            } else if px >= (wx + ww) as isize {
                dist_x = px - (wx + ww) as isize + 1;
            }

            let mut dist_y = shadow_sz as isize;
            if py < wy as isize {
                dist_y = wy as isize - py;
            } else if py >= (wy + wh) as isize {
                dist_y = py - (wy + wh) as isize + 1;
            }

            let dist = core::cmp::max(dist_x, dist_y) as f32;
            if dist < shadow_sz as f32 {
                // Closer means darker shadow
                let ratio = 1.0 - (dist / shadow_sz as f32);
                let alpha = (ratio * 120.0) as u8; // Max shadow opacity of ~120/255
                blend_pixel(buffer, idx, 0x000000, alpha);
            }
        }
    }
}

/// Draw a classic black and white 3D-outline arrow cursor
pub fn draw_mouse_cursor(buffer: &mut [u32], stride: usize, mx: usize, my: usize) {
    let cursor_mask = [
        "X               ",
        "XX              ",
        "X.X             ",
        "X..X            ",
        "X...X           ",
        "X....X          ",
        "X.....X         ",
        "X......X        ",
        "X.......X       ",
        "X........X      ",
        "X.....XXXX      ",
        "X..X..X         ",
        "X.X X..X        ",
        "XX   X..X       ",
        "X     X..X      ",
        "       XX       ",
    ];

    let buf_len = buffer.len();
    for (row, line) in cursor_mask.iter().enumerate() {
        let py = my + row;
        let row_offset = py * stride;
        for (col, ch) in line.chars().enumerate() {
            let px = mx + col;
            let idx = row_offset + px;
            if idx >= buf_len {
                continue;
            }

            match ch {
                'X' => buffer[idx] = 0x000000, // Black border
                '.' => buffer[idx] = 0xFFFFFF, // White arrow body
                _ => {} // Transparent
            }
        }
    }
}

/// Draw a translucent rectangle with rounded corners
pub fn draw_translucent_rounded_rect(
    buffer: &mut [u32],
    stride: usize,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    r: usize,
    color: u32,
    alpha: u8,
) {
    let buf_len = buffer.len();
    let r_sq = (r * r) as isize;

    for dy in 0..h {
        let py = y + dy;
        let row_offset = py * stride;
        for dx in 0..w {
            let px = x + dx;
            let idx = row_offset + px;
            if idx >= buf_len {
                continue;
            }

            // Check if we are inside one of the four rounded corners
            let mut draw = true;
            
            // Top-left
            if dx < r && dy < r {
                let cx = r;
                let cy = r;
                let dist_sq = ((cx - dx) * (cx - dx) + (cy - dy) * (cy - dy)) as isize;
                if dist_sq > r_sq { draw = false; }
            }
            // Top-right
            else if dx >= w - r && dy < r {
                let cx = w - r - 1;
                let cy = r;
                let dist_sq = ((dx - cx) * (dx - cx) + (cy - dy) * (cy - dy)) as isize;
                if dist_sq > r_sq { draw = false; }
            }
            // Bottom-left
            else if dx < r && dy >= h - r {
                let cx = r;
                let cy = h - r - 1;
                let dist_sq = ((cx - dx) * (cx - dx) + (dy - cy) * (dy - cy)) as isize;
                if dist_sq > r_sq { draw = false; }
            }
            // Bottom-right
            else if dx >= w - r && dy >= h - r {
                let cx = w - r - 1;
                let cy = h - r - 1;
                let dist_sq = ((dx - cx) * (dx - cx) + (dy - cy) * (dy - cy)) as isize;
                if dist_sq > r_sq { draw = false; }
            }

            if draw {
                blend_pixel(buffer, idx, color, alpha);
            }
        }
    }
}

/// Draw a single char using font8x8
pub fn draw_char(buffer: &mut [u32], stride: usize, x: usize, y: usize, c: char, color: u32) {
    use font8x8::UnicodeFonts;
    if let Some(font_char) = font8x8::BASIC_FONTS.get(c) {
        let buf_len = buffer.len();
        for (row, byte) in font_char.iter().enumerate() {
            let py = y + row;
            let row_offset = py * stride;
            for col in 0..8 {
                if (byte & (1 << col)) != 0 {
                    let px = x + col;
                    let idx = row_offset + px;
                    if idx < buf_len {
                        buffer[idx] = color;
                    }
                }
            }
        }
    }
}

/// Draw a string using font8x8
pub fn draw_string(buffer: &mut [u32], stride: usize, x: usize, y: usize, s: &str, color: u32) {
    let mut cx = x;
    for c in s.chars() {
        draw_char(buffer, stride, cx, y, c, color);
        cx += 8;
    }
}

/// Draw a solid filled circle
pub fn draw_filled_circle(buffer: &mut [u32], stride: usize, cx: usize, cy: usize, r: usize, color: u32) {
    let buf_len = buffer.len();
    let r_sq = (r * r) as isize;
    
    for dy in 0..=(r * 2) {
        let py = cy as isize - r as isize + dy as isize;
        if py < 0 { continue; }
        let row_offset = (py as usize) * stride;
        
        for dx in 0..=(r * 2) {
            let px = cx as isize - r as isize + dx as isize;
            if px < 0 { continue; }
            let idx = row_offset + (px as usize);
            if idx >= buf_len { continue; }
            
            let dist_x = r as isize - dx as isize;
            let dist_y = r as isize - dy as isize;
            let dist_sq = dist_x * dist_x + dist_y * dist_y;
            if dist_sq <= r_sq {
                buffer[idx] = color;
            }
        }
    }
}

