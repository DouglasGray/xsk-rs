use std::{
    io::{self, ErrorKind},
    sync::Arc,
};

use super::Mmap;

#[derive(Debug)]
pub struct Headroom<T> {
    pub addr: T,
    pub len: usize,
}

impl Headroom<*const u8> {
    fn new(ptr: HeadroomPtr, len: usize) -> Self {
        Self {
            addr: ptr.0 as *const u8,
            len,
        }
    }
}

impl Headroom<*mut u8> {
    fn new(ptr: HeadroomPtr, len: usize) -> Self {
        Self {
            addr: ptr.0 as *mut u8,
            len,
        }
    }
}

#[derive(Debug)]
pub struct Data<T> {
    pub addr: T,
    pub len: usize,
}

impl Data<*const u8> {
    fn new(ptr: DataPtr, len: usize) -> Self {
        Self {
            addr: ptr.0 as *const u8,
            len,
        }
    }
}

impl Data<*mut u8> {
    fn new(ptr: DataPtr, len: usize) -> Self {
        Self {
            addr: ptr.0 as *mut u8,
            len,
        }
    }
}

struct HeadroomPtr(pub *mut u8);

struct DataPtr(pub *mut u8);

/// Dimensions of a [`Umem`](crate::umem::Umem) frame.
#[derive(Debug, Clone, Copy)]
pub struct FrameLayout {
    pub xdp_headroom: usize,
    pub frame_headroom: usize,
    pub mtu: usize,
}

impl FrameLayout {
    #[inline]
    pub fn frame_size(&self) -> usize {
        self.xdp_headroom + self.frame_headroom + self.mtu
    }
}

/// A [`Mmap`] chunked into frames of size `frame_size`.
#[derive(Clone)]
pub struct FramedMmap {
    layout: FrameLayout,
    mmap: Arc<Mmap>,
}

impl FramedMmap {
    pub fn new(layout: FrameLayout, mmap: Arc<Mmap>) -> io::Result<Self> {
        let frame_size = layout.frame_size();

        if frame_size == 0 || mmap.len() % frame_size != 0 {
            Err(io::Error::new(
                ErrorKind::Other,
                format!(
                    "mmap with len {} cannot be split exactly into frames of size {}",
                    mmap.len(),
                    frame_size
                ),
            ))
        } else {
            Ok(FramedMmap { layout, mmap })
        }
    }

    /// Retrieve a immutable pointer and length pair which describe
    /// the frame's headroom and data segments respectively.
    ///
    /// # Safety
    ///
    /// `data_addr` must be the starting address of the data segment
    /// of a frame within this [`Mmap`] region.
    #[inline]
    pub unsafe fn get_unchecked(&self, data_addr: usize) -> (Headroom<*const u8>, Data<*const u8>) {
        let (h, d) = unsafe { self.frame_pointers(data_addr) };

        (
            Headroom::<*const u8>::new(h, self.layout.frame_headroom),
            Data::<*const u8>::new(d, self.layout.mtu),
        )
    }

    /// Retrieve a mutable pointer and length pair which describe
    /// the frame's headroom and data segments respectively.
    ///
    /// # Safety
    ///
    /// `data_addr` must be the starting address of the data segment
    /// of a frame within this [`Mmap`] region.
    #[inline]
    pub unsafe fn get_unchecked_mut(
        &mut self,
        data_addr: usize,
    ) -> (Headroom<*mut u8>, Data<*mut u8>) {
        let (h, d) = unsafe { self.frame_pointers(data_addr) };

        (
            Headroom::<*mut u8>::new(h, self.layout.frame_headroom),
            Data::<*mut u8>::new(d, self.layout.mtu),
        )
    }

    /// # Safety
    ///
    /// `data_addr` must be the starting address of the data segment
    /// of a frame within this [`Mmap`] region.
    #[inline]
    unsafe fn frame_pointers(&self, data_addr: usize) -> (HeadroomPtr, DataPtr) {
        let headroom = unsafe { self.mmap.addr().add(data_addr - self.layout.frame_headroom) };
        let data = unsafe { self.mmap.addr().add(data_addr) };

        (HeadroomPtr(headroom), DataPtr(data))
    }
}

#[cfg(test)]
mod tests {
    use core::slice;
    use std::io::Write;

    use libbpf_sys::xdp_desc;

