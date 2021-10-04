use libbpf_sys::{
    xsk_umem_config, XDP_PACKET_HEADROOM, XSK_RING_CONS__DEFAULT_NUM_DESCS,
    XSK_RING_PROD__DEFAULT_NUM_DESCS, XSK_UMEM__DEFAULT_FRAME_HEADROOM,
    XSK_UMEM__DEFAULT_FRAME_SIZE,
};
use std::{error, fmt};

use super::{FrameSize, QueueSize};

/// Builder for a [`UmemConfig`](Config).
#[derive(Debug, Clone, Copy)]
pub struct ConfigBuilder {
    config: Config,
}

impl ConfigBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn frame_size(&mut self, size: FrameSize) -> &mut Self {
        self.config.frame_size = size;
        self
    }

    pub fn fill_queue_size(&mut self, size: QueueSize) -> &mut Self {
        self.config.fill_queue_size = size;
        self
    }

    pub fn comp_queue_size(&mut self, size: QueueSize) -> &mut Self {
        self.config.comp_queue_size = size;
        self
    }

    pub fn frame_headroom(&mut self, headroom: u32) -> &mut Self {
        self.config.frame_headroom = headroom;
        self
    }

    pub fn build(&self) -> Result<Config, ConfigBuildError> {
        let frame_size = self.config.frame_size.get();
        let total_headroom = XDP_PACKET_HEADROOM + self.config.frame_headroom;

        if total_headroom > frame_size {
            Err(ConfigBuildError {
                frame_size,
                total_headroom,
            })
        } else {
            Ok(self.config)
        }
    }
}

impl Default for ConfigBuilder {
    fn default() -> Self {
        Self {
            config: Default::default(),
        }
    }
}

/// Config for a [`Umem`](crate::umem::Umem) instance.
///
/// It's worth noting that the specified `frame_size` is not
/// necessarily the buffer size that will be available to write data
/// into. Some of this will be eaten up by
/// [`XDP_PACKET_HEADROOM`] and any
/// non-zero `frame_headroom`. Use the [`mtu`](Config::mtu) function
/// to determine whether the frame is large enough to hold the data
/// you wish to transmit.
#[derive(Debug, Clone, Copy)]
pub struct Config {
    frame_size: FrameSize,
    fill_queue_size: QueueSize,
    comp_queue_size: QueueSize,
    frame_headroom: u32,
}

impl Config {
    pub fn builder() -> ConfigBuilder {
        ConfigBuilder::new()
    }

    pub fn frame_size(&self) -> FrameSize {
        self.frame_size
    }

    pub fn fill_queue_size(&self) -> QueueSize {
        self.fill_queue_size
    }

    pub fn comp_queue_size(&self) -> QueueSize {
        self.comp_queue_size
    }

    pub fn frame_headroom(&self) -> u32 {
        self.frame_headroom
    }

    pub fn mtu(&self) -> u32 {
        self.frame_size.get() - (XDP_PACKET_HEADROOM + self.frame_headroom)
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            frame_size: FrameSize(XSK_UMEM__DEFAULT_FRAME_SIZE),
            fill_queue_size: QueueSize(XSK_RING_PROD__DEFAULT_NUM_DESCS),
            comp_queue_size: QueueSize(XSK_RING_CONS__DEFAULT_NUM_DESCS),
            frame_headroom: XSK_UMEM__DEFAULT_FRAME_HEADROOM,
        }
    }
}

impl From<Config> for xsk_umem_config {
    fn from(c: Config) -> Self {
        xsk_umem_config {
            fill_size: c.fill_queue_size.get(),
            comp_size: c.comp_queue_size.get(),
            frame_size: c.frame_size.get(),
            frame_headroom: c.frame_headroom,
            flags: 0,
        }
    }
}

/// Error detailing why [`UmemConfig`](Config) creation failed.
#[derive(Debug)]
pub struct ConfigBuildError {
    frame_size: u32,
    total_headroom: u32,
}

impl fmt::Display for ConfigBuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "total headroom {} cannot be greater than frame size {}",
            self.total_headroom, self.frame_size
        )
    }
}

impl error::Error for ConfigBuildError {}
