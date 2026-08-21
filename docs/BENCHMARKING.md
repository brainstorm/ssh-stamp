<!--
SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>

SPDX-License-Identifier: GPL-3.0-or-later
-->

# Benchmarking

The ssh-stamp codebase implements benchmarking in two ways:

- **Firmware size** which needs no hardware. The CI measures the boards on push and PR via `.github/workflows/size.yml`.
- **On-device metrics** needs a physical board. This measures the boot timeline, KEX metrics, RTT, and the heap and 
  stack usage. These can be uploaded manually as part of a PR.

Both these kinds of measurements are stored in bencher, under:

https://bencher.dev/console/projects/ssh-stamp/testbeds?per_page=8&page=1

For the manual benches, each board gets its own testbed, whereas the CI compares
the size in one `ci` testbed.

The benchmarking is implemented inside ssh-stamp by the `xtask`:

| Subcommand          | Needs hardware | What it does                                                                             |
|---------------------|----------------|------------------------------------------------------------------------------------------|
| `cargo xtask bench` | yes            | Build, flash, run SSH sessions, measuring boot, KEX, RTT, heap and stack.                |
| `cargo xtask size`  | no             | Build the release firmware, measuring the flash and RAM usage, as well as `cargo bloat`. |
| `cargo xtask bmf`   | no             | Convert `bench`/`size` JSON output into the Bencher Metric Format.                       |

## What the benchmarking measures

The following data points are measured:

- **Boot timeline**, time from reset to each startup checkpoint, ending at "TCP listening".
- **Key exchange**, the time from TCP accept to the first auth, per session, for a KEX algorithm (i.e.
  `curve25519-sha256` vs `mlkem768x25519-sha256`).
- **Bridge round trips**, latency of a byte making it from the SSH client to the UART and back.
- **Heap**, used, total and max bytes of the allocator at labelled
  points: `boot`, `peripherals`, `wifi_up`, and a heap probe that
  finds the smallest heap the firmware still boots with.
- **Stack**, maximum stack usage of a real run, measured by painting the
  stack reservation and scanning it.
- **Flash and RAM**, the size of the flashed application image and
  of everything placed in internal RAM.

## Implementation of benchmarking

Benchmarking is implemented on device by using the `mem-probe` feature. The firmware
emits a record that signals a benchmarking value, which is parsed by the host:

```text
@BENCH checkpoint=bench_tcp_listening t_us=412300
@BENCH kex=accept->firstauth elapsed_us=285700
@BENCH heap=wifi_up used_bytes=53900 total_bytes=73728 max_bytes=61688 ...
```

The startup checkpoints are `bench_boot`, `bench_peripherals_ready`,
`bench_wifi_up` and `bench_tcp_listening`. The session checkpoints
are `bench_tcp_accept`, `bench_kex_complete`, `bench_auth_success`,
`bench_channel_open`.

For parsing, the host looks for these `@BENCH` lines, and then parses a `key=value` measurement.

## `xtask bench`

The `xtask bench` command performs a benchmark against a physical board:

```bash
cargo xtask bench --board esp32c6-devkitc --kex curve25519-sha256 -o bench-curve25519.json
```

Since the KEX is a single algorithm per invocation, both should be tested for a complete picture:

```bash
cargo xtask bench --board esp32c6-devkitc --kex mlkem768x25519-sha256 -o bench-mlkem.json
```

Note that mlkem generally requires a newer version of OpenSSH (e.g. version 10), and that these
commands expect a public key to be available under `~/.ssh/id_ed25519.pub` for first auth.

The general process that happens per run:

* Build and flash the firmware with the right features.
* Paint the stack to measure the stack usage using `probe-rs`.
* Boot the actual board and take boot measurements.
* Join the device access point automatically and enrol a public key if needed.
* Run the sessions, read the stack painting, and then output the results.

### Heap usage probe

