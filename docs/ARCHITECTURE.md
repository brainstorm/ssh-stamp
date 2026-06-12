<!--
SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>

SPDX-License-Identifier: GPL-3.0-or-later
-->

# Architecture

This document has been migrated to rustdoc. Run the following to read it locally:

```
cargo doc -p ssh-stamp -p ssh-stamp-hal --target riscv32imac-unknown-none-elf --no-deps
```

Then open `target/riscv32imac-unknown-none-elf/doc/ssh_stamp/index.html`.

The crate-level documentation in `ssh-stamp` covers architecture, invariants,
common tasks, and adding new hardware ports. The HAL trait map and porting
guide live in `ssh-stamp-hal`.