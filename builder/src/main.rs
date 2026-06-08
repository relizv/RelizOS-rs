use std::process::Command;
use std::path::Path;

fn main() {
    println!("=== RelizOS Rust Build Pipeline ===");

    // 1. Build the kernel
    println!("[1/5] Building kernel (x86_64-unknown-none)...");
    let status = Command::new("cargo")
        .args(&["build", "--package", "kernel", "--target", "x86_64-unknown-none"])
        .status()
        .expect("Failed to execute cargo build for kernel");
    
    if !status.success() {
        eprintln!("Error: Kernel compilation failed.");
        std::process::exit(1);
    }

    // 2. Build the host-tool
    println!("[2/5] Building host-tool...");
    let status = Command::new("cargo")
        .args(&["build", "--package", "host-tool"])
        .status()
        .expect("Failed to execute cargo build for host-tool");
    
    if !status.success() {
        eprintln!("Error: host-tool compilation failed.");
        std::process::exit(1);
    }

    // Create build directory if it doesn't exist
    std::fs::create_dir_all("build").unwrap();

    // 3. Generate UEFI boot image
    println!("[3/5] Generating temporary UEFI GPT boot image...");
    let kernel_elf = "target/x86_64-unknown-none/debug/kernel";
    let temp_gpt = "build/temp_boot.gpt";

    let bootloader = bootloader::UefiBoot::new(Path::new(kernel_elf));
    if let Err(e) = bootloader.create_disk_image(Path::new(temp_gpt)) {
        eprintln!("Error: Failed to create UEFI boot image: {:?}", e);
        std::process::exit(1);
    }

    // 4. Create combined image and copy boot partition
    println!("[4/5] Merging boot image and allocating data space...");
    let output_img = "build/relizos-rust.img";
    
    let mut temp_file = std::fs::File::open(temp_gpt).expect("Failed to open temporary boot image");
    let mut output_file = std::fs::File::create(output_img).expect("Failed to create combined image");
    
    std::io::copy(&mut temp_file, &mut output_file).expect("Failed to copy bootloader partition");
    
    // Extend combined image to 30 MB (30 * 1024 * 1024 bytes = 61440 sectors)
    // RelizFS starts at sector 40000 (approx 20.48 MB) and is 2 MB (4096 sectors) in size.
    output_file.set_len(30 * 1024 * 1024).expect("Failed to allocate 30MB combined disk image");
    
    // Explicitly drop files to release handles
    drop(temp_file);
    drop(output_file);

    // Clean up temporary boot image
    std::fs::remove_file(temp_gpt).expect("Failed to remove temporary boot image file");

    // 5. Format the RelizFS partition inside the combined image
    println!("[5/5] Formatting RelizFS partition at sector 40000...");
    let host_tool_bin = if cfg!(windows) {
        "target/debug/host-tool.exe"
    } else {
        "target/debug/host-tool"
    };

    let status = Command::new(host_tool_bin)
        .args(&[output_img, "40000"])
        .status()
        .expect("Failed to execute host-tool");

    if !status.success() {
        eprintln!("Error: RelizFS partitioning/formatting failed.");
        std::process::exit(1);
    }

    println!("\n=== Build Complete ===");
    println!("  Unified Disk Image:  {}", output_img);
    println!("");
    println!("To run in VirtualBox:");
    println!("  1. Create a 64-bit VM (General -> Version: Other/Unknown 64-bit).");
    println!("  2. In VM Settings -> System -> Motherboard: Check 'Enable EFI (special OSes only)'.");
    println!("  3. In VM Settings -> Storage: Add an IDE Controller.");
    println!("  4. Attach '{}' as IDE Primary Master.", output_img);
    println!("  5. Start the VM!");
}
