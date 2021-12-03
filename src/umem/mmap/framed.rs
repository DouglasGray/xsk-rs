use std::{
    io::{self, ErrorKind},
    sync::Arc,
};

use super::Mmap;

#[derive(Debug)]
pub struct Headroom<T>(pub T);

#[derive(Debug)]
pub struct Data<T>(pub T);

/// Dimensions of a [`Umem`](crate::umem::Umem) frame.
#[derive(Debug, Clone, Copy)]
pub struct FrameLayout {
    pub xdp_headroom: usize,
    pub frame_headroom: usize,
    pub data_size: usize,
}

/// A [`Mmap`] chunked into frames of size `frame_size`.
#[derive(Clone)]
pub struct FramedMmap {
    frame_size: usize,
    layout: FrameLayout,
    mmap: Arc<Mmap>,
}

impl FramedMmap {
    pub fn new(layout: FrameLayout, mmap: Arc<Mmap>) -> io::Result<Self> {
        let frame_size = layout.xdp_headroom + layout.frame_headroom + layout.data_size;

        if frame_size == 0 || mmap.len % frame_size != 0 {
            Err(io::Error::new(
                ErrorKind::Other,
                format!(
                    "mmap with len {} cannot be split exactly into frames of size {}",
                    mmap.len, frame_size
                ),
            ))
        } else {
            Ok(FramedMmap {
                frame_size,
                layout,
                mmap,
            })
        }
    }

    /// Retrieve a immutable pointer and length pair which describe
    /// the frame's headroom and data segments respectively.
    ///
    /// # Safety
    ///
    /// `addr` must be within the [`Mmap`] region.
    #[inline]
    pub unsafe fn get_unchecked(
        &self,
        addr: usize,
    ) -> (Headroom<(*const u8, usize)>, Data<(*const u8, usize)>) {
        let (h, d) = unsafe { self.frame_pointers(addr) };

        (
            Headroom((h.0 as *const u8, self.layout.frame_headroom)),
            Data((d.0 as *const u8, self.layout.data_size)),
        )
    }

    /// Retrieve a mutable pointer and length pair which describe
    /// the frame's headroom and data segments respectively.
    ///
    /// # Safety
    ///
    /// `addr` must be within the [`Mmap`] region.
    #[inline]
    pub unsafe fn get_unchecked_mut(
        &mut self,
        addr: usize,
    ) -> (Headroom<(*mut u8, usize)>, Data<(*mut u8, usize)>) {
        let (h, d) = unsafe { self.frame_pointers(addr) };

        (
            Headroom((h.0 as *mut u8, self.layout.frame_headroom)),
            Data((d.0 as *mut u8, self.layout.data_size)),
        )
    }

    #[inline]
    fn calculate_base_addr(&self, addr: usize) -> usize {
        (addr / self.frame_size) * self.frame_size
    }

    /// # Safety
    ///
    /// `addr` must be within the [`Mmap`] region.
    #[inline]
    unsafe fn frame_pointers(
        &self,
        addr: usize,
    ) -> (Headroom<*mut libc::c_void>, Data<*mut libc::c_void>) {
        let base_addr = self.calculate_base_addr(addr);

        let headroom_offset = base_addr + self.layout.xdp_headroom;
        let data_offset = headroom_offset + self.layout.frame_headroom;

        let headroom_addr = unsafe { self.mmap.addr.as_ptr().add(headroom_offset) };
        let data_addr = unsafe { self.mmap.addr.as_ptr().add(data_offset) };

        (Headroom(headroom_addr), Data(data_addr))
    }
}

#[cfg(test)]
mod tests {
    use core::slice;
    use std::io::Write;

    use libbpf_sys::xdp_desc;

    use crate::umem::Frame;

    use super::*;

    #[test]
    fn check_writes_persist() {
        let layout = FrameLayout {
            xdp_headroom: 0,
            frame_headroom: 512,
            data_size: 2048,
        };

        let frame_size = layout.frame_headroom + layout.data_size;

        let mmap = Mmap::new(16 * frame_size, false).unwrap();

        let addr = mmap.addr;

        let framed_mmap = FramedMmap::new(layout, Arc::new(mmap)).unwrap();

        let mut frame_0 = Frame::new(0 * frame_size + layout.frame_headroom, framed_mmap.clone());
        let mut frame_1 = Frame::new(1 * frame_size + layout.frame_headroom, framed_mmap.clone());

        let mut desc = xdp_desc::default();

        unsafe { frame_0.data_mut() }
            .cursor()
            .write_all(b"hello")
            .unwrap();

        frame_0.write_xdp_desc(&mut desc);

        assert_eq!(desc.addr, (0 * frame_size + layout.frame_headroom) as u64);
        assert_eq!(desc.len, 5);
        assert_eq!(desc.options, 0);

        unsafe { frame_1.data_mut() }
            .cursor()
            .write_all(b"world!")
            .unwrap();

        frame_1.write_xdp_desc(&mut desc);

        assert_eq!(desc.addr, (1 * frame_size + layout.frame_headroom) as u64);
        assert_eq!(desc.len, 6);
        assert_eq!(desc.options, 0);

        assert_eq!(
            unsafe {
                slice::from_raw_parts(
                    addr.as_ptr().add(0 * frame_size + layout.frame_headroom) as *const u8,
                    5,
                )
            },
            b"hello"
        );

        assert_eq!(
            unsafe {
                slice::from_raw_parts(
                    addr.as_ptr().add(1 * frame_size + layout.frame_headroom) as *const u8,
                    6,
                )
            },
            b"world!"
        );
    }
}