// Static settings

use core::net::Ipv4Addr;

// SSH server settings
//pub(crate) const MTU: usize = 1536;
//pub(crate) const PORT: u16 = 22;
pub(crate) const DEFAULT_SSID: &str = "ssh-stamp";
//pub(crate) const SSH_SERVER_ID: &str = "SSH-2.0-ssh-stamp-0.1";
pub(crate) const KEY_SLOTS: usize = 1; // TODO: Document whether this a "reasonable default"? Justify why?
                                       //pub(crate) const PASSWORD_AUTHENTICATION: bool = true;
pub(crate) const DEFAULT_IP: &Ipv4Addr = &Ipv4Addr::new(192, 168, 4, 1);

// UART settings
//pub(crate) const BAUD_RATE: u32 = 115200;
//pub(crate) const UART_SETTINGS: &str = "8N1";
pub(crate) const DEFAULT_UART_TX_PIN: u8 = 10;
pub(crate) const DEFAULT_UART_RX_PIN: u8 = 11;
