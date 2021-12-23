//! [`Umem`](crate::umem::Umem) and [`Socket`](crate::socket::Socket)
//! configuration.

mod socket;
pub use socket::{
    BindFlags, Config as SocketConfig, ConfigBuilder as SocketConfigBuilder, Interface,
    LibbpfFlags, XdpFlags,
};

mod umem;
pub use umem::{
    Config as UmemConfig, ConfigBuildError as UmemConfigBuilderError,
    ConfigBuilder as UmemConfigBuilder,
};

use std::{convert::TryFrom, error, fmt};

use crate::util;

/// A ring's buffer size. Must be a power of two.
#[derive(Debug, Clone, Copy)]
pub struct QueueSize(u32);

impl QueueSize {
    /// Create a new `QueueSize` instance. Fails if `size` is not a
    /// power of two.
    pub fn new(size: u32) -> Result<Self, QueueSizeError> {
        if !util::is_pow_of_two(size) {
            Err(QueueSizeError(size))
        } else {
            Ok(Self(size))
        }
    }

    /// The queue size.
    pub fn get(&self) -> u32 {
        self.0
    }
}

impl TryFrom<u32> for QueueSize {
    type Error = QueueSizeError;

    fn try_from(size: u32) -> Result<Self, Self::Error> {
        QueueSize::new(size)
    }
}

/// Error signifying incorrect queue size.
#[derive(Debug)]
pub struct QueueSizeError(u32);

impl fmt::Display for QueueSizeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "expected a power of two as queue size, got {}", self.0)
    }
}

impl error::Error for QueueSizeError {}

/// The size of a [`Umem`](crate::umem::Umem) frame. Cannot be smaller than 2048.
#[derive(Debug, Clone, Copy)]
pub struct FrameSize(u32);

impl FrameSize {
    /// Create a new `FrameSize` instance. Fails if `size` is smaller than 2048.
    pub fn new(size: u32) -> Result<Self, FrameSizeError> {
        if size < 2048 {
            Err(FrameSizeError(size))
        } else {
            Ok(Self(size))
        }
    }

    /// The frame size.
    pub fn get(&self) -> u32 {
        self.0
    }
}

impl TryFrom<u32> for FrameSize {
    type Error = FrameSizeError;

    fn try_from(size: u32) -> Result<Self, Self::Error> {
        FrameSize::new(size)
    }
}

/// Error signifying incorrect frame size.
#[derive(Debug)]
pub struct FrameSizeError(u32);

impl fmt::Display for FrameSizeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "expected frame size >= 2048, got {}", self.0)
    }
}

impl error::Error for FrameSizeError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_size_should_accept_only_non_zero_powers_of_two() {
        assert!(QueueSize::new(0).is_err());
        assert!(QueueSize::new(1).is_ok());
        assert!(QueueSize::new(2).is_ok());
        assert!(QueueSize::new(3).is_err());
        assert!(QueueSize::new(4).is_ok());
    }

    #[test]
    fn frame_size_should_reject_values_below_2048() {
        assert!(FrameSize::new(0).is_err());
        assert!(FrameSize::new(2047).is_err());
        assert!(FrameSize::new(2048).is_ok());
        assert!(FrameSize::new(2049).is_ok())
    }
}
