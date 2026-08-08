// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
// SPDX-FileCopyrightText: 2026 Julio Beltran Ortega <jubeormk1@gmail.com>
// SPDX-FileCopyrightText: 2026 Angus Gratton <gus@projectgus.com>
// SPDX-FileCopyrightText: 2026 Sergio Gasquez <sergio.gasquez@gmail.com>
// SPDX-FileCopyrightText: 2026 Gabriel Ku Wei Bin <gabriel.ku@fsfe.org>
// SPDX-FileCopyrightText: 2026 Anthony Tambasco <anthony.tambasco@fastmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! SSH connection loop orchestration.
//!
//! [`connection_loop`] processes `ServEvent` enums from the `sunset` SSH library
//! and dispatches to the appropriate handler in [`handle`](crate::handle).

use log::trace;

use crate::config::SSHStampConfig;
use crate::handle::{
    EventContext, SessionType, defunct, first_auth, hostkeys, open_session, password_auth,
    pubkey_auth, session_env, session_exec, session_pty, session_shell, session_subsystem,
};
use crate::mem_probe::{Checkpoint, checkpoint, log_kex_elapsed};
use crate::platform::PlatformServices;
use crate::settings::UART_BUFFER_SIZE;
use sunset::{ChanHandle, ServEvent};
use sunset_async::{ProgressHolder, SSHServer, SunsetMutex};

use core::result::Result;

use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;

/// Handles the SSH connection loop, processing events from clients.
///
/// # Errors
/// Returns an error if SSH protocol operations fail.
///
/// # Panics
/// Panics if flash storage lock cannot be acquired when saving configuration.
pub async fn connection_loop<P: PlatformServices>(
    serv: &SSHServer<'_>,
    chan_pipe: &Channel<NoopRawMutex, SessionType, 1>,
    config: &SunsetMutex<SSHStampConfig>,
    platform: &P,
    #[cfg(feature = "can")] can_queue: &Channel<NoopRawMutex, ChanHandle, 1>,
) -> Result<(), sunset::Error> {
    let mut session: Option<ChanHandle> = None;
    let mut config_changed = false;
    let mut needs_reset = false;
    let mut auth_checked = false;
    #[cfg(all(feature = "sftp-ota", feature = "can"))]
    let mut can_dispatched = false;

    loop {
        let mut ph = ProgressHolder::new();
        let ev = serv.progress(&mut ph).await?;

        trace!("{ev:?}");

        let mut ctx = EventContext {
            session: &mut session,
            auth_checked: &mut auth_checked,
            config_changed: &mut config_changed,
            needs_reset: &mut needs_reset,
            #[cfg(feature = "can")]
            can_queue,
            #[cfg(all(feature = "sftp-ota", feature = "can"))]
            can_dispatched: &mut can_dispatched,
        };

        match ev {
            ServEvent::SessionSubsystem(_) => {
                #[cfg(feature = "sftp-ota")]
                session_subsystem(ev, &mut ctx, chan_pipe)?;
                #[cfg(not(feature = "sftp-ota"))]
                session_subsystem(ev, &mut ctx)?;
            }
            ServEvent::SessionShell(_) => {
                session_shell(ev, &mut ctx, config, chan_pipe, platform).await?;
            }
            ServEvent::FirstAuth(_) => {
                checkpoint(Checkpoint::KexComplete);
                log_kex_elapsed("accept->firstauth");
                first_auth(ev, config).await?;
            }
            ServEvent::Hostkeys(_) => {
                hostkeys(ev, config).await?;
            }
            ServEvent::PasswordAuth(_) => {
                password_auth(ev)?;
            }
            ServEvent::PubkeyAuth(_) => {
                pubkey_auth(ev, &mut ctx, config).await?;
            }
            ServEvent::OpenSession(_) => {
                checkpoint(Checkpoint::ChannelOpen);
                open_session(ev, &mut ctx)?;
            }
            ServEvent::SessionEnv(_) => {
                session_env(ev, &mut ctx, config).await?;
            }
            ServEvent::SessionPty(_) => {
                session_pty(ev, &mut ctx, config).await?;
            }
            ServEvent::SessionExec(_) => {
                session_exec(ev)?;
            }
            ServEvent::Defunct => {
                defunct()?;
            }
            ServEvent::Authenticated => {
                checkpoint(Checkpoint::AuthSuccess);
            }
            ServEvent::PollAgain => {}
        }
    }
}

/// Creates a new [`SSHServer`] with the provided I/O buffers.
pub fn ssh_wait_for_initialisation<'server>(
    inbuf: &'server mut [u8; UART_BUFFER_SIZE],
    outbuf: &'server mut [u8; UART_BUFFER_SIZE],
) -> SSHServer<'server> {
    SSHServer::new(inbuf, outbuf)
}
