use bitflags::bitflags;
use libbpf_sys::{
    xsk_socket_config, XSK_RING_CONS__DEFAULT_NUM_DESCS, XSK_RING_PROD__DEFAULT_NUM_DESCS,
};
use std::{
    convert::{TryFrom, TryInto},
    ffi::{CStr, CString, NulError},
    str::FromStr,
};

use super::QueueSize;

bitflags! {
    pub struct LibbpfFlags: u32 {
        const XSK_LIBBPF_FLAGS_INHIBIT_PROG_LOAD = 1;
    }
}

bitflags! {
    pub struct XdpFlags: u32 {
        const XDP_FLAGS_UPDATE_IF_NOEXIST = 1;
        const XDP_FLAGS_SKB_MODE = 2;
        const XDP_FLAGS_DRV_MODE = 4;
        const XDP_FLAGS_HW_MODE = 8;
        const XDP_FLAGS_REPLACE = 16;
    }
}

bitflags! {
    pub struct BindFlags: u16 {
        const XDP_UMEM_SHARED_MEMORY = 1;
        const XDP_COPY = 2;
        const XDP_ZEROCOPY = 4;
        const XDP_USE_NEED_WAKEUP = 8;
    }
}

/// A device interface name.
#[derive(Debug, Clone)]
pub struct Interface(CString);

impl Interface {
    pub fn new(name: CString) -> Self {
        Self(name)
    }

    pub fn as_cstr(&self) -> &CStr {
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
#[derive(Debug, Clone, Copy)]
pub struct ConfigBuilder {
    config: Config,
}

impl ConfigBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn rx_queue_size(&mut self, size: QueueSize) -> &mut Self {
        self.config.rx_queue_size = size;
        self
    }

    pub fn tx_queue_size(&mut self, size: QueueSize) -> &mut Self {
        self.config.tx_queue_size = size;
        self
    }

    pub fn libbpf_flags(&mut self, flags: LibbpfFlags) -> &mut Self {
        self.config.libbpf_flags = flags;
        self
    }

    pub fn xdp_flags(&mut self, flags: XdpFlags) -> &mut Self {
        self.config.xdp_flags = flags;
        self
    }

    pub fn bind_flags(&mut self, flags: BindFlags) -> &mut Self {
        self.config.bind_flags = flags;
        self
    }

    pub fn build(&self) -> Config {
        self.config
    }
}

impl Default for ConfigBuilder {
    fn default() -> Self {
        Self {
            config: Default::default(),
        }
    }
}

/// Config for an AF_XDP [`Socket`](crate::socket::Socket) instance.
#[derive(Debug, Clone, Copy)]
pub struct Config {
    rx_queue_size: QueueSize,
    tx_queue_size: QueueSize,
    libbpf_flags: LibbpfFlags,
    xdp_flags: XdpFlags,
    bind_flags: BindFlags,
}

impl Config {
    pub fn builder() -> ConfigBuilder {
        ConfigBuilder::new()
    }

    pub fn rx_queue_size(&self) -> QueueSize {
        self.rx_queue_size
    }

    pub fn tx_queue_size(&self) -> QueueSize {
        self.tx_queue_size
    }

    pub fn libbpf_flags(&self) -> &LibbpfFlags {
        &self.libbpf_flags
    }

    pub fn xdp_flags(&self) -> &XdpFlags {
        &self.xdp_flags
    }

    pub fn bind_flags(&self) -> &BindFlags {
        &self.bind_flags
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            rx_queue_size: QueueSize(XSK_RING_CONS__DEFAULT_NUM_DESCS),
            tx_queue_size: QueueSize(XSK_RING_PROD__DEFAULT_NUM_DESCS),
            libbpf_flags: LibbpfFlags::empty(),
            xdp_flags: XdpFlags::empty(),
            bind_flags: BindFlags::empty(),
        }
    }
}

impl From<Config> for xsk_socket_config {
    fn from(c: Config) -> Self {
        xsk_socket_config {
            rx_size: c.rx_queue_size.get(),
            tx_size: c.tx_queue_size.get(),
            xdp_flags: c.xdp_flags.bits(),
            bind_flags: c.bind_flags.bits(),
            libbpf_flags: c.libbpf_flags.bits(),
            __bindgen_padding_0: Default::default(),
        }
    }
}
