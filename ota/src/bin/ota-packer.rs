use clap::{ArgAction, Command};
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
        std::process::exit(1);
    }

    println!("Processing file: {}", file_path.display());

    if matches.get_flag("unpack") {
        println!("Unpacking OTA file...");

        std::process::exit(0);
    }

    println!("Packing file as OTA...");

    std::process::exit(0);
}
