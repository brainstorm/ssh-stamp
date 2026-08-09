// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Human-readable messages from the device to the SSH client.
//!
//! # Why stderr
//!
//! A shell session on ssh-stamp is a transparent pipe to the target UART:
//! whatever the target emits arrives verbatim on the client's stdout, and
//! that property is the whole point of the bridge. Diagnostics cannot share
//! that stream — a `Bridge connected` line spliced into a firmware log would
//! corrupt anything parsing it, and there is no escape sequence a UART peer
//! could not itself produce.
//!
//! SSH already separates the two. Channel data carries an *extended data*
//! type alongside the normal stream (RFC 4254 §5.2), and type 1 is stderr.
//! Notices go there, so:
//!
//! ```text
//! ssh stamp > capture.bin     # only UART bytes, byte-for-byte
//! ssh stamp 2>/dev/null       # only UART bytes, notices discarded
//! ssh stamp                   # both, interleaved on the terminal
//! ```
//!
//! The client chooses, with no device-side flag and no in-band signalling.
//! `SSH_STAMP_NOTICES=off` exists as well for clients that merge the two
//! streams and cannot separate them after the fact.
//!
//! # Why buffered
//!
//! Most of what is worth reporting happens *before* there is a channel to
//! report it on: environment variables are processed during session setup,
//! so a `SSH_STAMP_WIFI_AP_SSID` change is already applied by the time the
//! shell opens. [`Notices`] accumulates those, and the bridge flushes them
//! once it has a stderr sink.

use core::fmt::Write as _;

use embedded_io_async::Write;
use heapless::String;

use sunset::DisconnectReason;

use crate::config::SSHStampConfig;

/// Capacity of the pending-notice buffer.
///
/// Sized for the realistic worst case of one session: a config summary plus
/// a handful of change lines. Overflow is counted and reported rather than
/// silently truncating, so this being too small is visible rather than
/// mysterious.
pub const NOTICE_BUF: usize = 1024;

/// Spare room in a [`NoticeDrain`] for the "N notice(s) dropped" line that
/// `flush_into` may append past the queued text.
pub const NOTICE_DRAIN_HEADROOM: usize = 64;

/// Scratch buffer a [`Notices`] queue is drained into before being written
/// to the channel.
pub type NoticeDrain = String<{ NOTICE_BUF + NOTICE_DRAIN_HEADROOM }>;

/// Capacity of a pre-authentication message.
///
/// Clients display these on the user's terminal, so a few lines is the
/// useful limit regardless of what the protocol permits.
pub const PREAUTH_LEN: usize = 256;

/// Text of a pre-authentication message.
pub type PreAuthText = String<PREAUTH_LEN>;

/// A message that has to reach the client before authentication finishes,
/// when no channel exists to carry it.
///
/// Queued by an event handler and sent by the connection loop, which cannot
/// send it inline: handlers run while a `ProgressHolder` holds the session
/// mutex, and sending would deadlock against it.
pub enum PreAuth {
    /// `SSH_MSG_USERAUTH_BANNER` — printed by the client before it
    /// authenticates.
    Banner(PreAuthText),
    /// A banner followed by `SSH_MSG_DISCONNECT`: the connection cannot
    /// succeed, so say why and end it.
    Refuse(DisconnectReason, PreAuthText),
}

/// Builds the pre-auth banner describing what a client is connecting to.
///
/// Returns [`PreAuth::Refuse`] when no credential could ever authenticate,
/// which is otherwise indistinguishable from a wrong key.
#[must_use]
pub fn preauth_for(config: &SSHStampConfig) -> PreAuth {
    let keys = config.pubkeys.iter().filter(|k| k.is_some()).count();
    let mut text = PreAuthText::new();

    if config.first_login {
        // Worth stating plainly: until a key is provisioned this device
        // accepts anyone within radio range.
        let _ = text.push_str(
            "ssh-stamp: first-login provisioning is OPEN - this device accepts any client.\r\n\
             ssh-stamp: claim it by sending SSH_STAMP_PUBKEY.\r\n",
        );
        return PreAuth::Banner(text);
    }

    if keys == 0 {
        // first_login closed with nothing to authenticate against: no key
        // can work, so a "permission denied" would be actively misleading.
        let _ = text.push_str(
            "ssh-stamp: no authorised keys and provisioning is closed - no login is possible.\r\n\
             ssh-stamp: erase the stored config or re-flash to provision again.\r\n",
        );
        return PreAuth::Refuse(
            DisconnectReason::SSH_DISCONNECT_NO_MORE_AUTH_METHODS_AVAILABLE,
            text,
        );
    }

    let _ = write!(
        text,
        "ssh-stamp: {keys} authorised key(s); provisioning is closed.\r\n"
    );
    PreAuth::Banner(text)
}

