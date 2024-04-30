use bitflags::bitflags;
use libxdp_sys::{
    xsk_socket_config, xsk_socket_config__bindgen_ty_1, XSK_RING_CONS__DEFAULT_NUM_DESCS,
    XSK_RING_PROD__DEFAULT_NUM_DESCS,
};
use std::{
    convert::{TryFrom, TryInto},
    ffi::{CStr, CString, NulError},
    str::FromStr,
};

use super::QueueSize;

bitflags! {
    /// Libbpf flags.
    #[derive(Debug, Clone, Copy)]
    pub struct LibxdpFlags: u32 {
        /// Set to avoid loading of default XDP program on socket
        /// creation.
        const XSK_LIBXDP_FLAGS_INHIBIT_PROG_LOAD = 1;
    }
}

bitflags! {
    /// XDP flags.
    ///
    /// Some may not be applicable if an XDP program is already loaded
    /// on the target interface.
    #[derive(Debug, Clone, Copy)]
    pub struct XdpFlags: u32 {
        /// Fail if an XDP program is already loaded on the target
        /// interface.
        const XDP_FLAGS_UPDATE_IF_NOEXIST = 1;
        /// Force generic/SKB mode.
        const XDP_FLAGS_SKB_MODE = 2;
        /// Force driver mode. The driver must support XDP.
        const XDP_FLAGS_DRV_MODE = 4;
        /// Offload to hardware. The NIC must support XDP.
        const XDP_FLAGS_HW_MODE = 8;
    }
}

bitflags! {
    /// Bind flags.
    #[derive(Debug, Clone, Copy)]
    pub struct BindFlags: u16 {
        /// Forces copy-mode.
        const XDP_COPY = 2;
        /// Forces zero-copy mode. Socket creation will fail if not
        /// available.
        const XDP_ZEROCOPY = 4;
        /// If set, the driver may go to sleep, meaning the
        /// [`FillQueue`](crate::FillQueue) and/or
        /// [`TxQueue`](crate::TxQueue) will need waking up (using the
        /// `*_wakeup` or `poll` functions available on either
        /// struct). It is recommended to enable this flag as it often
        /// leads to better performance but especially if the driver
        /// and application are running on the same core. More details
        /// in the
        /// [docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html#xdp-use-need-wakeup-bind-flag).
        const XDP_USE_NEED_WAKEUP = 8;
    }
}

/// A device interface name.
#[derive(Debug, Clone)]
pub struct Interface(CString);

impl Interface {
    /// Creates a new `Interface` instance.
    pub fn new(name: CString) -> Self {
        Self(name)
    }

    pub(crate) fn as_cstr(&self) -> &CStr {
        &self.0
    }
}

impl FromStr for Interface {
    type Err = NulError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.as_bytes().try_into()
    }
}

impl TryFrom<&[u8]> for Interface {
    type Error = NulError;

    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        CString::new(bytes).map(Self)
    }
}

impl TryFrom<Vec<u8>> for Interface {
    type Error = NulError;

    fn try_from(bytes: Vec<u8>) -> Result<Self, Self::Error> {
        CString::new(bytes).map(Self)
    }
}

/// Builder for a [`SocketConfig`](Config).
#[derive(Debug, Default, Clone, Copy)]
pub struct ConfigBuilder {
    config: Config,
}

impl ConfigBuilder {
    /// Creates a new [`SocketConfigBuilder`](ConfigBuilder) instance
    /// with no flags set and with queue sizes as per the `libbpf`
    /// defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the [`RxQueue`](crate::RxQueue) size. Default is
    /// [`XSK_RING_CONS__DEFAULT_NUM_DESCS`].
    pub fn rx_queue_size(&mut self, size: QueueSize) -> &mut Self {
        self.config.rx_queue_size = size;
        self
    }

    /// Set the [`TxQueue`](crate::RxQueue) size. Default is
    /// [`XSK_RING_PROD__DEFAULT_NUM_DESCS`].
    pub fn tx_queue_size(&mut self, size: QueueSize) -> &mut Self {
        self.config.tx_queue_size = size;
        self
    }

    /// Set the [`LibxdpFlags`]. Default is no flags set.
    pub fn libxdp_flags(&mut self, flags: LibxdpFlags) -> &mut Self {
        self.config.libxdp_flags = flags;
        self
    }

    /// Set the [`XdpFlags`]. Default is no flags set.
    pub fn xdp_flags(&mut self, flags: XdpFlags) -> &mut Self {
        self.config.xdp_flags = flags;
        self
    }

    /// Set the socket [`BindFlags`]. Default is no flags set.
    pub fn bind_flags(&mut self, flags: BindFlags) -> &mut Self {
        self.config.bind_flags = flags;
        self
    }

    /// Build a [`SocketConfig`](Config) instance using the values set
    /// in this builder.
    pub fn build(&self) -> Config {
        self.config
    }
}

/// Config for an AF_XDP [`Socket`](crate::Socket) instance.
#[derive(Debug, Clone, Copy)]
pub struct Config {
    rx_queue_size: QueueSize,
    tx_queue_size: QueueSize,
    libxdp_flags: LibxdpFlags,
    xdp_flags: XdpFlags,
    bind_flags: BindFlags,
}

impl Config {
    /// Creates a [`SocketConfigBuilder`](ConfigBuilder) instance.
    pub fn builder() -> ConfigBuilder {
        ConfigBuilder::new()
    }

    /// The socket's [`RxQueue`](crate::RxQueue) size.
    pub fn rx_queue_size(&self) -> QueueSize {
        self.rx_queue_size
    }

    /// The socket's [`TxQueue`](crate::TxQueue) size.
    pub fn tx_queue_size(&self) -> QueueSize {
        self.tx_queue_size
    }

    /// The [`LibxdpFlags`] set.
    pub fn libxdp_flags(&self) -> &LibxdpFlags {
        &self.libxdp_flags
    }

    /// The [`XdpFlags`] set.
    pub fn xdp_flags(&self) -> &XdpFlags {
        &self.xdp_flags
    }

    /// The [`BindFlags`] set.
    pub fn bind_flags(&self) -> &BindFlags {
        &self.bind_flags
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            rx_queue_size: QueueSize(XSK_RING_CONS__DEFAULT_NUM_DESCS),
            tx_queue_size: QueueSize(XSK_RING_PROD__DEFAULT_NUM_DESCS),
            libxdp_flags: LibxdpFlags::empty(),
            xdp_flags: XdpFlags::empty(),
            bind_flags: BindFlags::empty(),
        }
    }
}

impl From<Config> for xsk_socket_config {
    fn from(c: Config) -> Self {
        let xsk_socket_config = xsk_socket_config__bindgen_ty_1 {
            libxdp_flags: c.libxdp_flags.bits(),
        };

        xsk_socket_config {
            rx_size: c.rx_queue_size.get(),
            tx_size: c.tx_queue_size.get(),
            xdp_flags: c.xdp_flags.bits(),
            bind_flags: c.bind_flags.bits(),
            __bindgen_anon_1: xsk_socket_config,
        }
    }
}
