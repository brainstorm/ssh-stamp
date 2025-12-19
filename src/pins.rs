use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};
use esp_hal::gpio::AnyPin;
use esp_hal::peripherals;
use static_cell::StaticCell;
use sunset::sshwire::{SSHDecode, SSHEncode, SSHSink, SSHSource, WireResult};
use sunset_async::SunsetMutex;

use crate::{
    config::{dec_option, enc_option},
    errors,
};

#[derive(Debug, Clone, PartialEq)]
pub struct SerdePinConfig {
    pub tx: u8,
    pub rx: u8,
    pub rts: Option<u8>,
    pub cts: Option<u8>,
}

impl SerdePinConfig {
    /// Create a new SerdePinConfig with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the TX pin, returning an updated SerdePinConfig (builder style).
    pub fn with_tx(mut self, tx: u8) -> Self {
        self.tx = tx;
        self
    }

    /// Set the RX pin, returning an updated SerdePinConfig (builder style).
    pub fn with_rx(mut self, rx: u8) -> Self {
        self.rx = rx;
        self
    }

    /// Set the RTS pin (optional), returning an updated SerdePinConfig.
    pub fn with_rts(mut self, rts: Option<u8>) -> Self {
        self.rts = rts;
        self
    }

    /// Set the CTS pin (optional), returning an updated SerdePinConfig.
    pub fn with_cts(mut self, cts: Option<u8>) -> Self {
        self.cts = cts;
        self
    }
}

impl Default for SerdePinConfig {
    fn default() -> Self {
        Self {
            tx: 10,
            rx: 11,
            rts: None,
            cts: None,
        }
    }
}

impl SSHEncode for SerdePinConfig {
    fn enc(&self, s: &mut dyn SSHSink) -> WireResult<()> {
        self.tx.enc(s)?;
        self.rx.enc(s)?;
        enc_option(&self.rts, s)?;
        enc_option(&self.cts, s)
    }
}

impl<'de> SSHDecode<'de> for SerdePinConfig {
    fn dec<S>(s: &mut S) -> WireResult<Self>
    where
        S: SSHSource<'de>,
    {
        // Decoding Options is problematic since encode only writes them if they exist.
        let mut pin_config = SerdePinConfig::default();
        pin_config.tx = u8::dec(s)?;
        pin_config.rx = u8::dec(s)?;

        pin_config.rts = dec_option(s)?;
        pin_config.cts = dec_option(s)?;

        Ok(pin_config)
    }
}

#[derive(Default)]
pub struct GPIOConfig {
    pub gpio10: Option<AnyPin<'static>>,
    pub gpio11: Option<AnyPin<'static>>,
}

pub struct PinChannel {
    pub config: SerdePinConfig,
    pub gpios: GPIOConfig,
    pub tx: Channel<CriticalSectionRawMutex, (), 1>,
    pub rx: Channel<CriticalSectionRawMutex, (), 1>,
    // TODO: cts/rts pins
}

impl PinChannel {
    pub fn new(config: SerdePinConfig, gpios: GPIOConfig) -> Self {
        Self {
            config,
            gpios,
            tx: Channel::<CriticalSectionRawMutex, (), 1>::new(),
            rx: Channel::<CriticalSectionRawMutex, (), 1>::new(),
        }
    }

    pub async fn recv_tx(&mut self) -> errors::Result<AnyPin<'static>> {
        // tx needs to lock here.
        //self.tx.receive().await;

