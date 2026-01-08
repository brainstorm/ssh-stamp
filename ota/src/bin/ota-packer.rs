use ota::{Header, tlv};

use clap::{ArgAction, Command};
use sha2::{Digest, Sha256};
use std::{io::Write, path::PathBuf};

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
        std::process::exit(unpack_ota(file_path));
    }

    std::process::exit(pack_bin(file_path));
}

// TODO: Optimize memory usage by streaming the file instead of reading it all at once
fn unpack_ota(file_path: PathBuf) -> i32 {
    println!("Unpacking BIN from OTA file {}...", file_path.display());
    let Ok(read) = std::fs::read(&file_path) else {
        eprintln!("Error: Could not read file '{}'", file_path.display(),);
        return 4;
    };
    let header = match Header::deserialize(&read) {
        Ok(header) => {
            println!("Header: {:?} ", header);
            header
        }
        Err(e) => {
            eprintln!(
                "Error: Could not parse OTA header from file '{}': {:?}",
                file_path.display(),
                e
            );
            return 5;
        }
    };

    let mut file_path_bin = file_path.clone();
    file_path_bin.set_extension("ota.unpkd");
    println!("Saving unpacked BIN file to: {}", file_path_bin.display());

    let Ok(mut bin_file) = std::fs::File::create(&file_path_bin) else {
        eprintln!(
            "Error: Could not create BIN file '{}'",
            file_path_bin.display(),
        );
        return 6;
    };
    eprintln!("Nothing written to unpacked file yet... WIP");
    // To be able to deserialize the rest of the file as the binary, I would first need to know where the header ended.
    // Using the length of the header is unreliable so I better instead read the file in stream mode to keep track of the position.

    return 0;
}

// TODO: Optimize memory usage by streaming the file instead of reading it all at once
fn pack_bin(file_path: PathBuf) -> i32 {
    println!("Packing {} as OTA...", file_path.display());

    // TODO: Check read permissions?
    let firmware_size = match file_path.metadata() {
        Ok(metadata) => u32::try_from(metadata.len()).unwrap_or_else(|_| {
            eprintln!(
                "Error: File '{}' is too large (max 4GB supported)",
                file_path.display()
            );
            return 5;
        }),
        Err(e) => {
            eprintln!(
                "Error: Could not retrieve metadata for file '{}': {}",
                file_path.display(),
                e
            );
            return 4;
        }
    };
    println!("Firmware size: {} bytes", firmware_size);

    let mut hasher = Sha256::new();
    let Ok(read) = std::fs::read(&file_path) else {
        eprintln!("Error: Could not read file '{}'", file_path.display(),);
        return 5;
    };
    hasher.update(&read);

    let firmware_sha256 = hasher.finalize();
    println!("Firmware SHA-256: {:x}", firmware_sha256);

    // We could read an u32 from an argument if we want to support multiple OTA types...
    let ota_type = tlv::OTA_TYPE_SSH_STAMP;
    println!("OTA Type Number: {} (SSH-Stamp)", ota_type);

    let mut ota_file_path = file_path.clone();
    ota_file_path.set_extension("ota");

    println!("Saving OTA file to: {}", ota_file_path.display());

    let Ok(mut ota_file) = std::fs::File::create(&ota_file_path) else {
        eprintln!(
            "Error: Could not create OTA file '{}'",
            ota_file_path.display(),
        );
        return 6;
    };

    let mut buf = [0u8; 512];
    // More than enough for the header

    let header_len =
        Header::new(ota_type, firmware_sha256.as_slice(), firmware_size).serialize(&mut buf);

    let Ok(bytes) = ota_file.write(&buf[..header_len]) else {
        eprintln!(
            "Error: Could not write to OTA file '{}'",
            ota_file_path.display(),
        );
        return 5;
    };
    println!("Wrote {} bytes of OTA header", bytes);

    let Ok(bytes) = ota_file.write(&read) else {
        eprintln!(
            "Error: Could not write firmware data to OTA file '{}'",
            ota_file_path.display(),
        );
        return 5;
    };
    println!("Wrote {} bytes of firmware data", bytes);

    0
}
