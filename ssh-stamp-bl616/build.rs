// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Replays the `BouffaloSDK` link line into this crate's firmware binary.
//!
//! Without this the binary links cleanly and is not firmware: none of the
//! vendor archives are present, the linker script is not applied, and the
//! `net_al` entry points the blobs call are pruned by `--gc-sections`. It
//! produces an ELF, which is the trap — the failure looks like success.
//!
//! `bl616-wifi-sys` is a direct dependency for this reason alone: cargo hands
//! `DEP_*` metadata only to direct dependents of the crate declaring `links`.

fn main() {
    bl616_link::emit();
}
