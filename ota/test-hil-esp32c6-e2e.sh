#!/bin/bash

# build and test OTA on ESP32-C6 HIL It requires that the user has set up the HIL environment
# the target must be reachable via espflash
# and the system must automatically connect it to the wifi network created by the board
# There is also a collection of tools required to run the script ( see check_tools() )
# TODO: Read the partitions.csv to obtain the OTA_1_OFFSET
# TODO: Consider replacing this script with [rexpect](https://crates.io/crates/rexpect) to avoid having to learn tcl language

EXIT_BAD_DIR=100 # The script is not being run from the project root, or the Cargo.toml file does not contain the expected content. Please run it from the correct location.
EXIT_INTERRUPTED=101 #
EXIT_MISSING_TOOL=102 # One of the required tools is not installed or not in PATH. See check_tools() function for details.
EXIT_UNREACHABLE=103 # The board is not reachable in the target network after flashing the app, or it does not open the SSH port within the expected time after OTA update
EXIT_SFTP_FAILED=104 # The SFTP upload of the OTA file failed or timed out.
EXIT_BAD_MD5=105 # The MD5 checksum of the OTA partition does not match the local BIN file, indicating an issue with the OTA update process.
EXIT_BAD_PARTITION=106 # The app partition does not match the expected partition in OTA_1_OFFSET

# Borrowed from https://github.com/mkj/sunset ci tests
if ! grep -sq '^name = "ssh-stamp"' Cargo.toml; then
    echo "Run $0 from the toplevel ssh-stamp directory"
    exit $EXIT_BAD_DIR
fi


SSH_STAMP_ELF="target/riscv32imac-unknown-none-elf/release/ssh-stamp"

RETRIES=30
RETRY_DELAY=2
DEVICE_IP="192.168.4.1"
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

show_board_info(){
    echo "Trying to contact board via espflash"
    espflash board-info
    echo "Board is responding"
}

build_app(){
    echo "Building esp32c6 binary"
    cargo build-esp32c6 --features sftp-ota
}

pack_ota(){
    mkdir -p $OUTPUT_DIR
    echo "saving app binary to app.bin"
    espflash save-image --chip esp32c6 $SSH_STAMP_ELF $OUTPUT_DIR/app.bin
    echo "saving app binary to app.ota"
    cargo ota-packer -- $OUTPUT_DIR/app.bin
}

clean_flash(){
    echo "Erasing the target device flash"
    espflash erase-flash
}

flash_app(){
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
    local OTA_FILE=$(realpath "$OUTPUT_DIR/app.ota")
    echo "Uploading $OTA_FILE to $DEVICE_IP via SFTP (OTA update)"
    echo "Will timeout after $OTA_UPLOAD_TIMEOUT if not completed"
    
    export DEVICE_IP OTA_FILE OTA_UPLOAD_TIMEOUT # Making them available to expect  
    expect <<-'EOF'
    set timeout $env(OTA_UPLOAD_TIMEOUT)
    spawn sftp -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null "user@$env(DEVICE_IP)"
    expect "password:"
    send "\r"
    expect "sftp>"
    send "put \"$env(OTA_FILE)\"\r"
    expect "sftp>"
    # Rebooting is rough ATM, the target restarts before closing the channel
    # send "bye\r" 
    # expect eof
EOF

    
    OTA_UPLOAD_RESULT=$?
    if [ $OTA_UPLOAD_RESULT -ne 0 ]; then
        echo "Error: SFTP upload failed or timed out"
        exit $EXIT_SFTP_FAILED
    fi
    
    echo "OTA upload complete. Waiting for the device to reboot and apply the update."
}


check_ota_partition_md5(){
    local BIN_FILE="$(realpath $OUTPUT_DIR/app.bin)"
    
    LOCAL_MD5=$(md5sum $BIN_FILE | sed -e 's/ .*//' -e 's/^0//')
    echo "Local BIN file MD5: $LOCAL_MD5"
    LOCAL_LENGTH=$(stat -c%s "$OUTPUT_DIR/app.bin"| sed -e 's/ *$//')
    echo "Local BIN file length: $LOCAL_LENGTH"

    FLASHED_MD5=$(espflash checksum-md5 $OTA_1_OFFSET $LOCAL_LENGTH | tail -n 1 | sed -e 's/0x0//' -e 's/^0x//')
    echo "Flashed OTA partition MD5: $FLASHED_MD5"

    if [ "$LOCAL_MD5" == "$FLASHED_MD5" ]; then
        echo "OTA update verified successfully: MD5 checksums match."
    else
        echo "Error: MD5 checksums do not match! OTA update may have failed."
        exit $EXIT_BAD_MD5
    fi
}

check_app_offset(){
# "I (344) boot: Loaded app from partition at offset 0x1f0000"
    export OTA_1_OFFSET OTA_UPLOAD_TIMEOUT EXIT_BAD_PARTITION
    expect <<'EOF'
    set timeout $env(OTA_UPLOAD_TIMEOUT)
    spawn espflash monitor
    
    # Wait for the command prompt or EOF
    expect {
        -re {Commands:} {
            puts "Received Commands. Time to reset the board"
            sleep 1
        }
        timeout {
            puts "ERROR: espflash monitor finished without prompting commands. Bad port?"
            exit 1
        }
        eof {
            puts "ERROR: espflash monitor finished without prompting commands. Bad port?"
            exit 1
        }
    }

    # ascii code for Ctrl+r or Device Ctrl 2
    puts "Sending return carriage and CTRL+R"
    send  "\r" 
    sleep 0.5
    send  "\x12" 
    
    
    expect {
        -re {boot: Loaded app from partition at offset (.*?)\r} {
            puts "\n\n\n\rOutput after sending CTRL+R:\n"
            set app_offset_output $expect_out(1,string)
            
            if { $env(OTA_1_OFFSET) == $app_offset_output } { 
                puts "Offset match the expected ota partition offset: $app_offset_output"
                exit 0
            } else { 
                puts "Offset do not match the expected ota partition offset: expected='$env(OTA_1_OFFSET)' vs obtained='$app_offset_output'"
                exit $env(EXIT_BAD_PARTITION)
            }
        }
        eof {
            puts "ERROR: espflash monitor finished without an app offset. Bad regexp?"
            exit 1
        }
    }

    sleep 10
EOF
}

set -v # Print commands as they are executed for better visibility in CI logs
set -e # Exit immediately if a command exits with a non-zero status

check_tools

show_board_info

build_app

pack_ota

clean_flash

flash_app

reach_board

reach_app

run_sftp_ota

check_ota_partition_md5

check_app_offset

echo "OTA test completed successfully."
exit 0