    use crate::umem::{Frame, Mmap};

    use super::*;

    #[test]
    fn check_writes_persist() {
        let layout = FrameLayout {
            xdp_headroom: 0,
            frame_headroom: 512,
            mtu: 2048,
        };

        let frame_size = layout.frame_size();

        let mmap = Mmap::new(16 * frame_size, false).unwrap();

        // Take a copy of the base addr
        let addr = mmap.addr();

        let framed_mmap = FramedMmap::new(layout, Arc::new(mmap)).unwrap();

        let mut frame_0 = unsafe {
            Frame::new(
                0 * frame_size + layout.frame_headroom,
                layout.mtu,
                framed_mmap.clone(),
            )
        };

        let mut frame_1 = unsafe {
            Frame::new(
                1 * frame_size + layout.frame_headroom,
                layout.mtu,
                framed_mmap.clone(),
            )
        };

        let mut desc = xdp_desc::default();

        unsafe { frame_0.data_mut() }
            .cursor()
            .write_all(b"hello")
            .unwrap();

        unsafe {
            frame_0.write_xdp_desc(&mut desc);
        }

        assert_eq!(desc.addr, (0 * frame_size + layout.frame_headroom) as u64);
        assert_eq!(desc.len, 5);
        assert_eq!(desc.options, 0);

        unsafe { frame_1.data_mut() }
            .cursor()
            .write_all(b"world!")
            .unwrap();

        unsafe {
            frame_1.write_xdp_desc(&mut desc);
        }

        assert_eq!(desc.addr, (1 * frame_size + layout.frame_headroom) as u64);
        assert_eq!(desc.len, 6);
        assert_eq!(desc.options, 0);

        assert_eq!(
            unsafe {
                slice::from_raw_parts(
                    addr.add(0 * frame_size + layout.frame_headroom) as *const u8,
                    5,
                )
            },
            b"hello"
        );

        assert_eq!(
            unsafe {
                slice::from_raw_parts(
                    addr.add(1 * frame_size + layout.frame_headroom) as *const u8,
                    6,
                )
            },
            b"world!"
        );
    }

    #[test]
    fn check_writes_are_contiguous() {
        let layout = FrameLayout {
            xdp_headroom: 4,
            frame_headroom: 8,
            mtu: 12,
        };

        let frame_count = 4;

        // An arbitrary layout
        let xdp_headroom_segment = [0, 0, 0, 0];
        let frame_headroom_segment = [1, 1, 1, 1, 1, 1, 1, 1];
        let data_segment = [2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2];

        let mut cursor = io::Cursor::new(Vec::new());

        cursor.write_all(&xdp_headroom_segment).unwrap();
        cursor.write_all(&frame_headroom_segment).unwrap();
        cursor.write_all(&data_segment).unwrap();

        let base_layout: Vec<u8> = cursor.into_inner();

        let expected_layout: Vec<u8> = (0..frame_count as u8)
            .into_iter()
            .map(|i| {
                base_layout
                    .iter()
                    .map(|el| el * (i + 1))
                    .collect::<Vec<_>>()
            })
            .flatten()
            .collect();

        // Create framed mmap and frames and write some data to them
        let frame_size = layout.frame_size();

        let mmap = Arc::new(Mmap::new(frame_count * frame_size, false).unwrap());

        let framed_mmap = FramedMmap::new(layout, Arc::clone(&mmap)).unwrap();

        (0..frame_count).into_iter().for_each(|i| {
            let mut frame = unsafe {
                Frame::new(
                    (i * frame_size) + layout.xdp_headroom + layout.frame_headroom,
                    layout.mtu,
                    framed_mmap.clone(),
                )
            };

            let (mut headroom, mut data) = unsafe { frame.segments_mut() };

            headroom
                .cursor()
                .write_all(
                    &frame_headroom_segment
                        .iter()
                        .map(|el| el * (i as u8 + 1))
                        .collect::<Vec<_>>(),
                )
                .unwrap();

            data.cursor()
                .write_all(
                    &data_segment
                        .iter()
                        .map(|el| el * (i as u8 + 1))
                        .collect::<Vec<_>>(),
                )
                .unwrap();
        });

        // Check they match
        let mmap_region = unsafe { slice::from_raw_parts(mmap.addr(), mmap.len()) };

        assert_eq!(mmap_region, expected_layout)
    }
}
