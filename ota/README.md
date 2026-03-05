# How to do a OTA over SFTP

**At the moment of writing this the OTA might only work on esp32c6**, but there is no reason why we would not be able to run it in other targets.

In SSH-Stamp by definition there is a SSH server running so why not using this secure channel to perform Over The Air (OTA) updates? It only took to implement a basic SFTP subsystem and we have all the ingredients that we need for it.

## Steps 

Following you can find the steps to perform an OTA. Some steps are optionals.

### 1. Build and extract app bin from elf file

```
cargo build-esp32c6
espflash save-image --chip=esp32c6 target/riscv32imac-unknown-none-elf/release/ssh-stamp ssh-stamp.bin
```


### 2. Pack the application for ota:

```
cargo ota-packer -- ssh-stamp.bin
```


### 3. Run the application

Optionally erase the flash for a fresh test

```
# optional
espflash erase-flash

cargo run-esp32c6
```

At the end of the bootloader log look for the app offset (At this point Factory)
`boot: Loaded app from partition at offset 0x10000`


### 4. Start the OTA

Keep the debug session and connect your system to the wifi "ssh-stamp" (Using a cheap WiFi dongle helps).

In a separate bash session within the project root folder run:

```
sftp 192.168.0.1 <<< $'put ssh-stamp.ota'
```


### 5. Wait for the OTA to complete

Wait for the put operation to finish. This might take up to 4 minutes and at times show that the progress is stalled. Be patient.

After that the OTA has been uploaded the target will reboot


### 6. Check that the OTA worked

At the end of the bootloader messages you will see that the line

`boot: Loaded app from partition at offset 0x10000`

will be replaced by the address of the ota slot 0 partiton:

`boot: Loaded app from partition at offset 0x210000`

Repeating the upload process will modify the offset of the app loaded:

`boot: Loaded app from partition at offset 0x410000`
