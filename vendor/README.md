<!--
SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>

SPDX-License-Identifier: GPL-3.0-or-later
-->

# vendor/

Third-party crates carried in-tree because ssh-stamp needs a fix that has
no released version yet. Each is an unmodified upstream release plus a
minimal patch, kept diffable against the release it came from.

These are **not** workspace members (see `exclude` in the root
`Cargo.toml`), so `cargo fmt --all` and the workspace lints leave them
alone. They are wired in through `[patch.crates-io]`, which means every
crate in the graph gets the patched version, not just the one that asked
for it.

## embassy-net-wiznet 0.3.0

Upstream: <https://github.com/embassy-rs/embassy/tree/main/embassy-net-wiznet>
Licence: MIT OR Apache-2.0 (see `.reuse/dep5`)

Two bugs in the released 0.3.0, both reachable on the W6300-EVB-Pico2:

1. **`read_frame` trusts the chip's length header.** It computes `len - 2`
   (underflow panic under this workspace's `overflow-checks = true`) and
   slices the caller's buffer to that length (out-of-range panic either
   way). A malformed length therefore kills the firmware — and the W6300
   driver disables MAC filtering, so every frame on the wire reaches this
   code. Our version validates the header, and on a bad value discards the
   reported backlog and resynchronises rather than trying to find the next
   frame in framing it no longer trusts.

2. **`write_frame` spins forever** waiting for TX buffer space, with no
   timeout and no yield, wedging the driver task if the chip never reports
   any. Ours bounds the wait at 1s and yields between polls.

### Retiring this

Upstream fixed (1) after the 0.3.0 release in commit `1459244`, taking a
different approach: it caps the read at the buffer length and skips the
remainder, rather than resyncing. (2) is still unfixed upstream and is
worth a PR.

When a release carries both, delete this directory, drop the
`[patch.crates-io]` block and the `exclude` entry from the root
`Cargo.toml`, and bump `embassy-net-wiznet` in
`ssh-stamp-rp2350/Cargo.toml`.

A git dependency on embassy's `main` is not an option in the meantime:
patching one crate out of that monorepo resolves its sibling path deps
from git as well, which pulls in a second `embassy-time-driver`, and cargo
refuses two packages declaring the same `links = "embassy-time"` as
embassy-rp already does.