There is an additional option that can determine the minimum bootable heap size for a board called `--heap`:

```bash
cargo xtask bench --board esp32c6-devkitc --kex mlkem768x25519-sha256 --heap 49152,57344,65536,73728
```

This will test each value with the heap size, and see if it actually runs, and is useful
for determining the heap usage requirements for a board.

## `xtask size`

The size command builds the image and measures flash and RAM usage without a physical board:

```bash
cargo xtask size --all -o size.json
```

This uses the release build, and measures:

- **flash**, the ESP-IDF application image, assembled from the ELF with headers, padding
  and the partition table accounted for.
- **RAM**, all loadable segments which addresses falls inside the board's
  internal-RAM windows.
- **stack reservation**, the `_stack_start`/`_stack_end` span the linker sets.

The `cargo-bloat` command should also be installed to see which crates contribute to the usage.

## `xtask bmf`

This converts the output of the previous commands to the [Bencher Metric Format][bmf]:

```bash
cargo xtask bmf --input bench-curve25519.json --input bench-mlkem.json --input size.json -o bmf.json
```

This will merge any mix of `bench` and `size` JSON files, and is used for uploading results to Bencher.

The following values are defined:

| Benchmark name            | Measure        | Value                                            |
|---------------------------|----------------|--------------------------------------------------|
| `kex/<algorithm>`         | `latency-us`   | median accept to first auth, with min/max bounds |
| `bridge/rtt`              | `latency-us`   | median loopback round trip, with min/max bounds  |
| `boot`                    | `latency-us`   | reset to `bench_tcp_listening`                   |
| `boot/wifi-association`   | `latency-us`   | `bench_wifi_up` − `bench_peripherals_ready`      |
| `heap/<label>`            | `heap-bytes`   | used bytes at `boot` / `peripherals` / `wifi_up` |
| `stack/max`               | `stack-bytes`  | maximum stack across all runs                    |
| `<board>/<profile>/size`  | `flash-bytes`  | application image size                           |
| `<board>/<profile>/size`  | `ram-bytes`    | internal RAM usage                               |

Everything that describes a run that isn't specifically to do with KEX (`boot`,
`bridge/rtt` and `heap/<label>`) is taken from the `curve25519-sha256` run when
present, falling back to the first run, so those values stay consistent.
`stack/max` is the maximum across all runs.

## CI Tracking

Currently, `.github/workflows/size.yml` runs `cargo xtask size` for every board
on each push and PR to `main` and uploads the JSON to Bencher. Eventually, this
will also support `cargo xtask bench` using HIL.

## Uploading results to Bencher manually

There is no automated HIL setup that uploads `cargo xtask bench` results to 
Bencher yet, so this needs to be done manually.

To do so, follow the instructions for `cargo xtask bench` and `cargo xtask bmf` to
generate the required data:

```bash
cargo xtask bench --board esp32c6-devkitc --kex curve25519-sha256 -o bench-curve25519.json
cargo xtask bench --board esp32c6-devkitc --kex mlkem768x25519-sha256 -o bench-mlkem.json
cargo xtask bmf --input bench-curve25519.json --input bench-mlkem.json -o bmf.json

git fetch origin main
bencher run \
    --project "ssh-stamp" \
    --key "$BENCHER_API_KEY" \
    --start-point main \
    --start-point-hash "$(git merge-base origin/main HEAD)" \
    --start-point-reset \
    --branch "$(git branch --show-current)" \
    --hash "$(git rev-parse HEAD)" \
    --testbed esp32c6-devkitc \
    --adapter json \
    --file bmf.json
```

The `--start-point` flags will draw a comparison against the main branch for a pull request. If benchmarking
main itself, the `--start-point` flags should be removed.

[Bencher]: https://bencher.dev
[bencher-cli]: https://bencher.dev/docs/how-to/install-cli/
[bmf]: https://bencher.dev/docs/reference/bencher-metric-format/
