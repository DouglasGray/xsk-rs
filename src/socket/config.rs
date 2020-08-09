use bitflags::bitflags;
use libbpf_sys::{XSK_RING_CONS__DEFAULT_NUM_DESCS, XSK_RING_PROD__DEFAULT_NUM_DESCS};
use thiserror::Error;

use crate::util;

bitflags! {
    pub struct LibbpfFlags: u32 {
        const XSK_LIBBPF_FLAGS__INHIBIT_PROG_LOAD = 1;
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
        const XDP_SHARED_UMEM = 1;
        const XDP_COPY = 2;
        const XDP_ZEROCOPY = 4;
        const XDP_USE_NEED_WAKEUP = 8;
    }
}

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Tx queue size invalid, must be a power of two")]
    TxSizeInvalid,
    #[error("Rx queue size invalid, must be a power of two")]
    RxSizeInvalid,
}

#[derive(Debug, Clone)]
pub struct Config {
    rx_queue_size: u32,
    tx_queue_size: u32,
    libbpf_flags: LibbpfFlags,
    xdp_flags: XdpFlags,
    bind_flags: BindFlags,
}

impl Config {
    pub fn new(
        rx_queue_size: u32,
        tx_queue_size: u32,
        libbpf_flags: LibbpfFlags,
        xdp_flags: XdpFlags,
        bind_flags: BindFlags,
    ) -> Result<Self, ConfigError> {
        if !util::is_pow_of_two(rx_queue_size) {
            return Err(ConfigError::RxSizeInvalid);
        }
        if !util::is_pow_of_two(tx_queue_size) {
            return Err(ConfigError::TxSizeInvalid);
        }

        Ok(Config {
            rx_queue_size,
            tx_queue_size,
            libbpf_flags,
            xdp_flags,
            bind_flags,
        })
    }

    pub fn default() -> Self {
        Config {
            rx_queue_size: XSK_RING_CONS__DEFAULT_NUM_DESCS,
            tx_queue_size: XSK_RING_PROD__DEFAULT_NUM_DESCS,
            libbpf_flags: LibbpfFlags::empty(),
            xdp_flags: XdpFlags::empty(),
            bind_flags: BindFlags::empty(),
        }
    }

    pub fn rx_queue_size(&self) -> u32 {
        self.rx_queue_size
    }

    pub fn tx_queue_size(&self) -> u32 {
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
