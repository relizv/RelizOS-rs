use bootloader_api::info::{FrameBufferInfo, PixelFormat};
use core::fmt;
use font8x8::UnicodeFonts;
use spin::Mutex;

/// Global writer protected by a Mutex
pub static WRITER: Mutex<Option<FrameBufferWriter>> = Mutex::new(None);

/// FrameBufferWriter handles cursor positions and colors for writing text
pub struct FrameBufferWriter {
    buffer: &'static mut [u8],
    info: FrameBufferInfo,
    x_pos: usize,
    y_pos: usize,
    text_color: (u8, u8, u8), // RGB
    bg_color: (u8, u8, u8),   // RGB
}

impl FrameBufferWriter {
    pub fn new(buffer: &'static mut [u8], info: FrameBufferInfo) -> Self {
        let mut writer = Self {
            buffer,
            info,
            x_pos: 10,
            y_pos: 10,
            text_color: (255, 255, 255), // White
            bg_color: (20, 20, 40),       // Deep Dark Blue
        };
        writer.clear();
        writer
    }

    /// Clear the entire screen with the background color
    pub fn clear(&mut self) {
        for y in 0..self.info.height {
            for x in 0..self.info.width {
                self.write_pixel(x, y, self.bg_color);
            }
        }
        self.x_pos = 10;
        self.y_pos = 10;
    }

    /// Set text colors
    pub fn set_colors(&mut self, text: (u8, u8, u8), bg: (u8, u8, u8)) {
        self.text_color = text;
        self.bg_color = bg;
    }

    /// Write a single pixel to the framebuffer
    #[inline]
    pub fn write_pixel(&mut self, x: usize, y: usize, color: (u8, u8, u8)) {
        if x >= self.info.width || y >= self.info.height {
            return;
        }

        let stride = self.info.stride;
        let bytes_per_pixel = self.info.bytes_per_pixel;
        let pixel_offset = (y * stride + x) * bytes_per_pixel;

        match self.info.pixel_format {
            PixelFormat::Rgb => {
                self.buffer[pixel_offset] = color.0;
                self.buffer[pixel_offset + 1] = color.1;
                self.buffer[pixel_offset + 2] = color.2;
            }
            PixelFormat::Bgr => {
                self.buffer[pixel_offset] = color.2;
                self.buffer[pixel_offset + 1] = color.1;
                self.buffer[pixel_offset + 2] = color.0;
            }
            PixelFormat::U8 => {
                // Grayscale approximation
                self.buffer[pixel_offset] = ((color.0 as u32 + color.1 as u32 + color.2 as u32) / 3) as u8;
            }
            _ => {}
        }
    }

    /// Write a character using the 8x8 font
    pub fn write_char(&mut self, c: char) {
        match c {
            '\n' => self.newline(),
            '\r' => self.x_pos = 10,
            '\x08' => {
                if self.x_pos > 10 {
                    self.x_pos -= 8;
                    for row in 0..8 {
                        for col in 0..8 {
                            self.write_pixel(self.x_pos + col, self.y_pos + row, self.bg_color);
                        }
                    }
                }
            }
            '\x0C' => {
                self.clear();
            }
            _ => {
                if self.x_pos + 8 >= self.info.width {
                    self.newline();
                }
                if self.y_pos + 16 >= self.info.height {
                    // Simple screen wrapping: instead of scrolling (which is slow), just clear and reset
                    self.clear();
                }

                // Get font bitmap for the character
                if let Some(font_char) = font8x8::BASIC_FONTS.get(c) {
                    for (row, byte) in font_char.iter().enumerate() {
                        for col in 0..8 {
                            let color = if (byte & (1 << col)) != 0 {
                                self.text_color
                            } else {
                                self.bg_color
                            };
                            self.write_pixel(self.x_pos + col, self.y_pos + row, color);
                        }
                    }
                } else {
                    // Write a solid box for unsupported characters
                    for row in 0..8 {
                        for col in 0..8 {
                            self.write_pixel(self.x_pos + col, self.y_pos + row, self.text_color);
                        }
                    }
                }
                self.x_pos += 8;
            }
        }
    }

    fn newline(&mut self) {
        self.x_pos = 10;
        self.y_pos += 12; // 8 pixels font height + 4 pixels line spacing
    }

    /// Draw a solid rectangle using self.write_pixel
    pub fn draw_rect(&mut self, x: usize, y: usize, w: usize, h: usize, color: (u8, u8, u8)) {
        for dy in 0..h {
            for dx in 0..w {
                self.write_pixel(x + dx, y + dy, color);
            }
        }
    }
}

impl fmt::Write for FrameBufferWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.chars() {
            self.write_char(c);
        }
        Ok(())
    }
}

/// Helper macros for printing
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ($crate::gop::_print(format_args!($($arg)*)));
}

#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}

#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    use core::fmt::Write;
    if let Some(ref mut writer) = *WRITER.lock() {
        writer.write_fmt(args).unwrap();
    }
}

/// Retrieve GOP framebuffer information
pub fn get_info() -> Option<FrameBufferInfo> {
    WRITER.lock().as_ref().map(|w| w.info)
}

