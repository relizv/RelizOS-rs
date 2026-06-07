#![no_std]

/// RelizFS constants
pub const RELIZFS_MAGIC: u64 = 0x52454C495A4653; // "RELIZFS" in ASCII
pub const BLOCK_SIZE: usize = 512;               // Block size matches sector size for simplicity
pub const DIRECT_BLOCKS_COUNT: usize = 10;
pub const MAX_FILENAME_LEN: usize = 27;

/// Superblock structure (located at Sector 2 of the partition)
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct Superblock {
    pub magic: u64,            // Magic number to identify RelizFS
    pub total_blocks: u32,     // Total number of blocks in the FS partition
    pub inode_count: u32,      // Total number of inodes available
    pub inode_table_start: u32,// Start sector of the Inode Table
    pub inode_table_blocks: u32,// Number of blocks/sectors for the Inode Table
    pub inode_bitmap_start: u32,// Start sector of the Inode Bitmap
    pub block_bitmap_start: u32,// Start sector of the Data Block Bitmap
    pub data_blocks_start: u32, // Start sector of the Data Blocks
    pub data_blocks_count: u32, // Number of data blocks
}

/// Inode structure (64 bytes, 8 inodes per 512-byte sector)
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct Inode {
    pub size: u64,                                  // Size of the file in bytes
    pub file_type: u8,                              // 0 = Free/Unused, 1 = Regular File, 2 = Directory
    pub reserved_flags: u8,                         // Extra flags (permissions, etc.)
    pub direct_blocks: [u32; DIRECT_BLOCKS_COUNT],   // Direct block addresses (sectors)
    pub single_indirect: u32,                       // Address of single indirect block sector
    pub double_indirect: u32,                       // Address of double indirect block sector
    pub reserved: [u8; 6],                          // Padding to make it exactly 64 bytes
}

impl Inode {
    /// Helper to check if the inode is unused/free
    pub fn is_free(&self) -> bool {
        self.file_type == 0
    }

    /// Helper to check if the inode represents a directory
    pub fn is_directory(&self) -> bool {
        self.file_type == 2
    }

    /// Helper to check if the inode represents a regular file
    pub fn is_file(&self) -> bool {
        self.file_type == 1
    }
}

/// Directory Entry structure (32 bytes, 16 entries per 512-byte data block)
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct DirectoryEntry {
    pub inode_num: u32,                      // Inode number of the entry
    pub name_len: u8,                        // Length of the file name
    pub name: [u8; MAX_FILENAME_LEN],        // File name (padded with zeros)
}

impl DirectoryEntry {
    /// Create a new directory entry
    pub fn new(inode_num: u32, name_str: &str) -> Self {
        let mut name = [0u8; MAX_FILENAME_LEN];
        let bytes = name_str.as_bytes();
        let len = core::cmp::min(bytes.len(), MAX_FILENAME_LEN);
        name[..len].copy_from_slice(&bytes[..len]);

        Self {
            inode_num,
            name_len: len as u8,
            name,
        }
    }

    /// Get the file name as a string slice (if valid UTF-8)
    pub fn get_name(&self) -> Result<&str, core::str::Utf8Error> {
        let len = self.name_len as usize;
        let active_len = core::cmp::min(len, MAX_FILENAME_LEN);
        core::str::from_utf8(&self.name[..active_len])
    }
}