/// Prefix on every notice, so a line that does reach a terminal alongside
/// UART output is identifiable as coming from the device rather than the
/// target.
const PREFIX: &str = "ssh-stamp: ";

/// Messages queued for the client, plus the on/off switch for the session.
///
/// Written by the connection loop's event handlers, drained by the bridge.
/// Both run in the same task, so this lives behind a blocking mutex rather
/// than an async one — it is never held across an await.
pub struct Notices {
    buf: String<NOTICE_BUF>,
    /// Notices lost because `buf` was full.
    dropped: u16,
    enabled: bool,
}

impl Default for Notices {
    fn default() -> Self {
        Self::new()
    }
}

impl Notices {
    #[must_use]
    pub fn new() -> Self {
        Self {
            buf: String::new(),
            dropped: 0,
            enabled: true,
        }
    }

    /// Turns notices off for the rest of the session, discarding anything
    /// already queued.
    pub fn disable(&mut self) {
        self.enabled = false;
        self.buf.clear();
        self.dropped = 0;
    }

    #[must_use]
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Queues one notice. A newline is appended; callers pass the text only.
    ///
    /// Silently counts the notice as dropped if the buffer is full — a
    /// diagnostic channel must never be able to fail a session.
    pub fn push(&mut self, args: core::fmt::Arguments<'_>) {
        if !self.enabled {
            return;
        }
        // heapless' `write_fmt` can fail part-way through, leaving a partial
        // line behind. Remember where the line started so it can be undone.
        let mark = self.buf.len();
        if self
            .buf
            .write_fmt(format_args!("{PREFIX}{args}\r\n"))
            .is_err()
        {
            self.buf.truncate(mark);
            self.dropped = self.dropped.saturating_add(1);
        }
    }

    /// Moves queued notices into `out`, clearing the queue.
    ///
    /// Synchronous by design. The queue lives behind a blocking mutex, so
    /// draining it cannot be the thing that awaits — the caller writes `out`
    /// to the channel after the lock is released.
    pub fn flush_into(&mut self, out: &mut NoticeDrain) {
        out.clear();
        if !self.enabled {
            return;
        }
        let _ = out.push_str(&self.buf);
        self.buf.clear();
        if self.dropped > 0 {
            // `out` has NOTICE_DRAIN_HEADROOM spare bytes for exactly this.
            let _ = write!(out, "{PREFIX}{} notice(s) dropped\r\n", self.dropped);
            self.dropped = 0;
        }
    }
}

/// Queues a formatted notice on an [`Notices`] behind its mutex.
///
/// Takes the mutex itself rather than a guard so the borrow cannot outlive
/// the statement, which keeps callers from holding it across an await.
#[macro_export]
macro_rules! notice {
    ($notices:expr, $($arg:tt)*) => {
        $notices.lock(|n| n.borrow_mut().push(format_args!($($arg)*)))
    };
}

/// Writes one notice straight to a stderr sink, bypassing the queue.
///
/// For events that happen while the bridge is running, where there is
/// already somewhere to put them.
///
/// # Errors
/// Returns an error if the channel write fails.
pub async fn emit<W: Write>(w: &mut W, args: core::fmt::Arguments<'_>) -> Result<(), W::Error> {
    let mut line = String::<160>::new();
    if write!(line, "{PREFIX}{args}\r\n").is_err() {
        // Truncated rather than dropped: a partial warning still tells the
        // user something happened.
        line.push_str("\r\n").ok();
    }
    w.write_all(line.as_bytes()).await
}