/// Runs a pixel-art space animation featuring Nyan Cat flying with a waving rainbow trail.
pub fn run_nyan_cat_demo() {
    let mut writer_lock = WRITER.lock();
    if let Some(ref mut writer) = *writer_lock {
        let width = writer.info.width;
        let height = writer.info.height;
        
        let bg_color = (15, 15, 40); // Deep space blue
        
        // Save current color settings to restore them later
        let old_text_color = writer.text_color;
        let old_bg_color = writer.bg_color;

        // Run animation for 60 frames (~3 seconds at ~20fps)
        for frame in 0..60 {
            // 1. Clear screen to space background
            writer.draw_rect(0, 0, width, height, bg_color);
            
            // Draw some stars
            let star_positions = [
                (100, 150), (250, 80), (400, 200), (550, 120), (700, 250), (850, 90),
                (150, 300), (300, 400), (500, 350), (650, 420), (800, 380),
            ];
            for (sx, sy) in star_positions.iter() {
                // Scroll stars left based on frame
                let shift = (frame * 8) % 900;
                let mut x = *sx as isize - shift as isize;
                if x < 0 {
                    x += 900;
                }
                writer.draw_rect(x as usize, *sy, 4, 4, (255, 255, 255));
            }
            
            // 2. Draw Rainbow trail (waving pattern depending on frame)
            let cat_x = if width > 200 { width / 2 - 50 } else { 10 };
            let cat_y = if height > 100 { height / 2 - 30 } else { 10 };
            
            // Rainbow colors: Red, Orange, Yellow, Green, Blue, Purple
            let rainbow_colors = [
                (255, 0, 0),     // Red
                (255, 127, 0),   // Orange
                (255, 255, 0),   // Yellow
                (0, 255, 0),     // Green
                (0, 127, 255),   // Blue
                (127, 0, 255),   // Purple
            ];
            
            for rx in (0..cat_x).step_by(8) {
                // Wave offset
                let wave = (((rx / 8) + frame) % 8) < 4;
                let wave_y = if wave { cat_y + 2 } else { cat_y - 2 };
                
                for (i, color) in rainbow_colors.iter().enumerate() {
                    writer.draw_rect(rx, wave_y + i * 8, 8, 8, *color);
                }
            }
            
            // 3. Draw Nyan Cat
            // Feet/legs (bobbing up and down)
            let feet_bob = (frame % 4) < 2;
            let feet_y = cat_y + 40;
            if feet_bob {
                writer.draw_rect(cat_x + 10, feet_y, 8, 8, (150, 150, 150)); // Leg 1
                writer.draw_rect(cat_x + 30, feet_y, 8, 8, (150, 150, 150)); // Leg 2
                writer.draw_rect(cat_x + 60, feet_y, 8, 8, (150, 150, 150)); // Leg 3
                writer.draw_rect(cat_x + 80, feet_y, 8, 8, (150, 150, 150)); // Leg 4
            } else {
                writer.draw_rect(cat_x + 12, feet_y + 2, 8, 8, (150, 150, 150));
                writer.draw_rect(cat_x + 32, feet_y + 2, 8, 8, (150, 150, 150));
                writer.draw_rect(cat_x + 62, feet_y + 2, 8, 8, (150, 150, 150));
                writer.draw_rect(cat_x + 82, feet_y + 2, 8, 8, (150, 150, 150));
            }
            
            // Tail (wiggling)
            let tail_wiggle = (frame % 4) < 2;
            if tail_wiggle {
                writer.draw_rect(cat_x - 16, cat_y + 16, 16, 8, (150, 150, 150));
            } else {
                writer.draw_rect(cat_x - 16, cat_y + 20, 16, 8, (150, 150, 150));
            }
            
            // Pop-tart Body: Pink border, lighter pink inside
            writer.draw_rect(cat_x, cat_y + 4, 100, 36, (200, 130, 80)); // Crust (brownish)
            writer.draw_rect(cat_x + 4, cat_y + 8, 92, 28, (255, 150, 200)); // Frosting (pink)
            // Sprinkle dots (red/purple)
            writer.draw_rect(cat_x + 16, cat_y + 12, 4, 4, (255, 0, 100));
            writer.draw_rect(cat_x + 40, cat_y + 20, 4, 4, (255, 0, 100));
            writer.draw_rect(cat_x + 70, cat_y + 16, 4, 4, (255, 0, 100));
            writer.draw_rect(cat_x + 30, cat_y + 28, 4, 4, (255, 0, 100));
            writer.draw_rect(cat_x + 80, cat_y + 26, 4, 4, (255, 0, 100));

            // Head (on the right)
            let head_x = cat_x + 75;
            let head_y = cat_y - 4;
            // Face base (grey)
            writer.draw_rect(head_x, head_y + 8, 32, 24, (150, 150, 150));
            // Ears
            writer.draw_rect(head_x, head_y, 8, 8, (150, 150, 150));
            writer.draw_rect(head_x + 24, head_y, 8, 8, (150, 150, 150));
            // Eyes
            writer.draw_rect(head_x + 6, head_y + 14, 4, 4, (0, 0, 0));
            writer.draw_rect(head_x + 22, head_y + 14, 4, 4, (0, 0, 0));
            // Cheeks (pink)
            writer.draw_rect(head_x + 2, head_y + 18, 4, 4, (255, 100, 150));
            writer.draw_rect(head_x + 26, head_y + 18, 4, 4, (255, 100, 150));
            // Mouth/nose
            writer.draw_rect(head_x + 14, head_y + 18, 4, 4, (0, 0, 0));
            
            // Sleep for 50ms using a timer-based delay loop
            let start = unsafe { crate::interrupts::TIMER_TICKS };
            while unsafe { crate::interrupts::TIMER_TICKS } < start + 1 {
                x86_64::instructions::hlt();
            }
        }
        
        // Restore color settings and clear the screen
        writer.text_color = old_text_color;
        writer.bg_color = old_bg_color;
        writer.clear();
    }
}
