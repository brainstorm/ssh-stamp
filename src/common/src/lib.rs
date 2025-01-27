// avoid warning about Send for the time being
//#[allow(async_fn_in_trait)]

mod server;

pub mod config;
pub mod takepipe;

pub use server::{DemoServer, ServerApp, listener};
pub use config::SSHConfig;

// needed for derive
use sunset::sshwire;