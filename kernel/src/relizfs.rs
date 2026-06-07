use crate::ata::ATA_DRIVE;
use shared::{Superblock, Inode, DirectoryEntry, RELIZFS_MAGIC, BLOCK_SIZE};

/// Custom file system reader
pub struct RelizFsReader {
    superblock: Superblock,
}

impl RelizFsReader {
    /// Initialize by reading and validating the superblock from sector 2 of the data drive (Primary Slave)
    pub fn init() -> Result<Self, &'static str> {
        let mut sector = [0u8; BLOCK_SIZE];
        
        // Read sector 2 (where Superblock is stored) from Drive 1 (Slave)
        ATA_DRIVE.lock().read_sector(1, 2, &mut sector)?;

        // Safety: Cast bytes to Superblock. Safe because sizes match and alignment is packed.
        let superblock = unsafe {
            let ptr = sector.as_ptr() as *const Superblock;
            ptr.read_unaligned()
        };

        if superblock.magic != RELIZFS_MAGIC {
            return Err("Invalid RelizFS magic number! Partition not formatted.");
        }

        Ok(Self { superblock })
    }

    /// Access the parsed superblock
    pub fn superblock(&self) -> &Superblock {
        &self.superblock
    }

    /// Read an Inode from the Inode Table by its index (0-indexed)
    pub fn read_inode(&self, inode_idx: u32) -> Result<Inode, &'static str> {
        if inode_idx >= self.superblock.inode_count {
            return Err("Inode index out of bounds");
        }

        // Calculate sector and byte offset inside the sector
        let inode_size = core::mem::size_of::<Inode>();
        let byte_offset = (inode_idx as usize) * inode_size;
        let sector_offset = byte_offset / BLOCK_SIZE;
        let sector_byte_index = byte_offset % BLOCK_SIZE;

        let target_sector = self.superblock.inode_table_start + (sector_offset as u32);
        
        let mut sector_buf = [0u8; BLOCK_SIZE];
        ATA_DRIVE.lock().read_sector(1, target_sector, &mut sector_buf)?;

        // Extract Inode
        let inode = unsafe {
            let ptr = sector_buf.as_ptr().add(sector_byte_index) as *const Inode;
            ptr.read_unaligned()
        };

        Ok(inode)
    }

    /// Print all directory entries inside a directory inode
    pub fn list_directory(&self, inode: &Inode) -> Result<(), &'static str> {
        if !inode.is_directory() {
            return Err("Inode is not a directory");
        }

        // Read direct blocks
        for &block_sector in inode.direct_blocks.iter() {
            if block_sector == 0 {
                continue; // Block not allocated
            }

            let mut sector_buf = [0u8; BLOCK_SIZE];
            ATA_DRIVE.lock().read_sector(1, block_sector, &mut sector_buf)?;

            let entry_size = core::mem::size_of::<DirectoryEntry>();
            let entries_count = BLOCK_SIZE / entry_size;

            for i in 0..entries_count {
                let entry = unsafe {
                    let ptr = sector_buf.as_ptr().add(i * entry_size) as *const DirectoryEntry;
                    ptr.read_unaligned()
                };

                if entry.inode_num != 0 || entry.name_len > 0 {
                    // Valid entry!
                    if let Ok(name) = entry.get_name() {
                        let type_str = if name == "." || name == ".." {
                            "<DIR>"
                        } else {
                            // Let's read the target inode to check type
                            if let Ok(target_inode) = self.read_inode(entry.inode_num) {
                                if target_inode.is_directory() { "<DIR>" } else { "<FILE>" }
                            } else {
                                "<ERR>"
                            }
                        };
                        
                        // Display name and type
                        crate::println!("  {: <6}   {: <16}   (inode {})", type_str, name, entry.inode_num);
                    }
                }
            }
        }

        Ok(())
    }

    /// Find an entry inside a directory by its name and return its inode index
    pub fn find_entry(&self, dir_inode: &Inode, name_to_find: &str) -> Result<u32, &'static str> {
        if !dir_inode.is_directory() {
            return Err("Parent is not a directory");
        }

        for &block_sector in dir_inode.direct_blocks.iter() {
            if block_sector == 0 {
                continue;
            }

            let mut sector_buf = [0u8; BLOCK_SIZE];
            ATA_DRIVE.lock().read_sector(1, block_sector, &mut sector_buf)?;

            let entry_size = core::mem::size_of::<DirectoryEntry>();
            let entries_count = BLOCK_SIZE / entry_size;

            for i in 0..entries_count {
                let entry = unsafe {
                    let ptr = sector_buf.as_ptr().add(i * entry_size) as *const DirectoryEntry;
                    ptr.read_unaligned()
                };

                if entry.inode_num != 0 || entry.name_len > 0 {
                    if let Ok(name) = entry.get_name() {
                        if name == name_to_find {
                            return Ok(entry.inode_num);
                        }
                    }
                }
            }
        }

        Err("File not found")
    }

    /// Read file content from an inode into a byte buffer
    pub fn read_file(&self, inode: &Inode, out_buf: &mut [u8]) -> Result<usize, &'static str> {
        if !inode.is_file() {
            return Err("Inode is not a regular file");
        }

        let file_size = inode.size as usize;
        let bytes_to_read = core::cmp::min(file_size, out_buf.len());
        let mut bytes_read = 0;

        for &block_sector in inode.direct_blocks.iter() {
            if block_sector == 0 || bytes_read >= bytes_to_read {
                break;
            }

            let mut sector_buf = [0u8; BLOCK_SIZE];
            ATA_DRIVE.lock().read_sector(1, block_sector, &mut sector_buf)?;

            let remaining = bytes_to_read - bytes_read;
            let chunk_size = core::cmp::min(remaining, BLOCK_SIZE);

            out_buf[bytes_read..bytes_read + chunk_size].copy_from_slice(&sector_buf[..chunk_size]);
            bytes_read += chunk_size;
        }

        Ok(bytes_read)
    }
}
