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
