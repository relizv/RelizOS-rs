use x86_64::instructions::port::Port;
use spin::Mutex;

/// Global ATA drive accessor protected by a Spinlock Mutex
pub static ATA_DRIVE: Mutex<AtaPio> = Mutex::new(unsafe { AtaPio::new() });

/// ATA PIO Controller Interface
pub struct AtaPio {
    data_port: Port<u16>,
    error_port: Port<u8>,
    sector_count_port: Port<u8>,
    lba_low_port: Port<u8>,
    lba_mid_port: Port<u8>,
    lba_high_port: Port<u8>,
    device_port: Port<u8>,
    command_port: Port<u8>,
}

impl AtaPio {
    /// Create raw port mapping. Safe because ports are hardcoded and unique to ATA primary controller.
    pub const unsafe fn new() -> Self {
        Self {
            data_port: Port::new(0x1F0),
            error_port: Port::new(0x1F1),
            sector_count_port: Port::new(0x1F2),
            lba_low_port: Port::new(0x1F3),
            lba_mid_port: Port::new(0x1F4),
            lba_high_port: Port::new(0x1F5),
            device_port: Port::new(0x1F6),
            command_port: Port::new(0x1F7),
        }
    }

    /// Read a single 512-byte sector from the specified drive.
    /// * `drive`: 0 for Primary Master, 1 for Primary Slave (RelizFS drive).
    /// * `lba`: 28-bit Logical Block Address.
    /// * `buffer`: Target 512-byte array to store data.
    pub fn read_sector(&mut self, drive: u8, lba: u32, buffer: &mut [u8; 512]) -> Result<(), &'static str> {
        if drive > 1 {
            return Err("Invalid drive index (only 0 or 1 supported)");
        }

        // 1. Select the device and send upper LBA bits
        // Bit 7: 1, Bit 6: 1 (LBA mode), Bit 5: 1
        // Bit 4: 0 for Master (Drive 0), 1 for Slave (Drive 1)
        // Bit 0-3: LBA bits 24-27
        let drive_select = if drive == 0 { 0xE0 } else { 0xF0 };
        let device_val = drive_select | (((lba >> 24) & 0x0F) as u8);
        
        unsafe {
            self.device_port.write(device_val);
            
            // Wait a tiny bit for the drive to switch selection status
            for _ in 0..4 {
                let _ = self.command_port.read();
            }

            // 2. Set sector count to 1
            self.sector_count_port.write(1);

            // 3. Send LBA bits
            self.lba_low_port.write((lba & 0xFF) as u8);
            self.lba_mid_port.write(((lba >> 8) & 0xFF) as u8);
            self.lba_high_port.write(((lba >> 16) & 0xFF) as u8);

            // 4. Send Read Command (0x20)
            self.command_port.write(0x20);

            // 5. Poll the status port with timeout
            // BSY (Bit 7) must be 0, and DRQ (Bit 3) must be 1.
            // If ERR (Bit 0) is 1, operation failed.
            let mut timeout = 100_000u32;
            loop {
                let status = self.command_port.read();
                if status == 0xFF {
                    // No drive connected on this port
                    return Err("No ATA drive detected (port reads 0xFF)");
                }
                if (status & 0x01) != 0 {
                    let _err = self.error_port.read();
                    return Err("ATA Controller reported a read error");
                }
                if (status & 0x80) == 0 && (status & 0x08) != 0 {
                    break;
                }
                timeout -= 1;
                if timeout == 0 {
                    return Err("ATA polling timed out");
                }
            }

            // 6. Read 256 16-bit words (512 bytes) from data port
            for i in 0..256 {
                let word = self.data_port.read();
                buffer[i * 2] = (word & 0xFF) as u8;
                buffer[i * 2 + 1] = ((word >> 8) & 0xFF) as u8;
            }
        }

        Ok(())
    }
}
