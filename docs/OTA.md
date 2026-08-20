<!--
SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>

SPDX-License-Identifier: GPL-3.0-or-later
-->

## Firmware updates over SFTP

Devices built with the `sftp-ota` feature accept a new firmware image over the
SSH connection they already serve. There is no separate update port, protocol
or bootloader shell: the image is written with a plain `sftp` client.

### Packing an image

The device does not accept a raw binary. It expects an `.ota` file, which is
the binary prefixed with a short TLV header describing it. `packer` produces
one:

```
cargo packer -- --target esp32c6 target/riscv32imac-unknown-none-elf/release/ssh-stamp-esp32.bin
```

This writes `ssh-stamp-esp32.ota` next to the input. `--target` records the
chip the image was built for; see [Target checking](#target-checking) below.
`packer --unpack` reverses the process and verifies the recorded checksum,
which is a quick way to inspect an image you did not build yourself.

### Uploading

```
sftp root@192.168.4.1
> put ssh-stamp-esp32.ota
```

The device streams the upload straight into the inactive OTA partition,
hashing as it goes. When the last byte arrives it compares the hash against
the checksum in the header; only on a match does it mark the partition
bootable and reset into the new image. A transfer that is interrupted,
truncated or corrupted leaves the running firmware untouched, because the
partition is never marked bootable.

The SFTP subsystem is exclusive: while an upload is in progress the device
will not also serve a shell or a bridge session.

### Target checking

A firmware image for the wrong chip is not merely useless, it is a brick
waiting to happen. Since the image carries no chip identity of its own, the
packer records one for it.

`packer --target <chip>` adds a `TargetChip` TLV holding the chip name exactly
as `esp_hal::chip!()` spells it — `esp32c6`, `esp32s3`, and so on. The record
is written immediately after the OTA type, so it lands within the first ~15
bytes of the transfer. A device that reads a name other than its own aborts
the write there and then with `SSH_FX_OP_UNSUPPORTED`; the client reports a
failed `put` after a few bytes instead of after a full upload.

`--target` is optional. An image packed without it has no `TargetChip` record,
cannot be screened, and is accepted with a warning on the device console —
which is what keeps images packed before this record existed working. Passing
the flag is strongly recommended; omitting it trades away the only cheap
protection against flashing the wrong chip.

Note the compatibility direction: an image packed **with** `--target` is
rejected by firmware predating the `TargetChip` record, because that firmware
treats any unrecognised TLV as fatal. Update the device first over USB, or
pack without `--target`, when crossing that boundary.

### Header format

The header is a sequence of type-length-value records, each with a one-byte
type and a one-byte length:

| Type | Name             | Length | Value                                        |
| ---- | ---------------- | ------ | -------------------------------------------- |
| 0    | `OtaType`        | 4      | `0x73736873` (`sshs`). **Must come first**   |
| 3    | `TargetChip`     | 1–16   | Chip name, UTF-8. Optional; should come second |
| 2    | `Sha256Checksum` | 32     | SHA-256 of the firmware blob                 |
| 1    | `FirmwareBlob`   | 4      | Blob length in bytes. **Must come last**     |

The firmware blob follows the `FirmwareBlob` record immediately; anything
after that point is blob, not header.

Ordering is enforced, not conventional. `OtaType` first means a file that is
not an ssh-stamp image is rejected on its first four bytes. `FirmwareBlob`
last means the device knows the size and the expected hash before it commits
a single byte to flash.

An unrecognised type is fatal: the device rejects the image rather than
skipping the record. That is deliberate. A parser that ignores what it does
not understand will happily accept a header assembled to mean one thing to the
packer and another to the device, and [Radically Open Security's audit of this
code](https://github.com/brainstorm/ssh-stamp/issues/76) flagged exactly that
shape. Forward compatibility, if it is wanted later, needs an explicit
mechanism — a critical/non-critical split in the type space, so that skipping
is a property the format grants a record rather than a default the parser
applies to everything — and not a silent `continue`.

### Target support

`sftp-ota` builds for every board below. "Verified on hardware" means an image
has actually been uploaded to the board and booted:

| Board                                | Chip    | Builds | Verified on hardware |
| ------------------------------------ | ------- | ------ | -------------------- |
| esp32c6-devkitc                      | esp32c6 | yes    | yes                  |
| esp32c6-generic                      | esp32c6 | yes    | no                   |
| esp32c5-devkitc                      | esp32c5 | yes    | no                   |
| esp32c61-devkitc                     | esp32c61| yes    | no                   |
| esp32-s2-saola                       | esp32s2 | yes    | no                   |
| waveshare-esp32-s3-touch-lcd-43      | esp32s3 | yes    | no                   |

Chips with an IC feature but no board definition (`esp32`, `esp32c2`,
`esp32c3`, `esp32s3` bare) build as a library only and have no OTA path to
exercise until a board is added.

Building is a weak claim: it says the OTA partition layout and flash driver
compile for the chip, not that the bootloader accepts what this code writes.
Treat anything in the last column marked "no" as untested.
