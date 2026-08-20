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

Nothing should live here without an exit plan. Each entry states what it
changes, where that change is going, and what to delete when it lands.

## embassy-net-wiznet 0.3.0

| | |
|---|---|
| Upstream | <https://github.com/embassy-rs/embassy/tree/main/embassy-net-wiznet> |
| Base | crates.io release 0.3.0, unmodified except for `src/device.rs` |
| Licence | MIT OR Apache-2.0 (see `.reuse/dep5`) |
| Status | both changes proposed upstream; see [Retiring this](#retiring-this) |

Two failure modes in the MACRAW path, both reachable on the
W6300-EVB-Pico2:

1. **`write_frame` spins forever** waiting for TX buffer space, with no
   timeout and no yield:

   ```rust
   while self.get_tx_free_size().await? < frame.len() as u16 {}
   ```

   A chip that stops reporting free space wedges the driver task
   permanently — and since `Runner::run` is the only thing servicing RX,
   TX and link state, the interface is dead rather than degraded. The
   loop also holds the `SpiDevice` throughout, so it pins the bus, and on
   a cooperative executor it starves the tasks that could have reported
   the problem. Ours bounds the wait at 1 s, yields between polls, and
   returns `Ok(0)`. The caller already tolerates that: it discards the
   result and marks the send done, so a timeout is indistinguishable from
   a frame the wire lost.

2. **`read_frame` recovers from a bad length header by trusting it.**
   Upstream stopped the panics after 0.3.0 (commit `14592448`: `raw < 2`
   underflowed, and an oversized header indexed past the caller's
   buffer). What is left is the recovery path — on a corrupt header the
   read pointer is still advanced by an offset derived from that header,
   up to ~64 KiB, so reading resumes mid-frame and every header after it
   is misaligned too.

   Ours uses the bound already in hand: `RSR`, read at the top of the
   function, is what the chip says is queued, and a header can never
   legitimately claim more. `raw < 2 || raw > rx_size` is therefore
   corruption — discard the reported backlog and resynchronise on a
   boundary the chip agrees with. Headers `RSR` corroborates keep the
   upstream truncate-and-skip behaviour, so a genuinely oversized frame
   still does not cost the frames queued behind it.

This matters more here than it looks: the W6300 init path disables the
MAC address filter (upstream found DHCP fails with it on), so every frame
on the segment reaches this code.

### Keeping it honest

Only `src/device.rs` differs. To see the full local delta against the
release:

```
diff -ru ~/.cargo/registry/src/*/embassy-net-wiznet-0.3.0/src \
         vendor/embassy-net-wiznet/src
```

The version proposed upstream carries `warn!` lines on both recovery
paths; the vendored copy takes the same branches silently, because 0.3.0
predates the crate's `fmt` module and adding a logging dependency to a
vendored crate is not worth two lines.

### Retiring this

When a release carries both changes: delete `vendor/embassy-net-wiznet/`,
drop the `[patch.crates-io]` block and the `exclude` entry from the root
`Cargo.toml`, drop the `vendor/embassy-net-wiznet/*` stanza from
`.reuse/dep5`, and bump `embassy-net-wiznet` in
`ssh-stamp-rp2350/Cargo.toml`.

A git dependency on embassy's `main` is not an option in the meantime:
patching one crate out of that monorepo resolves its sibling path deps
from git as well, which pulls in a second `embassy-time-driver`, and cargo
refuses two packages declaring the same `links = "embassy-time"` as
embassy-rp already does.
