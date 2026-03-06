# Intro

Over The Air updates (OTA) is a convenient way to upload the firmware of your target device. In many devices out there you will find that the process is done using a side channel rather rather than the core functionality of the application. 

In SSH-Stamp by definition there is a SSH server running so why not using this secure channel to perform the OTA updates? It only took us implementing a basic SFTP subsystem that uses a SSH tunnel and a helper terminal application, ota-packer, to add some metadata to the SSH-Stamp binary to pack it into an `ota` file.

Using SFTP is good news since any user with access to a sftp client and the keys for the target ssh-stamp device can upload a packed binary to it.


## How to do a OTA over SFTP

**At the moment of writing this the OTA might only work on esp32c6**, but there is no reason why we would not be able to run it in other targets.


### Steps 

Following you can find the steps to perform an OTA. Some steps are optionals.


#### 1. Build and extract app bin from elf file

```
cargo build-esp32c6
espflash save-image --chip=esp32c6 target/riscv32imac-unknown-none-elf/release/ssh-stamp ssh-stamp.bin
```


#### 2. Pack the application for ota:

```
cargo ota-packer -- ssh-stamp.bin
```


#### 3. Run the application

Optionally erase the flash for a fresh test

```
# optional
espflash erase-flash

cargo run-esp32c6 --features sftp-ota
```

At the end of the bootloader log look for the app offset (At this point Factory)
`boot: Loaded app from partition at offset 0x10000`


#### 4. Start the OTA

Keep the debug session and connect your system to the wifi "ssh-stamp" (Using a cheap WiFi dongle helps).

In a separate bash session within the project root folder run:

```
sftp 192.168.0.1 <<< $'put ssh-stamp.ota'
```


#### 5. Wait for the OTA to complete

Wait for the put operation to finish. This might take up to 4 minutes and at times show that the progress is stalled. Be patient.

After that the OTA has been uploaded the target will reboot


#### 6. Check that the OTA worked

At the end of the bootloader messages you will see that the line

`boot: Loaded app from partition at offset 0x10000`

will be replaced by the address of the ota slot 1 partiton:

`boot: Loaded app from partition at offset 0x1f0000`

Repeating the upload process will modify the offset of the app loaded:

`boot: Loaded app from partition at offset 0x10000`

which is the slot 0.

## Automated tests?

We have put together an end to end test to be run into a MCU development board. The file `test-hil-esp32c6-e2e.sh` is a script that step by step prepares the target board, performs an OTA update and checks that the MCU uploaded correctly the OTA (md5) and if the application is running from the right partion offset.

Some of you will prefer to read this file rather than follow a step by step tutorial.

Mind that the script has some prerequisites (see `check_tools()` in the file) and requires your computer to automatically connect to ssid "ssh-stamp".
