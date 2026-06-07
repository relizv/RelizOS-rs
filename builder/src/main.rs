use std::process::Command;
use std::path::Path;

fn main() {
    println!("=== RelizOS Rust Build Pipeline ===");

    // 1. Build the kernel
    println!("[1/4] Building kernel (x86_64-unknown-none)...");
    let status = Command::new("cargo")
        .args(&["build", "--package", "kernel", "--target", "x86_64-unknown-none"])
        .status()
        .expect("Failed to execute cargo build for kernel");
    
    if !status.success() {
        eprintln!("Error: Kernel compilation failed.");
        std::process::exit(1);
    }

    // 2. Build the host-tool
    println!("[2/4] Building host-tool...");
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

    // 3. Format the RelizFS disk image
    println!("[3/4] Formatting RelizFS disk image...");
    let host_tool_bin = if cfg!(windows) {
        "target/debug/host-tool.exe"
    } else {
        "target/debug/host-tool"
    };

    let status = Command::new(host_tool_bin)
        .arg("build/relizfs.img")
        .status()
        .expect("Failed to execute host-tool");

    if !status.success() {
        eprintln!("Error: RelizFS formatting failed.");
        std::process::exit(1);
    }

    // 4. Build the bootable GPT image
    println!("[4/4] Generating UEFI GPT boot image...");
    let kernel_elf = "target/x86_64-unknown-none/debug/kernel";
    let output_gpt = "build/boot_image.gpt";

    let bootloader = bootloader::UefiBoot::new(Path::new(kernel_elf));
    if let Err(e) = bootloader.create_disk_image(Path::new(output_gpt)) {
        eprintln!("Error: Failed to create UEFI boot image: {:?}", e);
        std::process::exit(1);
    }

    println!("\n=== Build Complete ===");
    println!("  Boot Disk:  {}", output_gpt);
    println!("  Data Disk:  build/relizfs.img");
    println!("");
    println!("To run in VirtualBox:");
    println!("  1. Create a 64-bit VM (General -> Version: Other/Unknown 64-bit).");
    println!("  2. In VM Settings -> System -> Motherboard: Check 'Enable EFI (special OSes only)'.");
    println!("  3. In VM Settings -> Storage: Add an IDE Controller.");
    println!("  4. Attach 'build/boot_image.gpt' as IDE Primary Master.");
    println!("  5. Attach 'build/relizfs.img' as IDE Primary Slave.");
    println!("  6. Start the VM!");
}