/// Writes a summary of the running configuration.
///
/// Secrets are deliberately omitted. The client is authenticated and could
/// in principle be told, but a PSK echoed into a terminal ends up in scroll
/// buffers, screen recordings and pasted bug reports, and none of that is
/// worth the convenience of confirming a value the user just set. Whether a
/// secret *is set* is reported, since that is what troubleshooting needs.
///
/// # Errors
/// Returns an error if the channel write fails.
pub async fn config_summary<W: Write>(w: &mut W, config: &SSHStampConfig) -> Result<(), W::Error> {
    emit(w, format_args!("--- configuration ---")).await?;

    emit(
        w,
        format_args!(
            "uart: rx=GPIO{} tx=GPIO{}",
            config.uart_pins.rx, config.uart_pins.tx
        ),
    )
    .await?;

    emit(
        w,
        format_args!(
            "wifi ap: ssid={:?} psk={} band={}",
            config.wifi_ap_ssid.as_str(),
            set_or_unset(config.wifi_ap_pw.is_empty()),
            band_label(config.wifi_ap_band),
        ),
    )
    .await?;

    if config.wifi_sta_ssid.is_empty() {
        emit(w, format_args!("wifi station: not configured")).await?;
    } else {
        emit(
            w,
            format_args!(
                "wifi station: ssid={:?} psk={}",
                config.wifi_sta_ssid.as_str(),
                set_or_unset(config.wifi_sta_pw.is_empty()),
            ),
        )
        .await?;
    }

    if config.mac == [0xFF; 6] {
        emit(w, format_args!("mac: randomised each boot")).await?;
    } else {
        let m = config.mac;
        emit(
            w,
            format_args!(
                "mac: {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                m[0], m[1], m[2], m[3], m[4], m[5]
            ),
        )
        .await?;
    }

    match &config.ipv4_static {
        Some(v4) => emit(w, format_args!("ipv4: static {}", v4.address)).await?,
        None => emit(w, format_args!("ipv4: dhcp")).await?,
    }

    let keys = config.pubkeys.iter().filter(|k| k.is_some()).count();
    emit(
        w,
        format_args!(
            "authorised keys: {}/{}{}",
            keys,
            config.pubkeys.len(),
            if config.first_login {
                " (first login: unauthenticated provisioning still open)"
            } else {
                ""
            }
        ),
    )
    .await?;

    emit(w, format_args!("---------------------")).await
}

fn set_or_unset(is_empty: bool) -> &'static str {
    if is_empty { "unset" } else { "set" }
}

