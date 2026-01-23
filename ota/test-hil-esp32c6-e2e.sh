#!/bin/bash

# build and test OTA on ESP32-C6 HIL It requires that the user has set up the HIL environment
# the target must be reachable via espflash
# and the system must automatically connect it to the wifi network created by the board
# There is also a collection of tools required to run the script ( see check_tools() )
# TODO: Is there a way in espflash to obtain current running app partition?
# TODO: Read the partitions.csv to obtain the OTA_1_OFFSET
# TODO: Interrupting expect does not work

EXIT_BAD_DIR=100
EXIT_UNREACHABLE=101
EXIT_INTERRUPTED=102
EXIT_MISSING_TOOL=103
EXIT_BAD_MD5=104

# Borrowed from https://github.com/mkj/sunset ci tests
if ! grep -sq '^name = "ssh-stamp"' Cargo.toml; then
    echo "Run $0 from the toplevel ssh-stamp directory"
    exit $EXIT_BAD_DIR
fi


SSH_STAMP_ELF="target/riscv32imac-unknown-none-elf/release/ssh-stamp"

RETRIES=30
RETRY_DELAY=1
DEVICE_IP="192.168.0.1"
OTA_1_OFFSET=0x1f0000 # Offset of ota_1 partition in partitions.csv. Reading this could be automated. Automating everything is a rabbit hole
OTA_UPLOAD_TIMEOUT=300s
OUTPUT_DIR="./target/ci"


check_tools() {
    local missing_tools=()
    
    for tool in expect espflash nc; do
        if ! command -v $tool &> /dev/null; then
            missing_tools+=($tool)
        fi
    done
    
    if [ ${#missing_tools[@]} -ne 0 ]; then
        echo "Error: Missing required tools: ${missing_tools[*]}"
        echo "Please install them:"
        echo "  - expect: sudo apt-get install expect (Debian/Ubuntu)"
        echo "  - espflash: cargo install espflash"
        echo "  - nc: sudo apt-get install netcat (Debian/Ubuntu)"
        exit $EXIT_MISSING_TOOL
    fi
    
    echo "All required tools are installed"
}

check_tools

show_board_info(){
    echo "Trying to contact board via espflash"
    espflash board-info
    echo "Board is responding"
}

build_app(){
    echo "Building esp32c6 binary"
    cargo build-esp32c6
}

pack_ota(){
    mkdir -p $OUTPUT_DIR
    echo "saving app binary to app.bin"
    espflash save-image --chip esp32c6 $SSH_STAMP_ELF $OUTPUT_DIR/app.bin
    echo "saving app binary to app.ota"
    cargo ota-packer -- $OUTPUT_DIR/app.bin
}

clean_flash_and_flash_app(){
    echo "Erasing the target device flash"
    espflash erase-flash
    echo "Flashing the board with the application"
    espflash flash --baud=921600 --partition-table partitions.csv $SSH_STAMP_ELF
}

reach_app(){
    echo "Waiting for the board to be reachable in the target network"
    sleep $RETRY_DELAY
    set +e
    for i in $(seq 1 $RETRIES); do
        echo "Attempt $i/$RETRIES: Checking SSH port on $DEVICE_IP..."
        if timeout 2 nc -zv $DEVICE_IP 22 2>&1 | grep -q succeeded; then
            echo "SSH port is open on $DEVICE_IP"
            set -e 
            break
        fi
        
        if [ $i -eq $RETRIES ]; then
            echo "Error: SSH port not reachable after $RETRIES attempts"
            exit $EXIT_UNREACHABLE
        fi
        
        sleep $RETRY_DELAY
    done
    set -e 
    echo "Board is reachable via SSH"

}

reach_board(){
    echo "Pinging the board to check reachability"
    sleep $RETRY_DELAY
    for i in $(seq 1 $RETRIES); do
        echo "Attempt $i/$RETRIES: Pinging $DEVICE_IP..."
        if ping -c 1 $DEVICE_IP &> /dev/null; then
            echo "Board responded to ping"
            break
        fi
        
        if [ $i -eq $RETRIES ]; then
            echo "Error: Board not reachable via ping after $RETRIES attempts"
            exit $EXIT_UNREACHABLE
        fi
        
        sleep $RETRY_DELAY
    done
}

# TODO: Does not work
# Trap Ctrl+C to cleanup
cleanup() {
    echo ""
    echo "Caught interrupt signal, cleaning up..."
    pkill -P $$ expect 2>/dev/null || true
    pkill -P $$ sftp 2>/dev/null || true
    exit $EXIT_INTERRUPTED
}
trap cleanup SIGINT SIGTERM


run_sftp_ota(){
    local OTA_FILE="$(realpath $OUTPUT_DIR/app.ota)"
    echo "Uploading $OTA_FILE to $DEVICE_IP via SFTP (OTA update)"
    echo "Will timeout after $OTA_UPLOAD_TIMEOUT if not completed"
    
    # Temp script as a workaround to pass ENV to expect
    cat > $OUTPUT_DIR/sftp_upload.exp <<EOF
set timeout -1
spawn sftp user@$DEVICE_IP
expect "password:"
send "\r"
expect "sftp>"
send "put $OTA_FILE\r"
expect "sftp>"
EOF
    
    timeout $OTA_UPLOAD_TIMEOUT expect $OUTPUT_DIR/sftp_upload.exp || { echo "SFTP operation timed out or failed"; cleanup; }
    rm -f $OUTPUT_DIR/sftp_upload.exp
    
    echo "OTA upload complete. Waiting for the device to reboot and apply the update."
}


validate_ota(){
    local BIN_FILE="$(realpath $OUTPUT_DIR/app.bin)"
    
    LOCAL_MD5=$(md5sum $BIN_FILE | awk '{printf "0x%s", $1}')
    LOCAL_LENGTH=$(stat -c%s "$OUTPUT_DIR/app.bin"| awk '{printf "0x%x", $1}')
    echo "Local BIN file MD5: $LOCAL_MD5"
    # echo "Local BIN file length: $LOCAL_LENGTH bytes"

    FLASHED_MD5=$(espflash checksum-md5 $OTA_1_OFFSET $LOCAL_LENGTH | tail -n 1)
    echo "Flashed OTA partition MD5: $FLASHED_MD5"

    if [ "$LOCAL_MD5" == "$FLASHED_MD5" ]; then
        echo "OTA update verified successfully: MD5 checksums match."
    else
        echo "Error: MD5 checksums do not match! OTA update may have failed."
        exit $EXIT_BAD_MD5
    fi
}

set -v
set -e 

check_tools

show_board_info

build_app

pack_ota

clean_flash_and_flash_app

reach_board

reach_app

run_sftp_ota

validate_ota

echo "OTA test completed successfully."
exit 0