        Ok(match self.config.tx {
            10 => self
                .gpios
                .gpio10
                .take()
                .ok_or_else(|| errors::Error::InvalidPin)?,
            11 => self
                .gpios
                .gpio11
                .take()
                .ok_or_else(|| errors::Error::InvalidPin)?,
            _ => return Err(errors::Error::InvalidPin),
        })
    }

    pub async fn send_tx(&mut self, pin: AnyPin<'static>) -> errors::Result<()> {
        match self.config.tx {
            10 => self.gpios.gpio10 = Some(pin),
            11 => self.gpios.gpio11 = Some(pin),
            _ => return Err(errors::Error::InvalidPin),
        };

        // tx lock needs to be released.
        self.tx.send(()).await;
        Ok(())
    }

    pub async fn recv_rx(&mut self) -> errors::Result<AnyPin<'static>> {
        let res = Ok(match self.config.rx {
            10 => self
                .gpios
                .gpio10
                .take()
                .ok_or_else(|| errors::Error::InvalidPin)?,
            11 => self
                .gpios
                .gpio11
                .take()
                .ok_or_else(|| errors::Error::InvalidPin)?,
            _ => return Err(errors::Error::InvalidPin),
        });
        // rx needs to lock here.
        // dbg!("recv_rx: before rx.receive.await");
        // self.rx.receive().await;
        // dbg!("recv_rx: after rx.receive.await");

        res
    }

    pub async fn send_rx(&mut self, pin: AnyPin<'static>) -> errors::Result<()> {
        match self.config.rx {
            10 => self.gpios.gpio10 = Some(pin),
            11 => self.gpios.gpio11 = Some(pin),
            _ => return Err(errors::Error::InvalidPin),
        };

        // rx lock needs to be released.
        self.rx.send(()).await;
        Ok(())
    }

    pub async fn with_channel<F>(&mut self, f: F) -> errors::Result<()>
    where
        F: for<'a> AsyncFnOnce(AnyPin<'a>, AnyPin<'a>),
    {
        let mut rx = self.recv_rx().await?;
        let mut tx = self.recv_tx().await?;

        f(rx.reborrow(), tx.reborrow()).await;

        self.send_rx(rx).await.unwrap();
        self.send_tx(tx).await.unwrap();

        Ok(())
    }

    // Update the runtime config's TX pin number. This only changes the
    // u8 config; actual AnyPin movement happens when the uart task next
    // reacquires pins via recv_tx/recv_rx.
    pub fn set_tx_pin(&mut self, tx: u8) -> errors::Result<()> {
        if tx == self.config.rx {
            return Err(errors::Error::InvalidPin);
        }
        match tx {
            10 | 11 => {
                self.config.tx = tx;
                Ok(())
            }
            _ => Err(errors::Error::InvalidPin),
        }
    }

    pub fn set_rx_pin(&mut self, rx: u8) -> errors::Result<()> {
        if rx == self.config.tx {
            return Err(errors::Error::InvalidPin);
        }
        match rx {
            10 | 11 => {
                self.config.rx = rx;
                Ok(())
            }
            _ => Err(errors::Error::InvalidPin),
        }
    }
}

// Global PinChannel holder: initialize from main() and access from other modules.
// We keep a StaticCell but avoid any unsafe global pointer; callers receive
// the &'static SunsetMutex returned by init_global_channel and must retain it.
static GLOBAL_PIN_CHANNEL: StaticCell<SunsetMutex<PinChannel>> = StaticCell::new();

pub fn init_global_channel(ch: PinChannel) -> &'static SunsetMutex<PinChannel> {
    // Initialize the StaticCell and return the &'static reference.
    GLOBAL_PIN_CHANNEL.init(SunsetMutex::new(ch))
}

pub struct PinConfig {
    pub tx: AnyPin<'static>,
    pub rx: AnyPin<'static>,
}

pub struct PinConfigAlt {
    pub peripherals: peripherals::Peripherals,
}

impl PinConfigAlt {
    pub fn new(peripherals: peripherals::Peripherals) -> Self {
        Self { peripherals }
    }

    pub fn take_pin<'a>(&'a mut self, pin: u8) -> AnyPin<'a> {
        match pin {
            0 => self.peripherals.GPIO0.reborrow().into(),
            1 => self.peripherals.GPIO1.reborrow().into(),
            _ => panic!(),
        }
    }
}

impl PinConfig {
    pub fn new(mut gpio_config: GPIOConfig, config_inner: SerdePinConfig) -> errors::Result<Self> {
        if config_inner.rx == config_inner.tx {
            return Err(errors::Error::InvalidPin);
        }

        // SAFETY: Safe because moved in peripherals.
        Ok(Self {
            rx: match config_inner.rx {
                10 => gpio_config.gpio10.take().unwrap().into(),
                11 => gpio_config.gpio11.take().unwrap().into(),
                _ => return Err(errors::Error::InvalidPin),
            },
            tx: match config_inner.tx {
                10 => gpio_config.gpio10.take().unwrap().into(),
                11 => gpio_config.gpio11.take().unwrap().into(),
                _ => return Err(errors::Error::InvalidPin),
            },
        })
    }

    /// Resolves a u8 pin number into an AnyPin GPIO type.
    /// Returns None if the pin number is invalid or unsupported.
    pub fn initialize_pin(
        peripherals: peripherals::Peripherals,
        pin_number: u8,
    ) -> errors::Result<AnyPin<'static>> {
        match pin_number {
            0 => Ok(peripherals.GPIO0.into()),

            _ => Err(errors::Error::InvalidPin),
        }
    }
}
