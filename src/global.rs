pub(crate) struct GlobalState {
    // If locking multiple mutexes, always lock in the order below to avoid inversion.
    pub usb_pipe: &'static TakePipe<'static>,
    pub serial1_pipe: &'static TakePipe<'static>,

    pub config: &'static SunsetMutex<SSHConfig>,
    pub flash: &'static SunsetMutex<flashconfig::Fl<'static>>,
    pub watchdog: &'static SunsetMutex<embassy_rp::watchdog::Watchdog>,

    pub net_mac: SunsetMutex<[u8; 6]>,
}

struct PicoServer {
    global: &'static GlobalState,
}