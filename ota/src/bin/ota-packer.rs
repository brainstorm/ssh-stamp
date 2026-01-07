use ota::ota_tlv;

use clap::{ArgAction, Command};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

fn main() {
    let matches = Command::new("ota-packer")
        .about("SSH-Stamp utility to pack (unpack) OTA update files adding the required metadata.")
        .arg(clap::arg!(<FILE> "The file to process").required(true))
        .arg(
            clap::arg!(-u --unpack "Unpacks a OTA file. Will save to <file> with .ota.unpkd extension")
                .action(ArgAction::SetTrue)
                .conflicts_with("pack"),
        )
        .arg(
            clap::arg!(-p --pack "(default) Packs a binary file as an OTA file. Will save to <file>.ota")
                .action(ArgAction::SetTrue)
                .conflicts_with("unpack"),
        )
        .get_matches();
    let Some(file_path) = matches.get_one::<String>("FILE") else {
        eprintln!("Error: No file provided");
        std::process::exit(1);
    };

    let file_path = PathBuf::from(file_path);
    if !file_path.exists() {
        eprintln!("Error: File '{}' does not exist", file_path.display());
        std::process::exit(2);
    }
    if !file_path.is_file() {
        eprintln!(
            "Error: File '{}' is not a regular file",
            file_path.display()
        );
        std::process::exit(3);
    }

    if matches.get_flag("unpack") {
        println!("Unpacking OTA file...");

        std::process::exit(0);
    }

    println!("Packing {} as OTA...", file_path.display());

    // TODO: Check read permissions?
    let firmware_size = match file_path.metadata() {
        Ok(metadata) => u32::try_from(metadata.len()).unwrap_or_else(|_| {
            eprintln!(
                "Error: File '{}' is too large (max 4GB supported)",
                file_path.display()
            );
            std::process::exit(5);
        }),
        Err(e) => {
            eprintln!(
                "Error: Could not retrieve metadata for file '{}': {}",
                file_path.display(),
                e
            );
            std::process::exit(4);
        }
    };
    println!("Firmware size: {} bytes", firmware_size);

    let mut hasher = Sha256::new();
    hasher.update(std::fs::read(&file_path).unwrap_or_else(|e| {
        eprintln!(
            "Error: Could not read file '{}': {}",
            file_path.display(),
            e
        );
        std::process::exit(5);
    }));
    let firmware_sha256 = hasher.finalize();
    println!("Firmware SHA-256: {:x}", firmware_sha256);

    // We could read an u32 from an argument if we want to support multiple OTA types...
    let ota_type = ota_tlv::OTA_TYPE_SSH_STAMP;
    println!("OTA Type Number: {} (SSH-Stamp)", ota_type);

    let mut ota_file_path = file_path.clone();
    ota_file_path.set_extension("ota");
    println!("Saving OTA file to: {}", ota_file_path.display());
    // let mut temp_buf = [0u8; ota_tlv::OTA_TLV_TOTAL_HEADER_SIZE as usize];
    std::process::exit(0);
}