/// Human name for the stored band code.
#[must_use]
pub fn band_label(band: u8) -> &'static str {
    match band {
        0 => "2.4GHz",
        1 => "5GHz",
        2 => "auto",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drains `n` and returns the text the client would have received.
    fn drained(n: &mut Notices) -> NoticeDrain {
        let mut out = NoticeDrain::new();
        n.flush_into(&mut out);
        out
    }

    #[test]
    fn notices_are_prefixed_and_crlf_terminated() {
        let mut n = Notices::new();
        n.push(format_args!("bridge connected"));
        // CRLF, not LF: the client may be on a raw PTY where a bare newline
        // leaves the cursor mid-line.
        assert_eq!(drained(&mut n).as_str(), "ssh-stamp: bridge connected\r\n");
    }

    #[test]
    fn flushing_empties_the_queue() {
        let mut n = Notices::new();
        n.push(format_args!("first"));
        assert!(!drained(&mut n).is_empty());
        assert!(drained(&mut n).is_empty());
    }

    #[test]
    fn disable_discards_queued_and_suppresses_further() {
        let mut n = Notices::new();
        n.push(format_args!("before"));
        n.disable();
        n.push(format_args!("after"));
        assert!(!n.enabled());
        assert!(drained(&mut n).is_empty());
    }

    #[test]
    fn overflow_is_counted_and_leaves_no_partial_line() {
        let mut n = Notices::new();
        // Fill until a push is refused.
        let mut accepted = 0;
        while n.dropped == 0 {
            n.push(format_args!("0123456789012345678901234567890123456789"));
            if n.dropped == 0 {
                accepted += 1;
            }
        }
        assert!(accepted > 0, "buffer should hold at least one notice");

        let out = drained(&mut n);
        // Every line that made it through is intact and complete...
        let lines: heapless::Vec<&str, 64> = out.trim_end().split("\r\n").collect();
        assert_eq!(
            lines.len(),
            accepted + 1,
            "one extra line for the drop report"
        );
        for line in &lines[..accepted] {
            assert_eq!(*line, "ssh-stamp: 0123456789012345678901234567890123456789");
        }
        // ...and the loss is reported rather than hidden.
        assert_eq!(lines[accepted], "ssh-stamp: 1 notice(s) dropped");
    }

    #[test]
    fn drop_counter_resets_after_reporting() {
        let mut n = Notices::new();
        while n.dropped == 0 {
            n.push(format_args!("0123456789012345678901234567890123456789"));
        }
        let _ = drained(&mut n);
        assert_eq!(n.dropped, 0);
        assert!(drained(&mut n).is_empty());
    }

    /// A config with the given `first_login` state and `keys` authorised
    /// slots filled. The key material is irrelevant — `preauth_for` only
    /// counts occupied slots. `keys` is capped at [`KEY_SLOTS`].
    fn config_with(first_login: bool, keys: usize) -> SSHStampConfig {
        use crate::config::UartPins;
        let mut c = SSHStampConfig::new([0; 6], UartPins { rx: 17, tx: 16 })
            .expect("config generation needs an RNG");
        c.first_login = first_login;
        let signing =
            sunset::SignKey::generate(sunset::KeyType::Ed25519, None).expect("key generation");
        for slot in c.pubkeys.iter_mut().take(keys) {
            match signing.pubkey() {
                sunset::packets::PubKey::Ed25519(k) => *slot = Some(k),
                sunset::packets::PubKey::Unknown(_) => panic!("expected an ed25519 key"),
            }
        }
        c
    }

    #[test]
    fn first_login_banner_says_the_device_is_open() {
        let c = config_with(true, 0);
        match preauth_for(&c) {
            PreAuth::Banner(text) => {
                // The security-relevant half: anyone in range can connect.
                assert!(text.contains("OPEN"), "got {text:?}");
                assert!(text.contains("SSH_STAMP_PUBKEY"), "got {text:?}");
            }
            PreAuth::Refuse(..) => panic!("first login must not be refused"),
        }
    }

    #[test]
    fn provisioned_device_reports_its_key_count() {
        // The device stores KEY_SLOTS keys, currently 1; fill them all so
        // this keeps asserting the real count if that grows.
        let c = config_with(false, crate::settings::KEY_SLOTS);
        match preauth_for(&c) {
            PreAuth::Banner(text) => {
                let expected = crate::settings::KEY_SLOTS;
                assert!(
                    text.contains(&format!("{expected} authorised key(s)")),
                    "got {text:?}"
                );
            }
            PreAuth::Refuse(..) => panic!("a provisioned device must not be refused"),
        }
    }

    #[test]
    fn no_keys_and_closed_provisioning_is_refused_not_denied() {
        // Nothing can authenticate here, so "permission denied" would send
        // the user hunting for a key problem that does not exist.
        let c = config_with(false, 0);
        match preauth_for(&c) {
            PreAuth::Refuse(reason, text) => {
                assert_eq!(
                    reason,
                    DisconnectReason::SSH_DISCONNECT_NO_MORE_AUTH_METHODS_AVAILABLE
                );
                assert!(text.contains("no login is possible"), "got {text:?}");
            }
            PreAuth::Banner(_) => panic!("an unusable device must say so"),
        }
    }

    #[test]
    fn preauth_text_is_crlf_terminated() {
        // Clients print banners verbatim onto a terminal.
        for c in [
            config_with(true, 0),
            config_with(false, 1),
            config_with(false, 0),
        ] {
            let text = match preauth_for(&c) {
                PreAuth::Banner(t) | PreAuth::Refuse(_, t) => t,
            };
            assert!(text.ends_with("\r\n"), "got {text:?}");
        }
    }

    #[test]
    fn band_labels_cover_the_stored_encoding() {
        assert_eq!(band_label(0), "2.4GHz");
        assert_eq!(band_label(1), "5GHz");
        assert_eq!(band_label(2), "auto");
        assert_eq!(band_label(99), "unknown");
    }

    #[test]
    fn secrets_are_reported_as_presence_only() {
        assert_eq!(set_or_unset(true), "unset");
        assert_eq!(set_or_unset(false), "set");
    }
}
