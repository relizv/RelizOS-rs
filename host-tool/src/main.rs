use shared::{Superblock, Inode, DirectoryEntry, RELIZFS_MAGIC, BLOCK_SIZE};
use std::fs::OpenOptions;
use std::io::{Write, Seek, SeekFrom};
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <output_disk_image> [sector_offset_sectors]", args[0]);
        std::process::exit(1);
    }

    let disk_path = &args[1];
    let sector_offset = if args.len() >= 3 {
        args[2].parse::<u64>().unwrap_or(0)
    } else {
        0
    };

    println!("=== RelizFS Formatter ===");
    println!("Target Disk Image:   {}", disk_path);
    println!("Partition Offset:    {} sectors ({} bytes)", sector_offset, sector_offset * BLOCK_SIZE as u64);

    // Let's create/format a 2 MB partition (4096 blocks/sectors)
    let total_blocks = 4096u32;
    let required_size = (sector_offset + total_blocks as u64) * BLOCK_SIZE as u64;

    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(Path::new(disk_path))
        .expect("Failed to open disk image file");

    // Pre-allocate the file size if it's smaller than required
    let current_len = file.metadata().unwrap().len();
    if current_len < required_size {
        file.set_len(required_size).expect("Failed to set disk image size");
        println!("Resized image file to {} bytes", required_size);
    }

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
        total_blocks,
        inode_count,
        inode_table_start,
        inode_table_blocks,
        inode_bitmap_start,
        block_bitmap_start,
        data_blocks_start,
        data_blocks_count,
    };

    // 1. Write Superblock to sector 2 of the partition (offset 1024 bytes)
    file.seek(SeekFrom::Start((sector_offset + 2) * BLOCK_SIZE as u64)).unwrap();
    let sb_bytes = unsafe {
        std::slice::from_raw_parts(
            &sb as *const Superblock as *const u8,
            std::mem::size_of::<Superblock>(),
        )
    };
    file.write_all(sb_bytes).unwrap();
    println!("Written Superblock at sector {}", sector_offset + 2);

    // 2. Initialize Inode Bitmap (sector 11 of partition)
    // Inode 0 (Root Dir) and Inode 1 (hello.txt) will be allocated.
    let mut inode_bitmap = [0u8; BLOCK_SIZE];
    inode_bitmap[0] = 0b0000_0011; 
    file.seek(SeekFrom::Start((sector_offset + inode_bitmap_start as u64) * BLOCK_SIZE as u64)).unwrap();
    file.write_all(&inode_bitmap).unwrap();
    println!("Initialized Inode Bitmap at sector {}", sector_offset + inode_bitmap_start as u64);

    // 3. Initialize Data Block Bitmap (sector 12 of partition)
    // Data Block 0 (Root Dir contents) and Data Block 1 (hello.txt contents) will be allocated.
    let mut block_bitmap = [0u8; BLOCK_SIZE];
    block_bitmap[0] = 0b0000_0011;
    file.seek(SeekFrom::Start((sector_offset + block_bitmap_start as u64) * BLOCK_SIZE as u64)).unwrap();
    file.write_all(&block_bitmap).unwrap();
    println!("Initialized Block Bitmap at sector {}", sector_offset + block_bitmap_start as u64);

    // 4. Initialize Inode Table (sectors 3 to 10 of partition)
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
    file.seek(SeekFrom::Start((sector_offset + inode_table_start as u64) * BLOCK_SIZE as u64)).unwrap();
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

    // 5. Write Root Directory entries to Data Block 0 (sector 13 of partition)
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

    file.seek(SeekFrom::Start((sector_offset + data_blocks_start as u64) * BLOCK_SIZE as u64)).unwrap();
    file.write_all(&dir_block).unwrap();
    println!("Written directory entries to sector {}", sector_offset + data_blocks_start as u64);

    // 6. Write hello.txt content to Data Block 1 (sector 14 of partition)
    let mut file_block = [0u8; BLOCK_SIZE];
    file_block[..test_file_bytes.len()].copy_from_slice(test_file_bytes);
    
    file.seek(SeekFrom::Start((sector_offset + data_blocks_start as u64 + 1) * BLOCK_SIZE as u64)).unwrap();
    file.write_all(&file_block).unwrap();
    println!("Written file contents to sector {}", sector_offset + data_blocks_start as u64 + 1);

    println!("RelizFS partitioning/formatting completed successfully!");
}
