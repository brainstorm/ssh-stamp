# Purpose of ota-packer

The content of this file is provided for illustrative purposes. For a complete understanding of what this utility does read `ota-packer.rs`.

This binary is a helper cli application to pack binary files together with a header to allow for the sftp-ota procedure to validate the binary before applying the OTA.

## What this tool does

It takes one binary file and adds the following Type Length Value fields (TLV):

- ota type: SSH-Stamp "magic number" used to identify the ota file as SSH-Stamp. Any other value should be rejected in a OTA procedure by an SSH-Stamp binary.
- checksum: SHA256 checksum of the binary. SSH-Stamp will calculate the checksum of the binary uploaded and will abort the OTA if it does not match this field.
- binary length: Additional validation step. SSH-Stamp will only write/validate the announced bytes into flash memory. A target chip with an ota partition smaller than the announced binary length should abort the OTA.

## What this tool does not...

... and it might do in the future:

- Sign the binary
- Add information about the target architecture to help the target instance aborting a wrong binary.

... and will definitely not do:

- Upload the OTA to the target device (The user does this with any standard SFTP client and the appropriate credentials)
- Validate or test in any way the binary

## Usage

For updated information on how to use this tool build and run the binary from the `ssh-stamp/ota` directory

```sh
ssh-stamp/ota$ cargo run --bin ota-packer -- --help
```

At the moment of redaction, this command outputs:

```sh
SSH-Stamp utility 0.1.0 to pack (unpack) OTA update files adding the required metadata.

Usage: ota-packer [OPTIONS] <FILE>

Arguments:
  <FILE>  The file to process

Options:
  -u, --unpack  Unpacks a OTA file. Will save to <file> with .ota.npkd extension
  -p, --pack    (default) Packs a binary file as an OTA file. Will save to <file>.ota
  -h, --help    Print help
```