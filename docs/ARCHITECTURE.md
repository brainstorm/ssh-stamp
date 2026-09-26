<!--
SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>

SPDX-License-Identifier: GPL-3.0-or-later
-->

# Architecture

This document has been migrated to rustdoc. The published version is on
docs.rs: start at [ssh-stamp](https://docs.rs/ssh-stamp) for the architecture
and repository layout, and [ssh-stamp-hal](https://docs.rs/ssh-stamp-hal) for
the HAL trait map and porting guide.

To read it locally instead, run:

```
cargo doc -p ssh-stamp -p ssh-stamp-hal --target riscv32imac-unknown-none-elf --no-deps
```

Then open `target/riscv32imac-unknown-none-elf/doc/ssh_stamp/index.html`.

The crate-level documentation in `ssh-stamp` covers architecture, invariants,
common tasks, the repository layout (platform-agnostic crates at the root,
per-manufacturer crates under `boards/ssh-stamp-<manufacturer>/`), and adding
new hardware ports. The HAL trait map and porting
guide live in `ssh-stamp-hal`.