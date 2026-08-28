#![feature(prelude_import)]
#![no_std]
#![forbid(unsafe_code)]
#![deny(clippy::mem_forget)]
#![deny(unused_imports)]
#![deny(unused_variables)]
extern crate core;
#[prelude_import]
use core::prelude::rust_2024::*;
extern crate alloc;
pub mod bench {
    //! ESP32 heap instrumentation for benchmarking.
    //!
    //! This is the Platform-specific analogue to [`ssh_stamp::mem_probe`], as it requires
    //! `esp_alloc::HEAP` to emit logs.
    //!
    /// Empty function, compiles to a no-op if `mem-probe` is not enabled.
    pub fn log_heap(_label: &str) {}
}
mod boot {
    //! The boot sequence macros which are used when initializing the device. These
    //! are separate macros so that they remain testable. Everything that consumes
    //! `Peripherals` must be a macro because fields like `TIMG1` or `SYSTIMER` are
    //! different per-chip, and cannot be resolved in a single function in the
    //! library crate.
    use embassy_executor::SendSpawner;
    use esp_hal::interrupt::{Priority, software::SoftwareInterrupt};
    use esp_rtos::embassy::InterruptExecutor;
    use static_cell::StaticCell;
    /// Starts the `InterruptExecutor` on the [`SoftwareInterrupt<1>`](SoftwareInterrupt) left over
    /// from  [`start_rtos!`](macro@crate::start_rtos), and returns its spawner.
    pub fn start_interrupt_executor(
        sw_int1: SoftwareInterrupt<'static, 1>,
    ) -> SendSpawner {
        static INT_EXECUTOR: StaticCell<InterruptExecutor<1>> = StaticCell::new();
        let interrupt_executor = INT_EXECUTOR
            .init_with(|| InterruptExecutor::new(sw_int1));
        let interrupt_spawner = interrupt_executor.start(Priority::Priority1);
        interrupt_spawner
    }
}
pub mod flash {
    //! Flash storage and OTA implementation for ESP32 family
    //!
    //! Provides access to flash storage for configuration persistence and firmware updates.
    use embedded_storage::nor_flash::NorFlash;
    use esp_bootloader_esp_idf::ota::OtaImageState;
    use esp_bootloader_esp_idf::ota_updater::OtaUpdater;
    use esp_bootloader_esp_idf::partitions::PARTITION_TABLE_MAX_LEN;
    use esp_hal::peripherals::FLASH;
    use esp_storage::FlashStorage;
    use log::{debug, error};
    use once_cell::sync::OnceCell;
    use ssh_stamp_hal::{FlashError, HalError, OtaActions};
    use sunset_async::SunsetMutex;
    const FLASH_BUF_SIZE: usize = FlashStorage::SECTOR_SIZE as usize;
    /// Flash storage singleton
    static FLASH_STORAGE: OnceCell<SunsetMutex<FlashBuffer<'static>>> = OnceCell::new();
    /// Flash buffer holding both storage and read/write buffer
    pub struct FlashBuffer<'d> {
        pub flash: FlashStorage<'d>,
        pub buf: [u8; FLASH_BUF_SIZE],
    }
    #[automatically_derived]
    impl<'d> ::core::fmt::Debug for FlashBuffer<'d> {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            ::core::fmt::Formatter::debug_struct_field2_finish(
                f,
                "FlashBuffer",
                "flash",
                &self.flash,
                "buf",
                &&self.buf,
            )
        }
    }
    impl<'d> FlashBuffer<'d> {
        #[must_use]
        pub fn new(flash: FlashStorage<'static>) -> Self {
            Self {
                flash,
                buf: [0u8; FLASH_BUF_SIZE],
            }
        }
        /// Get mutable references to both flash and buffer
        pub fn split_ref_mut(&mut self) -> (&mut FlashStorage<'d>, &mut [u8]) {
            (&mut self.flash, &mut self.buf)
        }
    }
    /// Initialize flash storage
    pub fn init(flash: FLASH<'static>) {
        let fl = FlashBuffer::new(FlashStorage::new(flash));
        let Ok(()) = FLASH_STORAGE.set(SunsetMutex::new(fl)) else {
            {
                {
                    let lvl = ::log::Level::Warn;
                    if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level() {
                        ::log::__private_api::log(
                            { ::log::__private_api::GlobalLogger },
                            format_args!("Flash storage already initialized"),
                            lvl,
                            &(
                                "ssh_stamp_esp32::flash",
                                "ssh_stamp_esp32::flash",
                                ::log::__private_api::loc(),
                            ),
                            (),
                        );
                    }
                }
            };
            return;
        };
    }
    /// Get flash storage and buffer
    pub fn get_flash_n_buffer() -> Option<&'static SunsetMutex<FlashBuffer<'static>>> {
        FLASH_STORAGE.get()
    }
    /// OTA writer for ESP32
    pub struct EspOtaWriter {}
    #[automatically_derived]
    impl ::core::fmt::Debug for EspOtaWriter {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            ::core::fmt::Formatter::write_str(f, "EspOtaWriter")
        }
    }
    #[automatically_derived]
    impl ::core::marker::Copy for EspOtaWriter {}
    #[automatically_derived]
    #[doc(hidden)]
    unsafe impl ::core::clone::TrivialClone for EspOtaWriter {}
    #[automatically_derived]
    impl ::core::clone::Clone for EspOtaWriter {
        #[inline]
        fn clone(&self) -> EspOtaWriter {
            *self
        }
    }
    impl EspOtaWriter {
        #[must_use]
        pub fn new() -> Self {
            EspOtaWriter {}
        }
        async fn next_ota_size() -> Result<u32, HalError> {
            let Some(fb) = get_flash_n_buffer() else {
                {
                    {
                        let lvl = ::log::Level::Error;
                        if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level() {
                            ::log::__private_api::log(
                                { ::log::__private_api::GlobalLogger },
                                format_args!("Flash storage not initialized"),
                                lvl,
                                &(
                                    "ssh_stamp_esp32::flash",
                                    "ssh_stamp_esp32::flash",
                                    ::log::__private_api::loc(),
                                ),
                                (),
                            );
                        }
                    }
                };
                return Err(HalError::Flash(FlashError::InternalError));
            };
            let mut fb = fb.lock().await;
            let (storage, _) = fb.split_ref_mut();
            let mut buff_ota = [0u8; PARTITION_TABLE_MAX_LEN];
            let mut ota = OtaUpdater::new(storage, &mut buff_ota)
                .map_err(|_| HalError::Flash(FlashError::InternalError))?;
            let (target_partition, _) = ota
                .next_partition()
                .map_err(|_| HalError::Flash(FlashError::InternalError))?;
            u32::try_from(target_partition.partition_size())
                .map_err(|_| HalError::Flash(FlashError::InternalError))
        }
        async fn write_to_target(offset: u32, data: &[u8]) -> Result<(), HalError> {
            let Some(fb) = get_flash_n_buffer() else {
                {
                    {
                        let lvl = ::log::Level::Error;
                        if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level() {
                            ::log::__private_api::log(
                                { ::log::__private_api::GlobalLogger },
                                format_args!("Flash storage not initialized"),
                                lvl,
                                &(
                                    "ssh_stamp_esp32::flash",
                                    "ssh_stamp_esp32::flash",
                                    ::log::__private_api::loc(),
                                ),
                                (),
                            );
                        }
                    }
                };
                return Err(HalError::Flash(FlashError::InternalError));
            };
            let mut fb = fb.lock().await;
            let (storage, _) = fb.split_ref_mut();
            let mut buff_ota = [0u8; PARTITION_TABLE_MAX_LEN];
            let mut ota = OtaUpdater::new(storage, &mut buff_ota)
                .map_err(|_| HalError::Flash(FlashError::InternalError))?;
            let (mut target_partition, part_type) = ota
                .next_partition()
                .map_err(|_| HalError::Flash(FlashError::InternalError))?;
            {
                {
                    let lvl = ::log::Level::Debug;
                    if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level() {
                        ::log::__private_api::log(
                            { ::log::__private_api::GlobalLogger },
                            format_args!("Flashing image to {0:?}", part_type),
                            lvl,
                            &(
                                "ssh_stamp_esp32::flash",
                                "ssh_stamp_esp32::flash",
                                ::log::__private_api::loc(),
                            ),
                            (),
                        );
                    }
                }
            };
            {
                {
                    let lvl = ::log::Level::Debug;
                    if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level() {
                        ::log::__private_api::log(
                            { ::log::__private_api::GlobalLogger },
                            format_args!(
                                "Writing data to target_partition at offset {0}, with len {1}",
                                offset,
                                data.len(),
                            ),
                            lvl,
                            &(
                                "ssh_stamp_esp32::flash",
                                "ssh_stamp_esp32::flash",
                                ::log::__private_api::loc(),
                            ),
                            (),
                        );
                    }
                }
            };
            NorFlash::write(&mut target_partition, offset, data)
                .map_err(|_| HalError::Flash(FlashError::Write))?;
            Ok(())
        }
        async fn activate_next_ota_slot() -> Result<(), HalError> {
            let Some(fb) = get_flash_n_buffer() else {
                {
                    {
                        let lvl = ::log::Level::Error;
                        if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level() {
                            ::log::__private_api::log(
                                { ::log::__private_api::GlobalLogger },
                                format_args!("Flash storage not initialized"),
                                lvl,
                                &(
                                    "ssh_stamp_esp32::flash",
                                    "ssh_stamp_esp32::flash",
                                    ::log::__private_api::loc(),
                                ),
                                (),
                            );
                        }
                    }
                };
                return Err(HalError::Flash(FlashError::InternalError));
            };
            let mut fb = fb.lock().await;
            let (storage, _) = fb.split_ref_mut();
            let mut buff_ota = [0u8; PARTITION_TABLE_MAX_LEN];
            let mut ota = OtaUpdater::new(storage, &mut buff_ota)
                .map_err(|_| HalError::Flash(FlashError::InternalError))?;
            ota.activate_next_partition()
                .map_err(|_| HalError::Flash(FlashError::Write))?;
            ota.set_current_ota_state(OtaImageState::New)
                .map_err(|_| HalError::Flash(FlashError::Write))?;
            Ok(())
        }
    }
    impl Default for EspOtaWriter {
        fn default() -> Self {
            Self::new()
        }
    }
    impl OtaActions for EspOtaWriter {
        async fn try_validating_current_ota_partition() -> Result<(), HalError> {
            let Some(fb) = get_flash_n_buffer() else {
                {
                    {
                        let lvl = ::log::Level::Error;
                        if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level() {
                            ::log::__private_api::log(
                                { ::log::__private_api::GlobalLogger },
                                format_args!("Flash storage not initialized"),
                                lvl,
                                &(
                                    "ssh_stamp_esp32::flash",
                                    "ssh_stamp_esp32::flash",
                                    ::log::__private_api::loc(),
                                ),
                                (),
                            );
                        }
                    }
                };
                return Err(HalError::Flash(FlashError::InternalError));
            };
            let mut fb = fb.lock().await;
            let (storage, _) = fb.split_ref_mut();
            let mut buff_ota = [0u8; PARTITION_TABLE_MAX_LEN];
            let mut ota = OtaUpdater::new(storage, &mut buff_ota)
                .map_err(|_| HalError::Flash(FlashError::InternalError))?;
            ota.selected_partition()
                .map_err(|_| HalError::Flash(FlashError::InternalError))?;
            {
                {
                    let lvl = ::log::Level::Debug;
                    if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level() {
                        ::log::__private_api::log(
                            { ::log::__private_api::GlobalLogger },
                            format_args!(
                                "current image state {0:?}",
                                ota.current_ota_state(),
                            ),
                            lvl,
                            &(
                                "ssh_stamp_esp32::flash",
                                "ssh_stamp_esp32::flash",
                                ::log::__private_api::loc(),
                            ),
                            (),
                        );
                    }
                }
            };
            let state_result = ota.current_ota_state();
            if let Ok(state) = state_result
                && (state == esp_bootloader_esp_idf::ota::OtaImageState::New
                    || state
                        == esp_bootloader_esp_idf::ota::OtaImageState::PendingVerify)
            {
                ota.set_current_ota_state(
                        esp_bootloader_esp_idf::ota::OtaImageState::Valid,
                    )
                    .map_err(|_| HalError::Flash(FlashError::Write))?;
                {
                    {
                        let lvl = ::log::Level::Debug;
                        if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level() {
                            ::log::__private_api::log(
                                { ::log::__private_api::GlobalLogger },
                                format_args!("Changed state to VALID"),
                                lvl,
                                &(
                                    "ssh_stamp_esp32::flash",
                                    "ssh_stamp_esp32::flash",
                                    ::log::__private_api::loc(),
                                ),
                                (),
                            );
                        }
                    }
                };
            }
            Ok(())
        }
        async fn get_ota_partition_size() -> Result<u32, HalError> {
            Self::next_ota_size().await
        }
        async fn write_ota_data(
            &self,
            offset: u32,
            data: &[u8],
        ) -> Result<(), HalError> {
            Self::write_to_target(offset, data).await
        }
        async fn finalize_ota_update(&mut self) -> Result<(), HalError> {
            Self::activate_next_ota_slot().await
        }
        fn reset_device(&self) -> ! {
            esp_hal::system::software_reset()
        }
    }
}
mod hash {
    //! HMAC-SHA256 implementation for ESP32 family
    //!
    //! Uses ESP32's hardware-accelerated HMAC peripheral.
    use core::future::{Future, ready};
    use hmac::{Hmac, Mac};
    use sha2::{Digest, Sha256 as Sha256Impl};
    use ssh_stamp_hal::{HashError, HashHal};
    /// ESP32 HMAC implementation
    pub struct EspHmac;
    impl HashHal for EspHmac {
        fn hmac_sha256(
            &mut self,
            key: &[u8],
            message: &[u8],
            output: &mut [u8; 32],
        ) -> impl Future<Output = Result<(), ssh_stamp_hal::HalError>> {
            ready(
                match Hmac::<Sha256Impl>::new_from_slice(key) {
                    Ok(mut mac) => {
                        mac.update(message);
                        output.copy_from_slice(&mac.finalize().into_bytes());
                        Ok(())
                    }
                    Err(_) => Err(ssh_stamp_hal::HalError::Hash(HashError::Config)),
                },
            )
        }
        fn sha256(
            &mut self,
            message: &[u8],
            output: &mut [u8; 32],
        ) -> impl Future<Output = Result<(), ssh_stamp_hal::HalError>> {
            let mut hasher = Sha256Impl::new();
            hasher.update(message);
            let result = hasher.finalize();
            output.copy_from_slice(&result);
            ready(Ok(()))
        }
    }
}
mod network {
    mod wifi {
        //! `WiFi` implementation for ESP32 family.
        //!
        //! Wraps `esp-radio` AP-mode `WiFi` behind the generic [`NetworkProviderHal`]
        //! and [`WifiHal`] traits so the app layer never names ESP-specific types.
        use core::net::Ipv4Addr;
        use core::net::SocketAddrV4;
        use alloc::string::String as AllocString;
        use edge_dhcp::io::{self, DEFAULT_SERVER_PORT};
        use edge_dhcp::server::{Server, ServerOptions};
        use edge_nal::UdpBind;
        use edge_nal_embassy::{Udp, UdpBuffers};
        use embassy_executor::Spawner;
        use embassy_net::DhcpConfig;
        use embassy_net::tcp::TcpSocket;
        use embassy_net::{
            IpListenEndpoint, Ipv4Cidr, Runner, Stack, StackResources, StaticConfigV4,
        };
        use embassy_time::{Duration, Timer};
        use esp_hal::peripherals::WIFI;
        use esp_hal::rng::Rng;
        use esp_radio::wifi::{
            AuthenticationMethod, BandMode as RadioBandMode, Config as RadioConfig,
            ControllerConfig, Interface, WifiController, ap::AccessPointConfig,
            ap::EventInfo, sta::StationConfig,
        };
        use log::info;
        use log::{debug, error, warn};
        use ssh_stamp::settings::STATION_MODE_MAX_RETRY_SECONDS;
        use ssh_stamp_hal::{
            BandMode, HalError, NetworkProviderHal, WifiApConfigStatic, WifiError,
            WifiHal,
        };
        use static_cell::StaticCell;
        extern crate alloc;
        /// Handle for bringing up ESP32-family `WiFi` as an access point.
        ///
        /// Construct with [`EspWifi::new`] once all ESP peripherals are available,
        /// call [`WifiHal::configure_ap`] with the desired SSID/PSK/MAC, then call
        /// [`NetworkProviderHal::bring_up`] to start the radio, spawn the driver
        /// tasks, and return a ready [`embassy_net::Stack`].
        pub struct EspWifi {
            spawner: Spawner,
            wifi_peri: Option<WIFI<'static>>,
            rng: Rng,
            ap_config: Option<WifiApConfigStatic>,
            gateway: Ipv4Addr,
        }
        impl EspWifi {
            /// Create a new uninitialised ESP32 `WiFi` handle.
            ///
            /// `gateway` is the static IPv4 address the device will serve as the
            /// access-point gateway (and DHCP server).
            #[must_use]
            pub fn new(
                spawner: Spawner,
                wifi_peri: WIFI<'static>,
                rng: Rng,
                gateway: Ipv4Addr,
            ) -> Self {
                Self {
                    spawner,
                    wifi_peri: Some(wifi_peri),
                    rng,
                    ap_config: None,
                    gateway,
                }
            }
        }
        impl WifiHal for EspWifi {
            fn configure_ap(
                &mut self,
                config: WifiApConfigStatic,
            ) -> Result<(), HalError> {
                self.ap_config = Some(config);
                Ok(())
            }
        }
        impl NetworkProviderHal for EspWifi {
            async fn bring_up(&mut self) -> Result<Stack<'static>, HalError> {
                static RESOURCES_CELL: StaticCell<StackResources<3>> = StaticCell::new();
                static STA_SSID_CELL: StaticCell<heapless::String<32>> = StaticCell::new();
                let ap_config = self
                    .ap_config
                    .clone()
                    .ok_or(HalError::Wifi(WifiError::Initialization))?;
                let wifi_peri = self
                    .wifi_peri
                    .take()
                    .ok_or(HalError::Wifi(WifiError::Initialization))?;
                esp_hal::efuse::override_mac_address(
                        esp_hal::efuse::MacAddress::new_eui48(ap_config.mac),
                    )
                    .map_err(|_| HalError::Wifi(WifiError::Initialization))?;
                let sta_ssid_static: &'static str = STA_SSID_CELL
                    .init(ap_config.sta_ssid.clone())
                    .as_str();
                let (ap_radio_config, net_config, wifi_interface) = build_radio_config(
                    &ap_config,
                    sta_ssid_static,
                    self.gateway,
                );
                let controller_config = ControllerConfig::default()
                    .with_initial_config(ap_radio_config)
                    .with_static_rx_buf_num(4)
                    .with_dynamic_rx_buf_num(16)
                    .with_dynamic_tx_buf_num(16)
                    .with_ampdu_rx_enable(false)
                    .with_ampdu_tx_enable(false);
                let mut wifi_controller = WifiController::new(
                        wifi_peri,
                        controller_config,
                    )
                    .map_err(|_| HalError::Wifi(WifiError::Initialization))?;
                if sta_ssid_static.is_empty() {
                    set_band_mode(&mut wifi_controller, ap_config.band);
                }
                let seed = u64::from(self.rng.random()) << 32
                    | u64::from(self.rng.random());
                let (ap_stack, runner) = embassy_net::new(
                    wifi_interface,
                    net_config,
                    RESOURCES_CELL.init(StackResources::<3>::new()),
                    seed,
                );
                self.spawner
                    .spawn(
                        wifi_up(wifi_controller, sta_ssid_static)
                            .map_err(|_| HalError::Wifi(WifiError::Initialization))?,
                    );
                self.spawner
                    .spawn(
                        net_up(runner)
                            .map_err(|_| HalError::Wifi(WifiError::Initialization))?,
                    );
                if sta_ssid_static.is_empty() {
                    self.spawner
                        .spawn(
                            dhcp_server(ap_stack, self.gateway)
                                .map_err(|_| HalError::Wifi(WifiError::Initialization))?,
                        );
                    loop {
                        {
                            {
                                let lvl = ::log::Level::Debug;
                                if lvl <= ::log::STATIC_MAX_LEVEL
                                    && lvl <= ::log::max_level()
                                {
                                    ::log::__private_api::log(
                                        { ::log::__private_api::GlobalLogger },
                                        format_args!("Checking if link is up"),
                                        lvl,
                                        &(
                                            "ssh_stamp_esp32::network::wifi",
                                            "ssh_stamp_esp32::network::wifi",
                                            ::log::__private_api::loc(),
                                        ),
                                        (),
                                    );
                                }
                            }
                        };
                        if ap_stack.is_link_up() {
                            if let Some(config) = ap_stack.config_v4() {
                                {
                                    {
                                        let lvl = ::log::Level::Info;
                                        if lvl <= ::log::STATIC_MAX_LEVEL
                                            && lvl <= ::log::max_level()
                                        {
                                            ::log::__private_api::log(
                                                { ::log::__private_api::GlobalLogger },
                                                format_args!(
                                                    "Connect to the AP `{0}` with IP {1}",
                                                    ap_config.ap_ssid.as_str(),
                                                    config.address,
                                                ),
                                                lvl,
                                                &(
                                                    "ssh_stamp_esp32::network::wifi",
                                                    "ssh_stamp_esp32::network::wifi",
                                                    ::log::__private_api::loc(),
                                                ),
                                                (),
                                            );
                                        }
                                    }
                                };
                            }
                            break;
                        }
                        Timer::after(Duration::from_millis(500)).await;
                    }
                } else {
                    let mut retry_count = 0;
                    loop {
                        {
                            {
                                let lvl = ::log::Level::Debug;
                                if lvl <= ::log::STATIC_MAX_LEVEL
                                    && lvl <= ::log::max_level()
                                {
                                    ::log::__private_api::log(
                                        { ::log::__private_api::GlobalLogger },
                                        format_args!("Checking if station has received IP address"),
                                        lvl,
                                        &(
                                            "ssh_stamp_esp32::network::wifi",
                                            "ssh_stamp_esp32::network::wifi",
                                            ::log::__private_api::loc(),
                                        ),
                                        (),
                                    );
                                }
                            }
                        };
                        if ap_stack.is_config_up() {
                            if let Some(config) = ap_stack.config_v4() {
                                {
                                    {
                                        let lvl = ::log::Level::Info;
                                        if lvl <= ::log::STATIC_MAX_LEVEL
                                            && lvl <= ::log::max_level()
                                        {
                                            ::log::__private_api::log(
                                                { ::log::__private_api::GlobalLogger },
                                                format_args!(
                                                    "Connect to the AP `{0}` with IP {1}",
                                                    sta_ssid_static,
                                                    config.address,
                                                ),
                                                lvl,
                                                &(
                                                    "ssh_stamp_esp32::network::wifi",
                                                    "ssh_stamp_esp32::network::wifi",
                                                    ::log::__private_api::loc(),
                                                ),
                                                (),
                                            );
                                        }
                                    }
                                };
                            }
                            break;
                        }
                        retry_count += 1;
                        if retry_count > STATION_MODE_MAX_RETRY_SECONDS {
                            return Err(HalError::Wifi(WifiError::StationMode));
                        }
                        Timer::after(Duration::from_millis(1000)).await;
                    }
                }
                Ok(ap_stack)
            }
        }
        /// Build the esp-radio config, embassy-net config, and interface for AP or
        /// Station mode based on whether a Station SSID is configured.
        fn build_radio_config(
            ap_config: &WifiApConfigStatic,
            sta_ssid: &str,
            gateway: Ipv4Addr,
        ) -> (RadioConfig, embassy_net::Config, Interface) {
            if sta_ssid.is_empty() {
                {
                    {
                        let lvl = ::log::Level::Info;
                        if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level() {
                            ::log::__private_api::log(
                                { ::log::__private_api::GlobalLogger },
                                format_args!("Wifi configuring Access Point Mode"),
                                lvl,
                                &(
                                    "ssh_stamp_esp32::network::wifi",
                                    "ssh_stamp_esp32::network::wifi",
                                    ::log::__private_api::loc(),
                                ),
                                (),
                            );
                        }
                    }
                };
                let password = AllocString::from(ap_config.ap_password.as_str());
                let radio = RadioConfig::AccessPoint(
                    AccessPointConfig::default()
                        .with_ssid(AllocString::from(ap_config.ap_ssid.as_str()))
                        .with_auth_method(AuthenticationMethod::Wpa2Wpa3Personal)
                        .with_password(password)
                        .with_channel(ap_config.channel),
                );
                let net = embassy_net::Config::ipv4_static(StaticConfigV4 {
                    address: Ipv4Cidr::new(gateway, 24),
                    gateway: Some(gateway),
                    dns_servers: Default::default(),
                });
                (radio, net, Interface::access_point())
            } else {
                {
                    {
                        let lvl = ::log::Level::Info;
                        if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level() {
                            ::log::__private_api::log(
                                { ::log::__private_api::GlobalLogger },
                                format_args!("Wifi configuring Station Mode"),
                                lvl,
                                &(
                                    "ssh_stamp_esp32::network::wifi",
                                    "ssh_stamp_esp32::network::wifi",
                                    ::log::__private_api::loc(),
                                ),
                                (),
                            );
                        }
                    }
                };
                let password = AllocString::from(ap_config.sta_password.as_str());
                let radio = RadioConfig::Station(
                    StationConfig::default()
                        .with_ssid(AllocString::from(ap_config.sta_ssid.as_str()))
                        .with_password(password),
                );
                let net = embassy_net::Config::dhcpv4(DhcpConfig::default());
                (radio, net, Interface::station())
            }
        }
        /// Set the `WiFi` band mode on the controller. Only the ESP32-C5 supports 5GHz;
        /// on other chips `set_band_mode` returns an error that is logged and ignored.
        fn set_band_mode(wifi_controller: &mut WifiController<'static>, band: BandMode) {
            let radio_band = match band {
                BandMode::Band2_4G => RadioBandMode::_2_4G,
                _ => RadioBandMode::_2_4G,
            };
            match wifi_controller.set_band_mode(radio_band.clone()) {
                Ok(()) => {
                    let lvl = ::log::Level::Debug;
                    if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level() {
                        ::log::__private_api::log(
                            { ::log::__private_api::GlobalLogger },
                            format_args!("Set WiFi band mode: {0:?}", radio_band),
                            lvl,
                            &(
                                "ssh_stamp_esp32::network::wifi",
                                "ssh_stamp_esp32::network::wifi",
                                ::log::__private_api::loc(),
                            ),
                            (),
                        );
                    }
                }
                Err(e) => {
                    let lvl = ::log::Level::Warn;
                    if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level() {
                        ::log::__private_api::log(
                            { ::log::__private_api::GlobalLogger },
                            format_args!(
                                "Failed to set band mode {0:?}: {1:?} (non-5G chip?)",
                                radio_band,
                                e,
                            ),
                            lvl,
                            &(
                                "ssh_stamp_esp32::network::wifi",
                                "ssh_stamp_esp32::network::wifi",
                                ::log::__private_api::loc(),
                            ),
                            (),
                        );
                    }
                }
            }
        }
        /// Accept an incoming TCP connection on port 22.
        /// Returns a connected `TcpSocket` ready for SSH processing.
        ///
        /// # Errors
        /// Returns an error if the socket cannot be accepted.
        /// Note that this function will block until a connection is accepted, and will
        /// only return an error if there is a failure in the underlying socket operations.
        pub async fn accept_requests<'a>(
            tcp_stack: Stack<'a>,
            rx_buffer: &'a mut [u8],
            tx_buffer: &'a mut [u8],
        ) -> Result<TcpSocket<'a>, HalError> {
            let mut tcp_socket = TcpSocket::new(tcp_stack, rx_buffer, tx_buffer);
            {
                {
                    let lvl = ::log::Level::Debug;
                    if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level() {
                        ::log::__private_api::log(
                            { ::log::__private_api::GlobalLogger },
                            format_args!("Waiting for SSH client..."),
                            lvl,
                            &(
                                "ssh_stamp_esp32::network::wifi",
                                "ssh_stamp_esp32::network::wifi",
                                ::log::__private_api::loc(),
                            ),
                            (),
                        );
                    }
                }
            };
            if let Err(_e) = tcp_socket
                .accept(IpListenEndpoint {
                    addr: None,
                    port: 22,
                })
                .await
            {
                {
                    {
                        let lvl = ::log::Level::Error;
                        if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level() {
                            ::log::__private_api::log(
                                { ::log::__private_api::GlobalLogger },
                                format_args!("Failed to accept incoming TCP connection"),
                                lvl,
                                &(
                                    "ssh_stamp_esp32::network::wifi",
                                    "ssh_stamp_esp32::network::wifi",
                                    ::log::__private_api::loc(),
                                ),
                                (),
                            );
                        }
                    }
                };
                return Err(HalError::Wifi(WifiError::SocketAccept));
            }
            {
                {
                    let lvl = ::log::Level::Debug;
                    if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level() {
                        ::log::__private_api::log(
                            { ::log::__private_api::GlobalLogger },
                            format_args!("Connected, port 22"),
                            lvl,
                            &(
                                "ssh_stamp_esp32::network::wifi",
                                "ssh_stamp_esp32::network::wifi",
                                ::log::__private_api::loc(),
                            ),
                            (),
                        );
                    }
                }
            };
            Ok(tcp_socket)
        }
        #[doc(hidden)]
        pub fn __wifi_up_task(
            wifi_controller: WifiController<'static>,
            sta_ssid: &'static str,
        ) -> impl ::core::future::Future<Output = ()> {
            /// Manages the `WiFi` access point lifecycle.
            async fn __wifi_up_task_inner_function(
                mut wifi_controller: WifiController<'static>,
                sta_ssid: &'static str,
            ) {
                if sta_ssid.is_empty() {
                    {
                        {
                            let lvl = ::log::Level::Debug;
                            if lvl <= ::log::STATIC_MAX_LEVEL
                                && lvl <= ::log::max_level()
                            {
                                ::log::__private_api::log(
                                    { ::log::__private_api::GlobalLogger },
                                    format_args!("Wifi AP starting..."),
                                    lvl,
                                    &(
                                        "ssh_stamp_esp32::network::wifi",
                                        "ssh_stamp_esp32::network::wifi",
                                        ::log::__private_api::loc(),
                                    ),
                                    (),
                                );
                            }
                        }
                    };
                    loop {
                        let ev = wifi_controller
                            .wait_for_access_point_connected_event_async()
                            .await;
                        match ev {
                            Ok(EventInfo::Connected(info)) => {
                                {
                                    {
                                        let lvl = ::log::Level::Info;
                                        if lvl <= ::log::STATIC_MAX_LEVEL
                                            && lvl <= ::log::max_level()
                                        {
                                            ::log::__private_api::log(
                                                { ::log::__private_api::GlobalLogger },
                                                format_args!("Station connected: {0:?}", info),
                                                lvl,
                                                &(
                                                    "ssh_stamp_esp32::network::wifi",
                                                    "ssh_stamp_esp32::network::wifi",
                                                    ::log::__private_api::loc(),
                                                ),
                                                (),
                                            );
                                        }
                                    }
                                };
                            }
                            Ok(EventInfo::Disconnected(info)) => {
                                {
                                    {
                                        let lvl = ::log::Level::Info;
                                        if lvl <= ::log::STATIC_MAX_LEVEL
                                            && lvl <= ::log::max_level()
                                        {
                                            ::log::__private_api::log(
                                                { ::log::__private_api::GlobalLogger },
                                                format_args!("Station disconnected: {0:?}", info),
                                                lvl,
                                                &(
                                                    "ssh_stamp_esp32::network::wifi",
                                                    "ssh_stamp_esp32::network::wifi",
                                                    ::log::__private_api::loc(),
                                                ),
                                                (),
                                            );
                                        }
                                    }
                                };
                            }
                            _ => {}
                        }
                        Timer::after(Duration::from_millis(5000)).await;
                    }
                } else {
                    loop {
                        {
                            {
                                let lvl = ::log::Level::Debug;
                                if lvl <= ::log::STATIC_MAX_LEVEL
                                    && lvl <= ::log::max_level()
                                {
                                    ::log::__private_api::log(
                                        { ::log::__private_api::GlobalLogger },
                                        format_args!("Connecting to access point..."),
                                        lvl,
                                        &(
                                            "ssh_stamp_esp32::network::wifi",
                                            "ssh_stamp_esp32::network::wifi",
                                            ::log::__private_api::loc(),
                                        ),
                                        (),
                                    );
                                }
                            }
                        };
                        match wifi_controller.connect_async().await {
                            Ok(info) => {
                                {
                                    {
                                        let lvl = ::log::Level::Info;
                                        if lvl <= ::log::STATIC_MAX_LEVEL
                                            && lvl <= ::log::max_level()
                                        {
                                            ::log::__private_api::log(
                                                { ::log::__private_api::GlobalLogger },
                                                format_args!("Wifi connected to {0:?}", info),
                                                lvl,
                                                &(
                                                    "ssh_stamp_esp32::network::wifi",
                                                    "ssh_stamp_esp32::network::wifi",
                                                    ::log::__private_api::loc(),
                                                ),
                                                (),
                                            );
                                        }
                                    }
                                };
                                let info = wifi_controller
                                    .wait_for_disconnect_async()
                                    .await
                                    .ok();
                                {
                                    {
                                        let lvl = ::log::Level::Info;
                                        if lvl <= ::log::STATIC_MAX_LEVEL
                                            && lvl <= ::log::max_level()
                                        {
                                            ::log::__private_api::log(
                                                { ::log::__private_api::GlobalLogger },
                                                format_args!("Disconnected: {0:?}", info),
                                                lvl,
                                                &(
                                                    "ssh_stamp_esp32::network::wifi",
                                                    "ssh_stamp_esp32::network::wifi",
                                                    ::log::__private_api::loc(),
                                                ),
                                                (),
                                            );
                                        }
                                    }
                                };
                            }
                            Err(e) => {
                                {
                                    {
                                        let lvl = ::log::Level::Info;
                                        if lvl <= ::log::STATIC_MAX_LEVEL
                                            && lvl <= ::log::max_level()
                                        {
                                            ::log::__private_api::log(
                                                { ::log::__private_api::GlobalLogger },
                                                format_args!("Failed to connect to wifi: {0:?}", e),
                                                lvl,
                                                &(
                                                    "ssh_stamp_esp32::network::wifi",
                                                    "ssh_stamp_esp32::network::wifi",
                                                    ::log::__private_api::loc(),
                                                ),
                                                (),
                                            );
                                        }
                                    }
                                };
                            }
                        }
                        Timer::after(Duration::from_millis(1000)).await;
                    }
                }
            }
            { __wifi_up_task_inner_function(wifi_controller, sta_ssid) }
        }
        /// Manages the `WiFi` access point lifecycle.
        pub fn wifi_up(
            wifi_controller: WifiController<'static>,
            sta_ssid: &'static str,
        ) -> ::core::result::Result<
            ::embassy_executor::SpawnToken<impl Sized>,
            ::embassy_executor::SpawnError,
        > {
            const fn __task_pool_get<F, Args, Fut>(
                _: F,
            ) -> &'static ::embassy_executor::raw::TaskPool<Fut, POOL_SIZE>
            where
                F: ::embassy_executor::_export::TaskFn<Args, Fut = Fut>,
                Fut: ::core::future::Future + 'static,
            {
                unsafe { &*POOL.get().cast() }
            }
            const POOL_SIZE: usize = 1;
            static POOL: ::embassy_executor::_export::TaskPoolHolder<
                {
                    ::embassy_executor::_export::task_pool_size::<
                        _,
                        _,
                        _,
                        POOL_SIZE,
                    >(__wifi_up_task)
                },
                {
                    ::embassy_executor::_export::task_pool_align::<
                        _,
                        _,
                        _,
                        POOL_SIZE,
                    >(__wifi_up_task)
                },
            > = unsafe {
                ::core::mem::transmute(
                    ::embassy_executor::_export::task_pool_new::<
                        _,
                        _,
                        _,
                        POOL_SIZE,
                    >(__wifi_up_task),
                )
            };
            unsafe {
                __task_pool_get(__wifi_up_task)
                    ._spawn_async_fn(move || __wifi_up_task(wifi_controller, sta_ssid))
            }
        }
        #[doc(hidden)]
        pub fn __net_up_task(
            runner: Runner<'static, Interface>,
        ) -> impl ::core::future::Future<Output = ()> {
            /// Network task for Embassy executor.
            async fn __net_up_task_inner_function(
                mut runner: Runner<'static, Interface>,
            ) {
                {
                    {
                        let lvl = ::log::Level::Debug;
                        if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level() {
                            ::log::__private_api::log(
                                { ::log::__private_api::GlobalLogger },
                                format_args!("Bringing up network stack..."),
                                lvl,
                                &(
                                    "ssh_stamp_esp32::network::wifi",
                                    "ssh_stamp_esp32::network::wifi",
                                    ::log::__private_api::loc(),
                                ),
                                (),
                            );
                        }
                    }
                };
                runner.run().await;
            }
            { __net_up_task_inner_function(runner) }
        }
        /// Network task for Embassy executor.
        pub fn net_up(
            runner: Runner<'static, Interface>,
        ) -> ::core::result::Result<
            ::embassy_executor::SpawnToken<impl Sized>,
            ::embassy_executor::SpawnError,
        > {
            const fn __task_pool_get<F, Args, Fut>(
                _: F,
            ) -> &'static ::embassy_executor::raw::TaskPool<Fut, POOL_SIZE>
            where
                F: ::embassy_executor::_export::TaskFn<Args, Fut = Fut>,
                Fut: ::core::future::Future + 'static,
            {
                unsafe { &*POOL.get().cast() }
            }
            const POOL_SIZE: usize = 1;
            static POOL: ::embassy_executor::_export::TaskPoolHolder<
                {
                    ::embassy_executor::_export::task_pool_size::<
                        _,
                        _,
                        _,
                        POOL_SIZE,
                    >(__net_up_task)
                },
                {
                    ::embassy_executor::_export::task_pool_align::<
                        _,
                        _,
                        _,
                        POOL_SIZE,
                    >(__net_up_task)
                },
            > = unsafe {
                ::core::mem::transmute(
                    ::embassy_executor::_export::task_pool_new::<
                        _,
                        _,
                        _,
                        POOL_SIZE,
                    >(__net_up_task),
                )
            };
            unsafe {
                __task_pool_get(__net_up_task)
                    ._spawn_async_fn(move || __net_up_task(runner))
            }
        }
        #[doc(hidden)]
        pub fn __dhcp_server_task(
            stack: Stack<'static>,
            ip: Ipv4Addr,
        ) -> impl ::core::future::Future<Output = ()> {
            /// DHCP server task for Embassy executor.
            async fn __dhcp_server_task_inner_function(
                stack: Stack<'static>,
                ip: Ipv4Addr,
            ) {
                let mut buf = [0u8; 1500];
                let mut gw_buf = [Ipv4Addr::UNSPECIFIED];
                let buffers = UdpBuffers::<3, 1024, 1024, 10>::new();
                let unbound_socket = Udp::new(stack, &buffers);
                let mut bound_socket = match unbound_socket
                    .bind(
                        core::net::SocketAddr::V4(
                            SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, DEFAULT_SERVER_PORT),
                        ),
                    )
                    .await
                {
                    Ok(socket) => socket,
                    Err(e) => {
                        {
                            {
                                let lvl = ::log::Level::Warn;
                                if lvl <= ::log::STATIC_MAX_LEVEL
                                    && lvl <= ::log::max_level()
                                {
                                    ::log::__private_api::log(
                                        { ::log::__private_api::GlobalLogger },
                                        format_args!("Failed to bind DHCP server socket: {0:?}", e),
                                        lvl,
                                        &(
                                            "ssh_stamp_esp32::network::wifi",
                                            "ssh_stamp_esp32::network::wifi",
                                            ::log::__private_api::loc(),
                                        ),
                                        (),
                                    );
                                }
                            }
                        };
                        return;
                    }
                };
                loop {
                    if let Err(e) = io::server::run(
                            &mut Server::<_, 64>::new_with_et(ip),
                            &ServerOptions::new(ip, Some(&mut gw_buf)),
                            &mut bound_socket,
                            &mut buf,
                        )
                        .await
                    {
                        {
                            {
                                let lvl = ::log::Level::Error;
                                if lvl <= ::log::STATIC_MAX_LEVEL
                                    && lvl <= ::log::max_level()
                                {
                                    ::log::__private_api::log(
                                        { ::log::__private_api::GlobalLogger },
                                        format_args!("DHCP server error: {0:?}", e),
                                        lvl,
                                        &(
                                            "ssh_stamp_esp32::network::wifi",
                                            "ssh_stamp_esp32::network::wifi",
                                            ::log::__private_api::loc(),
                                        ),
                                        (),
                                    );
                                }
                            }
                        };
                    }
                    Timer::after(Duration::from_millis(500)).await;
                }
            }
            { __dhcp_server_task_inner_function(stack, ip) }
        }
        /// DHCP server task for Embassy executor.
        pub fn dhcp_server(
            stack: Stack<'static>,
            ip: Ipv4Addr,
        ) -> ::core::result::Result<
            ::embassy_executor::SpawnToken<impl Sized>,
            ::embassy_executor::SpawnError,
        > {
            const fn __task_pool_get<F, Args, Fut>(
                _: F,
            ) -> &'static ::embassy_executor::raw::TaskPool<Fut, POOL_SIZE>
            where
                F: ::embassy_executor::_export::TaskFn<Args, Fut = Fut>,
                Fut: ::core::future::Future + 'static,
            {
                unsafe { &*POOL.get().cast() }
            }
            const POOL_SIZE: usize = 1;
            static POOL: ::embassy_executor::_export::TaskPoolHolder<
                {
                    ::embassy_executor::_export::task_pool_size::<
                        _,
                        _,
                        _,
                        POOL_SIZE,
                    >(__dhcp_server_task)
                },
                {
                    ::embassy_executor::_export::task_pool_align::<
                        _,
                        _,
                        _,
                        POOL_SIZE,
                    >(__dhcp_server_task)
                },
            > = unsafe {
                ::core::mem::transmute(
                    ::embassy_executor::_export::task_pool_new::<
                        _,
                        _,
                        _,
                        POOL_SIZE,
                    >(__dhcp_server_task),
                )
            };
            unsafe {
                __task_pool_get(__dhcp_server_task)
                    ._spawn_async_fn(move || __dhcp_server_task(stack, ip))
            }
        }
    }
    pub use wifi::{EspWifi, accept_requests, dhcp_server, net_up, wifi_up};
}
mod platform {
    //! ESP32 implementation of [`PlatformServices`].
    //!
    //! Wires the app layer's persistence, reset, OTA, and UART-activation hooks
    //! through to ESP-specific helpers (`flash::*`, `esp_hal::system`, the
    //! `UART_SIGNAL`).
    use ssh_stamp::config::SSHStampConfig;
    use ssh_stamp::platform::PlatformServices;
    use ssh_stamp::store;
    use ssh_stamp_hal::{FlashError, HalError};
    use crate::EspOtaWriter;
    use crate::flash;
    use crate::uart::UART_SIGNAL;
    /// Handle through which the app layer reaches ESP-only services.
    ///
    /// Construct once on the embassy executor and pass `&EspPlatform` to
    /// [`ssh_stamp::app::run_app`] / [`ssh_stamp::app::prepare_ap_config`].
    pub struct EspPlatform {}
    impl EspPlatform {
        #[must_use]
        pub fn new() -> Self {
            Self {}
        }
    }
    impl Default for EspPlatform {
        fn default() -> Self {
            Self::new()
        }
    }
    impl PlatformServices for EspPlatform {
        type OtaWriter = EspOtaWriter;
        async fn save_config(&self, config: &SSHStampConfig) -> Result<(), HalError> {
            let Some(flash_guard) = flash::get_flash_n_buffer() else {
                return Err(HalError::Flash(FlashError::InternalError));
            };
            let mut fb = flash_guard.lock().await;
            let (flash, buf) = fb.split_ref_mut();
            store::save(flash, buf, config)
                .map_err(|_| HalError::Flash(FlashError::Write))
        }
        fn reset(&self) -> ! {
            esp_hal::system::software_reset()
        }
        fn ota_writer(&self) -> Self::OtaWriter {
            EspOtaWriter::new()
        }
        fn activate_uart(&self) {
            UART_SIGNAL.signal(1);
        }
    }
}
mod rng {
    //! RNG implementation for ESP32 family
    //!
    //! Provides hardware random number generation using ESP32's true RNG.
    //!
    //! # Wiring this into `getrandom`
    //!
    //! getrandom 0.4 no longer selects its backend with a cargo feature and no
    //! longer offers `register_custom_getrandom!`. Instead the `custom` backend
    //! is chosen per target with `--cfg getrandom_backend="custom"` (set in
    //! `.cargo/config.toml` for every bare-metal target here), and getrandom
    //! links an `extern "Rust"` symbol that must be defined exactly once in the
    //! whole program.
    //!
    //! Defining that symbol requires `unsafe`, which this crate forbids, and
    //! binaries cannot link each other's definitions. So, the
    //! [`getrandom_backend!`](macro@crate::getrandom_backend) packages the
    //! definition as a macro that every binary invokes once. The `unsafe`
    //! only compiles where the macro is expanded, keeping this crate
    //! `#![forbid(unsafe_code)]`.
    use core::cell::RefCell;
    use core::future::{Future, ready};
    use embassy_sync::blocking_mutex::Mutex;
    use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
    use esp_hal::rng::Rng;
    use ssh_stamp_hal::{HalError, RngHal};
    use static_cell::StaticCell;
    static RNG: StaticCell<Rng> = StaticCell::new();
    static RNG_MUTEX: Mutex<
        CriticalSectionRawMutex,
        RefCell<Option<&'static mut Rng>>,
    > = Mutex::new(RefCell::new(None));
    /// Register the hardware RNG for use with getrandom
    pub fn register_custom_rng(rng: Rng) {
        let rng_ref = RNG.init(rng);
        RNG_MUTEX.lock(|t| t.borrow_mut().replace(rng_ref));
    }
    /// This is a wrapper that sets up the boot entropy source.
    ///
    /// On chips with a TRNG this holds the SAR ADC entropy source that
    /// [`init_entropy()`] enables. The RNG register only has randomness
    /// while a source is active, and the ADC source is what occurs on boot.
    ///
    /// Dropping this guard switches off the source. This handover is required
    /// because the ADC source needs to be switched off "before RF subsystem
    /// features, ADC, or I2S (ESP32 only) are initialized" and that it's "not
    /// safe to use if any other subsystem is accessing the RF subsystem or
    /// the ADC at the same time".
    ///
    /// On the ESP32-C5/C61, there is no TRNG driver yet, so the guard is empty.
    ///
    /// See: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/random.html>
    #[must_use = "dropping switches the boot-time entropy source off"]
    pub struct EntropySource {}
    /// Registers the hardware RNG with `getrandom`.
    ///
    /// The ESP32-C5/C61 have no TRNG in esp-hal yet, so the RNG register
    /// is all the firmware has at boot. This means that anything using
    /// this before the radio is up is only as good as that register.
    ///
    /// Call through [`init_entropy!`](macro@crate::init_entropy), which moves =
    /// the peripherals needed.
    pub fn init_entropy() -> (Rng, EntropySource) {
        {
            {
                let lvl = ::log::Level::Warn;
                if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level() {
                    ::log::__private_api::log(
                        { ::log::__private_api::GlobalLogger },
                        format_args!(
                            "No TRNG on this chip, RNG is not cryptographically secure until the radio is up",
                        ),
                        lvl,
                        &(
                            "ssh_stamp_esp32::rng",
                            "ssh_stamp_esp32::rng",
                            ::log::__private_api::loc(),
                        ),
                        (),
                    );
                }
            }
        };
        let rng = Rng::new();
        register_custom_rng(rng);
        (rng, EntropySource {})
    }
    /// Whether an entropy source is currently using the RNG register.
    /// Always true for esp32c5/c61 as there is no driver.
    #[must_use]
    pub fn entropy_source_active() -> bool {
        true
    }
    /// ESP32 RNG implementation
    pub struct EspRng;
    impl EspRng {
        #[must_use]
        pub fn new() -> Self {
            Self
        }
    }
    impl Default for EspRng {
        fn default() -> Self {
            Self::new()
        }
    }
    impl RngHal for EspRng {
        fn fill_bytes(
            &mut self,
            buf: &mut [u8],
        ) -> impl Future<Output = Result<(), HalError>> {
            ready(
                RNG_MUTEX
                    .lock(|t| {
                        let mut rng = t.borrow_mut();
                        let rng = rng.as_mut().ok_or(HalError::Rng)?;
                        rng.read(buf);
                        Ok(())
                    }),
            )
        }
    }
    /// Safe half of the `getrandom` custom backend: fills `buf` from the
    /// registered hardware RNG.
    ///
    /// The `__getrandom_v03_custom` that  [`getrandom_backend!`](macro@crate::getrandom_backend)
    /// defines forwards here. See the module docs for why the split exists.
    ///
    /// # Errors
    ///
    /// Returns an error if the RNG has not been registered via `register_custom_rng`.
    ///
    /// # Panics
    ///
    /// Panics if the RNG mutex lock fails internally.
    pub fn fill_bytes(buf: &mut [u8]) -> Result<(), getrandom::Error> {
        RNG_MUTEX
            .lock(|t| {
                let mut rng_ref = t.borrow_mut();
                let rng = rng_ref.as_mut().ok_or(getrandom::Error::UNEXPECTED)?;
                rng.read(buf);
                Ok(())
            })
    }
}
mod timer {
    //! Timer implementation for ESP32 family
    //!
    //! Provides microsecond and millisecond timing using ESP32 hardware timers.
    use embassy_time::{Duration, Instant};
    use ssh_stamp_hal::TimerHal;
    /// ESP32 Timer implementation using Embassy time
    pub struct EspTimer;
    impl TimerHal for EspTimer {
        fn now_micros(&self) -> u64 {
            Instant::now().as_micros()
        }
        async fn delay(&self, millis: u64) {
            embassy_time::Timer::after(Duration::from_millis(millis)).await;
        }
    }
}
mod uart {
    //! UART implementation for ESP32 family.
    //!
    //! Provides [`BufferedUart`] — a software-buffered, async, full-duplex UART
    //! satisfying [`ssh_stamp::serial::BufferedSerial`]. The bridge can poll the
    //! same UART from two futures (TX and RX) concurrently because both sides
    //! take `&self`.
    use core::future::Future;
    use embassy_executor::SendSpawner;
    use embassy_sync::pipe::TryWriteError;
    use embassy_sync::signal::Signal;
    use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, pipe::Pipe};
    use esp_hal::Async;
    use esp_hal::gpio::AnyPin;
    use esp_hal::peripherals::UART1;
    use esp_hal::uart::{Config, DataBits, Parity, RxConfig, StopBits, Uart};
    use portable_atomic::{AtomicUsize, Ordering};
    use ssh_stamp::serial::BufferedSerial;
    use ssh_stamp_hal::{Parity as LineParity, UartParams};
    use static_cell::StaticCell;
    const INWARD_BUF_SZ: usize = 512;
    const OUTWARD_BUF_SZ: usize = 256;
    const UART_BUF_SZ: usize = 64;
    /// The ESP32 UART peripherals reject anything above 5 Mbaud.
    const MAX_BAUD: u32 = 5_000_000;
    /// Bidirectional pipe buffer for UART communications.
    pub struct BufferedUart {
        outward: Pipe<CriticalSectionRawMutex, OUTWARD_BUF_SZ>,
        inward: Pipe<CriticalSectionRawMutex, INWARD_BUF_SZ>,
        dropped_rx_bytes: AtomicUsize,
    }
    impl BufferedUart {
        #[must_use]
        pub fn new() -> Self {
            BufferedUart {
                outward: Pipe::new(),
                inward: Pipe::new(),
                dropped_rx_bytes: AtomicUsize::from(0),
            }
        }
        /// Transfer data between UART hardware and internal buffers.
        ///
        /// This should be awaited from an Embassy task run in an `InterruptExecutor`
        /// for lower latency.
        pub async fn run(&self, uart: Uart<'_, Async>) {
            let (mut uart_rx, mut uart_tx) = uart.split();
            let mut rx_buf = [0u8; UART_BUF_SZ];
            let mut tx_buf = [0u8; UART_BUF_SZ];
            loop {
                use embassy_futures::select::select;
                let rd_from = async {
                    loop {
                        let Ok(n) = uart_rx.read_async(&mut rx_buf).await else {
                            continue;
                        };
                        let mut rx_slice = &rx_buf[..n];
                        while !rx_slice.is_empty() {
                            rx_slice = match self.inward.try_write(rx_slice) {
                                Ok(w) => &rx_slice[w..],
                                Err(TryWriteError::Full) => {
                                    let mut drop_buf = [0u8; UART_BUF_SZ];
                                    let dropped = self
                                        .inward
                                        .try_read(&mut drop_buf[..rx_slice.len()])
                                        .unwrap_or(0);
                                    let _ = self
                                        .dropped_rx_bytes
                                        .fetch_update(
                                            Ordering::Relaxed,
                                            Ordering::Relaxed,
                                            |d| Some(d.saturating_add(dropped)),
                                        );
                                    rx_slice
                                }
                            };
                        }
                    }
                };
                let rd_to = async {
                    loop {
                        let n = self.outward.read(&mut tx_buf).await;
                        let mut tx_slice = &tx_buf[..n];
                        while !tx_slice.is_empty() {
                            let Ok(written) = uart_tx.write_async(tx_slice).await else {
                                break;
                            };
                            tx_slice = &tx_slice[written..];
                        }
                    }
                };
                select(rd_from, rd_to).await;
            }
        }
        pub async fn read(&self, buf: &mut [u8]) -> usize {
            self.inward.read(buf).await
        }
        pub async fn write(&self, buf: &[u8]) {
            self.outward.write_all(buf).await;
        }
        /// Number of bytes the RX side dropped since the last call. Resets the counter.
        pub fn check_dropped_bytes(&self) -> usize {
            self.dropped_rx_bytes.swap(0, Ordering::Relaxed)
        }
    }
    impl Default for BufferedUart {
        fn default() -> Self {
            Self::new()
        }
    }
    impl BufferedSerial for BufferedUart {
        fn read(&self, buf: &mut [u8]) -> impl Future<Output = usize> {
            BufferedUart::read(self, buf)
        }
        fn write(&self, buf: &[u8]) -> impl Future<Output = ()> {
            BufferedUart::write(self, buf)
        }
        fn check_dropped_bytes(&self) -> usize {
            BufferedUart::check_dropped_bytes(self)
        }
    }
    /// UART pins configuration.
    ///
    /// The pin numbers inside come from the selected board's TOML in the
    /// `ssh-stamp-esp32-boards` crate; its front page carries the generated pin
    /// catalog for every board of this platform.
    pub struct EspUartPins<'a> {
        pub rx: AnyPin<'a>,
        pub tx: AnyPin<'a>,
    }
    /// Static storage for the buffered UART singleton.
    pub static UART_BUF: StaticCell<BufferedUart> = StaticCell::new();
    /// Signal raised by [`ssh_stamp::platform::PlatformServices::activate_uart`]
    /// to release [`uart_task`] from its initial wait.
    pub static UART_SIGNAL: Signal<CriticalSectionRawMutex, u8> = Signal::new();
    /// Translates the persisted, target-agnostic [`UartParams`] into an esp-hal
    /// [`Config`].
    ///
    /// Values the peripheral cannot honour fall back to the 8N1 default instead of
    /// refusing to bring the bridge up, so a stale or corrupt stored config still
    /// leaves a usable serial console.
    fn esp_uart_config(params: UartParams) -> Config {
        let data_bits = match params.data_bits {
            5 => DataBits::_5,
            6 => DataBits::_6,
            7 => DataBits::_7,
            _ => DataBits::_8,
        };
        let parity = match params.parity {
            LineParity::Even => Parity::Even,
            LineParity::Odd => Parity::Odd,
            LineParity::None => Parity::None,
        };
        let stop_bits = if params.stop_bits == 2 { StopBits::_2 } else { StopBits::_1 };
        Config::default()
            .with_baudrate(params.baud.clamp(1, MAX_BAUD))
            .with_data_bits(data_bits)
            .with_parity(parity)
            .with_stop_bits(stop_bits)
    }
    #[doc(hidden)]
    pub fn __uart_task_task(
        uart_buf: &'static BufferedUart,
        uart1: UART1<'static>,
        pins: EspUartPins<'static>,
        params: UartParams,
    ) -> impl ::core::future::Future<Output = ()> {
        /// Embassy task that owns the hardware UART and pumps it through
        /// [`BufferedUart::run`]. Spawn from a higher-priority `InterruptExecutor`
        /// for lower latency.
        ///
        /// `params` are the line settings from the device config, applied here since
        /// the UART is configured once for the lifetime of the boot.
        async fn __uart_task_task_inner_function(
            uart_buf: &'static BufferedUart,
            uart1: UART1<'static>,
            pins: EspUartPins<'static>,
            params: UartParams,
        ) {
            UART_SIGNAL.wait().await;
            let uart_config = esp_uart_config(params)
                .with_rx(
                    RxConfig::default().with_fifo_full_threshold(16).with_timeout(1),
                );
            let uart = Uart::new(uart1, uart_config).expect("UART config error");
            let uart = uart.with_rx(pins.rx).with_tx(pins.tx).into_async();
            uart_buf.run(uart).await;
        }
        { __uart_task_task_inner_function(uart_buf, uart1, pins, params) }
    }
    /// Embassy task that owns the hardware UART and pumps it through
    /// [`BufferedUart::run`]. Spawn from a higher-priority `InterruptExecutor`
    /// for lower latency.
    ///
    /// `params` are the line settings from the device config, applied here since
    /// the UART is configured once for the lifetime of the boot.
    pub fn uart_task(
        uart_buf: &'static BufferedUart,
        uart1: UART1<'static>,
        pins: EspUartPins<'static>,
        params: UartParams,
    ) -> ::core::result::Result<
        ::embassy_executor::SpawnToken<impl Sized>,
        ::embassy_executor::SpawnError,
    > {
        const fn __task_pool_get<F, Args, Fut>(
            _: F,
        ) -> &'static ::embassy_executor::raw::TaskPool<Fut, POOL_SIZE>
        where
            F: ::embassy_executor::_export::TaskFn<Args, Fut = Fut>,
            Fut: ::core::future::Future + 'static,
        {
            unsafe { &*POOL.get().cast() }
        }
        const POOL_SIZE: usize = 1;
        static POOL: ::embassy_executor::_export::TaskPoolHolder<
            {
                ::embassy_executor::_export::task_pool_size::<
                    _,
                    _,
                    _,
                    POOL_SIZE,
                >(__uart_task_task)
            },
            {
                ::embassy_executor::_export::task_pool_align::<
                    _,
                    _,
                    _,
                    POOL_SIZE,
                >(__uart_task_task)
            },
        > = unsafe {
            ::core::mem::transmute(
                ::embassy_executor::_export::task_pool_new::<
                    _,
                    _,
                    _,
                    POOL_SIZE,
                >(__uart_task_task),
            )
        };
        unsafe {
            __task_pool_get(__uart_task_task)
                ._spawn_async_fn(move || __uart_task_task(uart_buf, uart1, pins, params))
        }
    }
    /// Creates the [`BufferedUart`] singleton and spawns [`uart_task`] on the
    /// given spawner, returning the buffer the rest of the system talks to. The
    /// firmware feeds it the spawner from
    /// [`start_interrupt_executor`](crate::start_interrupt_executor), so the
    /// task runs at interrupt priority. The task waits on [`UART_SIGNAL`] before
    /// touching the hardware.
    ///
    /// # Panics
    ///
    /// Panics if called more than once per boot: the [`BufferedUart`] singleton
    /// and the task can each only be created once.
    pub fn spawn_uart(
        spawner: SendSpawner,
        uart1: UART1<'static>,
        pins: EspUartPins<'static>,
        params: UartParams,
    ) -> &'static BufferedUart {
        let uart_buf = UART_BUF.init_with(BufferedUart::new);
        spawner
            .spawn(
                uart_task(uart_buf, uart1, pins, params).expect("uart_task spawn failed"),
            );
        uart_buf
    }
}
pub use boot::start_interrupt_executor;
pub use flash::{EspOtaWriter, FlashBuffer, get_flash_n_buffer, init as flash_init};
pub use hash::EspHmac;
pub use network::{EspWifi, accept_requests, dhcp_server, net_up, wifi_up};
pub use platform::EspPlatform;
pub use rng::{
    EntropySource, EspRng, entropy_source_active, fill_bytes as rng_fill_bytes,
    init_entropy, register_custom_rng,
};
pub use timer::EspTimer;
pub use uart::{BufferedUart, EspUartPins, UART_BUF, UART_SIGNAL, spawn_uart, uart_task};
pub use esp_alloc;
pub use esp_bootloader_esp_idf;
pub use esp_hal;
pub use esp_println;
pub use esp_rtos;
pub use getrandom;
pub use log;
pub use ssh_stamp;
/// Read the device's hardware MAC address from eFuse.
#[must_use]
pub fn mac_address() -> [u8; 6] {
    let mac = esp_hal::efuse::base_mac_address();
    let bytes = mac.as_bytes();
    if true {
        {
            match (&bytes.len(), &6) {
                (left_val, right_val) => {
                    if !(*left_val == *right_val) {
                        let kind = ::core::panicking::AssertKind::Eq;
                        ::core::panicking::assert_failed(
                            kind,
                            &*left_val,
                            &*right_val,
                            ::core::option::Option::Some(
                                format_args!("eFuse MAC address must be 6 bytes"),
                            ),
                        );
                    }
                }
            }
        };
    }
    let mut arr = [0u8; 6];
    arr.copy_from_slice(bytes);
    arr
}
