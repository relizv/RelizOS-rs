use shared::{Superblock, Inode, DirectoryEntry, RELIZFS_MAGIC, BLOCK_SIZE};
use std::fs::File;
use std::io::{Write, Seek, SeekFrom};
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <output_disk_image>", args[0]);
        std::process::exit(1);
    }

    let disk_path = &args[1];
    println!("=== RelizFS Formatter ===");
    println!("Creating disk image at: {}", disk_path);

    // Let's create a 2 MB disk image (4096 blocks/sectors)
    // 4096 * 512 = 2,097,152 bytes
    let total_blocks = 4096u32;
    let disk_size = (total_blocks as usize) * BLOCK_SIZE;

    let mut file = File::create(Path::new(disk_path)).expect("Failed to create disk image file");
    // Pre-allocate the file size
    file.set_len(disk_size as u64).expect("Failed to set disk image size");

    // Layout configuration
    let inode_count = 64; // 8 sectors (64 inodes * 64 bytes = 4096 bytes)
    let inode_table_start = 3;
    let inode_table_blocks = 8;
    let inode_bitmap_start = 11;
    let block_bitmap_start = 12;
    let block_bitmap_blocks = 1; // 1 sector tracks 4096 blocks, which is enough for our 4096-block disk!
    let data_blocks_start = 13u32;
    let data_blocks_count = total_blocks - data_blocks_start;

    let sb = Superblock {
        magic: RELIZFS_MAGIC,
        total_blocks: total_blocks as u32,
        inode_count,
        inode_table_start,
        inode_table_blocks,
        inode_bitmap_start,
        block_bitmap_start,
        data_blocks_start,
        data_blocks_count: data_blocks_count as u32,
    };

    // 1. Write Superblock to sector 2 (offset 1024)
    file.seek(SeekFrom::Start((2 * BLOCK_SIZE) as u64)).unwrap();
    let sb_bytes = unsafe {
        std::slice::from_raw_parts(
            &sb as *const Superblock as *const u8,
            std::mem::size_of::<Superblock>(),
        )
    };
    file.write_all(sb_bytes).unwrap();
    println!("Written Superblock at sector 2");

    // 2. Initialize Inode Bitmap (sector 11)
    // Inode 0 (Root Dir) and Inode 1 (hello.txt) will be allocated.
    // Bit 0 = 1, Bit 1 = 1, others = 0.
    // Byte value = 0b00000011 = 3.
    let mut inode_bitmap = [0u8; BLOCK_SIZE];
    inode_bitmap[0] = 0b0000_0011; 
    file.seek(SeekFrom::Start((inode_bitmap_start as u64) * BLOCK_SIZE as u64)).unwrap();
    file.write_all(&inode_bitmap).unwrap();
    println!("Initialized Inode Bitmap at sector {}", inode_bitmap_start);

    // 3. Initialize Data Block Bitmap (sector 12)
    // Data Block 0 (Root Dir contents) and Data Block 1 (hello.txt contents) will be allocated.
    // Bit 0 = 1, Bit 1 = 1.
    // Byte value = 0b00000011 = 3.
    let mut block_bitmap = [0u8; BLOCK_SIZE];
    block_bitmap[0] = 0b0000_0011;
    file.seek(SeekFrom::Start((block_bitmap_start as u64) * BLOCK_SIZE as u64)).unwrap();
    file.write_all(&block_bitmap).unwrap();
    println!("Initialized Block Bitmap at sector {}", block_bitmap_start);

    // 4. Initialize Inode Table (sectors 3 to 10)
    // Inode 0: Root directory
    // Inode 1: hello.txt
    let mut root_inode = Inode {
        size: BLOCK_SIZE as u64, // 1 block for directory entries
        file_type: 2, // Directory
        reserved_flags: 0,
        direct_blocks: [0; 10],
        single_indirect: 0,
        double_indirect: 0,
        reserved: [0; 6],
    };
    root_inode.direct_blocks[0] = data_blocks_start; // Data block 0 (sector 13)

    let test_file_content = "Hello from RelizFS on Rust UEFI!\nIf you can read this, your custom filesystem driver works perfectly!\n";
    let test_file_bytes = test_file_content.as_bytes();
    
    let mut hello_inode = Inode {
        size: test_file_bytes.len() as u64,
        file_type: 1, // Regular File
        reserved_flags: 0,
        direct_blocks: [0; 10],
        single_indirect: 0,
        double_indirect: 0,
        reserved: [0; 6],
    };
    hello_inode.direct_blocks[0] = data_blocks_start + 1; // Data block 1 (sector 14)

    // Write Inodes to Inode Table
    file.seek(SeekFrom::Start((inode_table_start as u64) * BLOCK_SIZE as u64)).unwrap();
    let inodes_bytes_root = unsafe {
        std::slice::from_raw_parts(
            &root_inode as *const Inode as *const u8,
            std::mem::size_of::<Inode>(),
        )
    };
    file.write_all(inodes_bytes_root).unwrap();

    let inodes_bytes_hello = unsafe {
        std::slice::from_raw_parts(
            &hello_inode as *const Inode as *const u8,
            std::mem::size_of::<Inode>(),
        )
    };
    file.write_all(inodes_bytes_hello).unwrap();
    println!("Written Inodes 0 (root) and 1 (hello.txt) to Inode Table");

    // 5. Write Root Directory entries to Data Block 0 (sector 13)
    // Entries: "." (inode 0), ".." (inode 0), "hello.txt" (inode 1)
    let mut dir_block = [0u8; BLOCK_SIZE];
    
    let entry_self = DirectoryEntry::new(0, ".");
    let entry_parent = DirectoryEntry::new(0, "..");
    let entry_file = DirectoryEntry::new(1, "hello.txt");

    let entry_size = std::mem::size_of::<DirectoryEntry>();
    
    unsafe {
        let ptr = dir_block.as_mut_ptr();
        std::ptr::copy_nonoverlapping(
            &entry_self as *const DirectoryEntry as *const u8,
            ptr.add(0 * entry_size),
            entry_size,
        );
        std::ptr::copy_nonoverlapping(
            &entry_parent as *const DirectoryEntry as *const u8,
            ptr.add(1 * entry_size),
            entry_size,
        );
        std::ptr::copy_nonoverlapping(
            &entry_file as *const DirectoryEntry as *const u8,
            ptr.add(2 * entry_size),
            entry_size,
        );
    }

    file.seek(SeekFrom::Start((data_blocks_start as u64) * BLOCK_SIZE as u64)).unwrap();
    file.write_all(&dir_block).unwrap();
    println!("Written directory entries to sector {}", data_blocks_start);

    // 6. Write hello.txt content to Data Block 1 (sector 14)
    let mut file_block = [0u8; BLOCK_SIZE];
    file_block[..test_file_bytes.len()].copy_from_slice(test_file_bytes);
    
    file.seek(SeekFrom::Start(((data_blocks_start + 1) as u64) * BLOCK_SIZE as u64)).unwrap();
    file.write_all(&file_block).unwrap();
    println!("Written file contents to sector {}", data_blocks_start + 1);

    println!("RelizFS formatting completed successfully!");
}
