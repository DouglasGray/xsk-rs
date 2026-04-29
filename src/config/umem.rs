use libxdp_sys::{XDP_PACKET_HEADROOM, XSK_RING_CONS__DEFAULT_NUM_DESCS, XSK_RING_PROD__DEFAULT_NUM_DESCS, XSK_UMEM__DEFAULT_FRAME_HEADROOM, XSK_UMEM__DEFAULT_FRAME_SIZE, xsk_umem_config, xsk_umem_opts, XSK_UMEM__DEFAULT_TX_METADATA_LEN, XSK_UMEM__DEFAULT_FLAGS};
use std::{error, fmt};
use std::num::{NonZeroU32, NonZeroU64};
use super::{FrameSize, QueueSize};

/// Builder for a [`UmemConfig`](Config).
#[derive(Debug, Clone, Copy)]
pub struct ConfigBuilder {
    config: ConfigOpts,
    frame_count: NonZeroU32,
}

impl Default for ConfigBuilder {
    fn default() -> Self {
        Self {
            config: ConfigOpts::default(),
            frame_count: NonZeroU32::new(4096).unwrap(),
        }
    }   
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
    
/// Set the frame count. Default is 4096. Only used for building ConfigOpts
    pub fn frame_count(&mut self, count: NonZeroU32) -> &mut Self {
        self.frame_count = count;
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

    /// Set the flags field for the umem creation with xdp_umem_create_opts
    pub fn flags(&mut self, flags: u32) -> &mut Self {
        self.config.flags = flags;
        self
    }

    /// Set the tx_metadata_len field for the umem creation with xdp_umem_create_opts
    pub fn tx_metadata_len(&mut self, len: u32) -> &mut Self {
        self.config.tx_metadata_len = len;
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
            Ok(Config::from(self.config))
        }
    }


    /// Similar to build, but return ConfigOpts instead of Config
    pub fn build_opts(&mut self) -> Result<ConfigOpts, ConfigBuildError> {
        let frame_size = self.config.frame_size.get();
        let total_headroom = XDP_PACKET_HEADROOM + self.config.frame_headroom + self.config.tx_metadata_len;
        self.config.size = frame_size as u64 * self.frame_count.get() as u64 ;
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


/// supporting the new API of lib_xdp 1.6.3 for umem creation, namely xdp_umem_create_opts
#[derive(Debug, Clone, Copy)]
pub struct ConfigOpts {
    fd: ::std::os::raw::c_int,
    /// size of the umem, in bytes.
    size: u64,
    frame_size: FrameSize,
    fill_queue_size: QueueSize,
    comp_queue_size: QueueSize,
    frame_headroom: u32,
    flags: u32,
    tx_metadata_len: u32,
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

impl ConfigOpts {
    /// The file descriptor associated with this umem.
    pub fn fd(&self) -> ::std::os::raw::c_int {
        self.fd
    }

    /// The size of the umem, in bytes.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// The number of frames in the [`Umem`](crate::Umem). Is calculated from size and frame_size.
    pub fn frame_count(&self) -> NonZeroU32 {
        ((self.size / self.frame_size.get() as u64) as u32).try_into().unwrap()
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

    /// The flags set for umem creation.
    pub fn flags(&self) -> u32 {
        self.flags
    }

    /// The tx metadata length.
    pub fn tx_metadata_len(&self) -> u32 {
        self.tx_metadata_len
    }

    /// The maximum transmission unit, or the length of the packet
    /// data segment of the frame.
    ///
    /// Is defined as the frame size minus both the XDP headroom and
    /// user headroom.
    pub fn mtu(&self) -> u32 {
        self.frame_size.get() - (self.xdp_headroom() + self.frame_headroom + self.tx_metadata_len)
    }
}

impl Default for ConfigOpts {
    fn default() -> Self {
        Self {
            fd: 0,
            size: 0,
            frame_size: FrameSize(XSK_UMEM__DEFAULT_FRAME_SIZE),
            fill_queue_size: QueueSize(XSK_RING_PROD__DEFAULT_NUM_DESCS),
            comp_queue_size: QueueSize(XSK_RING_CONS__DEFAULT_NUM_DESCS),
            frame_headroom: XSK_UMEM__DEFAULT_FRAME_HEADROOM,
            flags: XSK_UMEM__DEFAULT_FLAGS,
            tx_metadata_len: XSK_UMEM__DEFAULT_TX_METADATA_LEN,
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

impl From<ConfigOpts> for xsk_umem_opts {
    fn from(c: ConfigOpts) -> Self {
        xsk_umem_opts {
            sz: std::mem::size_of::<xsk_umem_opts>(),
            fd: c.fd,
            size: c.size,
            fill_size: c.fill_queue_size.get(),
            comp_size: c.comp_queue_size.get(),
            frame_size: c.frame_size.get(),
            frame_headroom: c.frame_headroom,
            flags: c.flags,
            tx_metadata_len: c.tx_metadata_len,
        }
    }
}

impl From<ConfigOpts> for Config {
    fn from(c: ConfigOpts) -> Self {
        Config {
            fill_queue_size: c.fill_queue_size,
            comp_queue_size: c.comp_queue_size,
            frame_size: c.frame_size,
            frame_headroom: c.frame_headroom,
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
        assert!(
            ConfigBuilder::new()
                .frame_headroom(XDP_UMEM_MIN_CHUNK_SIZE - XDP_PACKET_HEADROOM)
                .frame_size(XDP_UMEM_MIN_CHUNK_SIZE.try_into().unwrap())
                .build()
                .is_ok()
        );

        assert!(
            ConfigBuilder::new()
                .frame_headroom(XDP_UMEM_MIN_CHUNK_SIZE - (XDP_PACKET_HEADROOM - 1))
                .frame_size(XDP_UMEM_MIN_CHUNK_SIZE.try_into().unwrap())
                .build()
                .is_err()
        );
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
