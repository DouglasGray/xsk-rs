use libxdp_sys::{
    xsk_umem_config, XDP_PACKET_HEADROOM, XSK_RING_CONS__DEFAULT_NUM_DESCS,
    XSK_RING_PROD__DEFAULT_NUM_DESCS, XSK_UMEM__DEFAULT_FRAME_HEADROOM,
    XSK_UMEM__DEFAULT_FRAME_SIZE,
};
use std::{error, fmt};

use super::{FrameSize, QueueSize};

/// Builder for a [`UmemConfig`](Config).
#[derive(Debug, Default, Clone, Copy)]
pub struct ConfigBuilder {
    config: Config,
}

impl ConfigBuilder {
    /// Creates a new [`UmemConfigBuilder`](ConfigBuilder) instance.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the frame size. Default is
    /// [`XSK_UMEM__DEFAULT_FRAME_SIZE`].
    pub fn frame_size(&mut self, size: FrameSize) -> &mut Self {
        self.config.frame_size = size;
        self
    }

    /// Set the [`FillQueue`](crate::FillQueue) size. Default is
    /// [`XSK_RING_PROD__DEFAULT_NUM_DESCS`].
    pub fn fill_queue_size(&mut self, size: QueueSize) -> &mut Self {
        self.config.fill_queue_size = size;
        self
    }

    /// Set the [`CompQueue`](crate::CompQueue) size. Default is
    /// [`XSK_RING_CONS__DEFAULT_NUM_DESCS`].
    pub fn comp_queue_size(&mut self, size: QueueSize) -> &mut Self {
        self.config.comp_queue_size = size;
        self
    }

    /// Set the frame headroom available to the user. Default size is
    /// [`XSK_UMEM__DEFAULT_FRAME_HEADROOM`].
    ///
    /// Not to be confused with [`XDP_PACKET_HEADROOM`] which is the
    /// amount of headroom reserved by XDP.
    pub fn frame_headroom(&mut self, headroom: u32) -> &mut Self {
        self.config.frame_headroom = headroom;
        self
    }

    /// Build a [`UmemConfig`](Config) instance using the values set
    /// in this builder.
    ///
    /// May fail if some of the values are incompatible. For example,
    /// if the requested frame headroom exceeds the frame size.
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

/// Config for a [`Umem`](crate::umem::Umem) instance.
///
/// It's worth noting that the specified `frame_size` is not
/// necessarily the buffer size that will be available to write data
/// into. Some of this will be eaten up by XDP program headroom
/// ([`XDP_PACKET_HEADROOM`]) and any non-zero `frame_headroom`. Use
/// the [`mtu`](Config::mtu) function to determine whether the frame
/// is large enough to hold the data you wish to transmit.
#[derive(Debug, Clone, Copy)]
pub struct Config {
    frame_size: FrameSize,
    fill_queue_size: QueueSize,
    comp_queue_size: QueueSize,
    frame_headroom: u32,
}

impl Config {
    /// Creates a new [`UmemConfigBuilder`](ConfigBuilder) instance
    /// with with sizes as per the `libbpf` defaults.
    pub fn builder() -> ConfigBuilder {
        ConfigBuilder::new()
    }

    /// The size of each frame in the [`Umem`](crate::Umem).
    pub fn frame_size(&self) -> FrameSize {
        self.frame_size
    }

    /// The [`FillQueue`](crate::FillQueue) size.
    pub fn fill_queue_size(&self) -> QueueSize {
        self.fill_queue_size
    }

    /// The [`CompQueue`](crate::CompQueue) size.
    pub fn comp_queue_size(&self) -> QueueSize {
        self.comp_queue_size
    }

    /// The frame headroom reserved for the XDP program.
    pub fn xdp_headroom(&self) -> u32 {
        XDP_PACKET_HEADROOM
    }

    /// The frame headroom available to the user.
    pub fn frame_headroom(&self) -> u32 {
        self.frame_headroom
    }

    /// The maximum transmission unit, or the length of the packet
    /// data segment of the frame.
    ///
    /// Is defined as the frame size minus both the XDP headroom and
    /// user headroom.
    pub fn mtu(&self) -> u32 {
        self.frame_size.get() - (self.xdp_headroom() + self.frame_headroom)
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

#[cfg(test)]
mod tests {
    use std::convert::TryInto;

    use crate::config::XDP_UMEM_MIN_CHUNK_SIZE;

    use super::*;

    #[test]
    fn frame_size_must_be_greater_than_total_headroom() {
        assert!(ConfigBuilder::new()
            .frame_headroom(XDP_UMEM_MIN_CHUNK_SIZE - XDP_PACKET_HEADROOM)
            .frame_size(XDP_UMEM_MIN_CHUNK_SIZE.try_into().unwrap())
            .build()
            .is_ok());

        assert!(ConfigBuilder::new()
            .frame_headroom(XDP_UMEM_MIN_CHUNK_SIZE - (XDP_PACKET_HEADROOM - 1))
            .frame_size(XDP_UMEM_MIN_CHUNK_SIZE.try_into().unwrap())
            .build()
            .is_err());
    }

    #[test]
    fn frame_mtu_has_expected_value() {
        let frame_headroom = 1024;

        let config = ConfigBuilder::new()
            .frame_headroom(frame_headroom)
            .frame_size(XDP_UMEM_MIN_CHUNK_SIZE.try_into().unwrap())
            .build()
            .unwrap();

        assert_eq!(
            config.mtu(),
            XDP_UMEM_MIN_CHUNK_SIZE - (frame_headroom + XDP_PACKET_HEADROOM)
        );
    }
}
