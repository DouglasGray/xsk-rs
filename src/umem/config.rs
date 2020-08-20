use bitflags::bitflags;
use libbpf_sys::{
    XSK_RING_CONS__DEFAULT_NUM_DESCS, XSK_RING_PROD__DEFAULT_NUM_DESCS,
    XSK_UMEM__DEFAULT_FRAME_HEADROOM, XSK_UMEM__DEFAULT_FRAME_SIZE,
};
use std::num::NonZeroU32;
use thiserror::Error;

use crate::util;

bitflags! {
    pub struct UmemFlags: u32 {
        const XDP_UMEM_UNALIGNED_CHUNK_FLAG = 1;
    }
}

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Completion queue size invalid")]
    CompSizeInvalid { reason: &'static str },
    #[error("Fill queue size invalid")]
    FillSizeInvalid { reason: &'static str },
    #[error("Frame size invalid")]
    FrameSizeInvalid { reason: &'static str },
}

#[derive(Debug, Clone)]
pub struct Config {
    frame_count: u32,
    frame_size: u32,
    fill_queue_size: u32,
    comp_queue_size: u32,
    frame_headroom: u32,
    use_huge_pages: bool,
    umem_flags: UmemFlags,
}

impl Config {
    pub fn new(
        frame_count: NonZeroU32,
        frame_size: NonZeroU32,
        fill_queue_size: u32,
        comp_queue_size: u32,
        frame_headroom: u32,
        use_huge_pages: bool,
        umem_flags: UmemFlags,
    ) -> Result<Self, ConfigError> {
        if !util::is_pow_of_two(fill_queue_size) {
            return Err(ConfigError::FillSizeInvalid {
                reason: "Fill queue size must be a power of two",
            });
        }
        if !util::is_pow_of_two(comp_queue_size) {
            return Err(ConfigError::CompSizeInvalid {
                reason: "Comp queue size must be a power of two",
            });
        }
        if frame_size.get() < 2048 {
            return Err(ConfigError::FrameSizeInvalid {
                reason: "Frame size must be greater than or equal to 2048",
            });
        }

        Ok(Config {
            frame_count: frame_count.get(),
            frame_size: frame_size.get(),
            fill_queue_size,
            comp_queue_size,
            frame_headroom,
            use_huge_pages,
            umem_flags,
        })
    }

    pub fn default(frame_count: NonZeroU32, use_huge_pages: bool) -> Self {
        Config {
            frame_count: frame_count.get(),
            frame_size: XSK_UMEM__DEFAULT_FRAME_SIZE,
            fill_queue_size: XSK_RING_PROD__DEFAULT_NUM_DESCS,
            comp_queue_size: XSK_RING_CONS__DEFAULT_NUM_DESCS,
            frame_headroom: XSK_UMEM__DEFAULT_FRAME_HEADROOM,
            use_huge_pages,
            umem_flags: UmemFlags::empty(),
        }
    }

    pub fn frame_count(&self) -> u32 {
        self.frame_count
    }

    pub fn frame_size(&self) -> u32 {
        self.frame_size
    }

    pub fn fill_queue_size(&self) -> u32 {
        self.fill_queue_size
    }

    pub fn comp_queue_size(&self) -> u32 {
        self.comp_queue_size
    }

    pub fn frame_headroom(&self) -> u32 {
        self.frame_headroom
    }

    pub fn use_huge_pages(&self) -> bool {
        self.use_huge_pages
    }

    pub fn umem_flags(&self) -> &UmemFlags {
        &self.umem_flags
    }

    pub fn umem_len(&self) -> u64 {
        (self.frame_count as u64)
            .checked_mul(self.frame_size as u64)
            .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn umem_len_doesnt_panic_when_frame_count_and_size_are_u32_max() {
        Config::new(
            NonZeroU32::new(u32::MAX).unwrap(),
            NonZeroU32::new(u32::MAX).unwrap(),
            8,
            8,
            0,
            false,
            UmemFlags::empty(),
        )
        .unwrap()
        .umem_len();
    }
}
