//! Types for representing and working with a [`Umem`](super::Umem)
//! frame.

mod cursor;
pub use cursor::Cursor;

use core::slice;
use std::{borrow::Borrow, ops::Deref};

use super::mmap::framed::FramedMmap;

/// A [`Umem`](super::Umem) frame descriptor.
///
/// Used to pass frame information between the kernel and
/// userspace. The `addr` field is an offset in bytes from the start
/// of the [`Umem`](super::Umem) and corresponds to some point within a frame. The
/// `len` field describes the length (in bytes) of any data stored in
/// that frame, starting from `addr`.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct FrameDesc {
    pub addr: usize,
    pub len: usize,
    pub options: u32,
}

/// The saved cursor positions for a frame's data and headroom
/// segments. Used to keep track of positions between writes.
#[derive(Debug, Default, Clone, Copy)]
struct CursorPos {
    headroom: usize,
    data: usize,
}

/// A single frame of some [`Umem`](super::Umem).
pub struct Frame {
    addr: usize,
    cursor_pos: CursorPos,
    options: u32,
    frames: FramedMmap,
}

impl Frame {
    pub(super) fn new(addr: usize, frames: FramedMmap) -> Self {
        Self {
            addr,
            cursor_pos: CursorPos::default(),
            options: 0,
            frames,
        }
    }

    /// The frame's address. This address is the start of the data segment.
    #[inline]
    pub fn addr(&self) -> usize {
        self.addr
    }

    /// The current length of the data segment.
    #[inline]
    pub fn len(&self) -> usize {
        self.cursor_pos.data
    }

    #[inline]
    pub fn options(&self) -> u32 {
        self.options
    }

    #[inline]
    pub fn set_options(&mut self, options: u32) {
        self.options = options
    }

    /// The frame's headroom and data segments.
    ///
    /// # Safety
    ///
    /// The underlying [`Umem`](super::Umem) region this frame
    /// accesses must not be mutably accessed anywhere else at the
    /// same time, either in userspace or by the kernel.
    #[inline]
    pub unsafe fn get(&self) -> (Headroom, Data) {
        let (h, d) = unsafe { self.frames.get_unchecked(self.addr) };

        (
            Headroom {
                buf: unsafe { &slice::from_raw_parts(h.0 .0, h.0 .1)[..self.cursor_pos.headroom] },
            },
            Data {
                buf: unsafe { &slice::from_raw_parts(d.0 .0, d.0 .1)[..self.cursor_pos.data] },
            },
        )
    }

    /// The frame's headroom segment.
    ///
    /// # Safety
    ///
    /// See [`get`](Frame::get).
    #[inline]
    pub unsafe fn headroom(&self) -> Headroom {
        let (h, _d) = unsafe { self.frames.get_unchecked(self.addr) };

        Headroom {
            buf: unsafe { &slice::from_raw_parts(h.0 .0, h.0 .1)[..self.cursor_pos.headroom] },
        }
    }

    /// The frame's data segment
    ///
    /// # Safety
    ///
    /// See [`get`](Frame::get).
    #[inline]
    pub unsafe fn data(&self) -> Data {
        let (_h, d) = unsafe { self.frames.get_unchecked(self.addr) };

        Data {
            buf: unsafe { &slice::from_raw_parts(d.0 .0, d.0 .1)[..self.cursor_pos.data] },
        }
    }

    /// Mutable references to the frame's headroom and data segments.
    ///
    /// # Safety
    ///
    /// The underlying [`Umem`](super::Umem) region this frame
    /// accesses must not be mutably or immutably accessed anywhere
    /// else at the same time, either in userspace or by the kernel.
    #[inline]
    pub unsafe fn get_mut(&mut self) -> (HeadroomMut, DataMut) {
        let (h, d) = unsafe { self.frames.get_unchecked_mut(self.addr) };

        (
            HeadroomMut {
                pos: &mut self.cursor_pos.headroom,
                buf: unsafe { slice::from_raw_parts_mut(h.0 .0, h.0 .1) },
            },
            DataMut {
                pos: &mut self.cursor_pos.data,
                buf: unsafe { slice::from_raw_parts_mut(d.0 .0, d.0 .1) },
            },
        )
    }

    /// A mutable reference to the frame's headroom segment.
    ///
    /// # Safety
    ///
    /// See [`get_mut`](Frame::get_mut).
    #[inline]
    pub unsafe fn headroom_mut(&mut self) -> HeadroomMut {
        let (h, _d) = unsafe { self.frames.get_unchecked_mut(self.addr) };

        HeadroomMut {
            pos: &mut self.cursor_pos.headroom,
            buf: unsafe { slice::from_raw_parts_mut(h.0 .0, h.0 .1) },
        }
    }

    /// A mutable reference to the frame's data segment.
    ///
    /// # Safety
    ///
    /// See [`get_mut`](Frame::get_mut).
    #[inline]
    pub unsafe fn data_mut(&mut self) -> DataMut {
        let (_h, d) = unsafe { self.frames.get_unchecked_mut(self.addr) };

        DataMut {
            pos: &mut self.cursor_pos.data,
            buf: unsafe { slice::from_raw_parts_mut(d.0 .0, d.0 .1) },
        }
    }

    /// # Safety
    ///
    /// The address in `desc` must belong to the same underlying
    /// [`Umem`](super::Umem) as this frame.
    #[inline]
    pub(crate) unsafe fn set_desc(&mut self, desc: &FrameDesc) {
        self.addr = desc.addr;
        self.options = desc.options;
        self.cursor_pos.headroom = 0;
        self.cursor_pos.data = desc.len;
    }

    #[inline]
    pub(crate) fn write_xdp_desc(&self, desc: &mut libbpf_sys::xdp_desc) {
        desc.addr = self.addr as u64;
        desc.options = self.options;
        desc.len = self.cursor_pos.data as u32;
    }
}

/// Headroom segment of a [`Umem`](crate::umem::Umem) frame.
#[derive(Debug)]
pub struct Headroom<'umem> {
    buf: &'umem [u8],
}

impl Headroom<'_> {
    #[inline]
    pub fn contents(&self) -> &[u8] {
        self.buf
    }
}

impl AsRef<[u8]> for Headroom<'_> {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.buf
    }
}

impl Borrow<[u8]> for Headroom<'_> {
    #[inline]
    fn borrow(&self) -> &[u8] {
        self.buf
    }
}

impl Deref for Headroom<'_> {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.buf
    }
}

/// Mutable headroom segment of a [`Umem`](crate::umem::Umem) frame
/// that allows writing via its [`cursor`](HeadroomMut::cursor)
/// method.
#[derive(Debug)]
pub struct HeadroomMut<'umem> {
    pos: &'umem mut usize,
    buf: &'umem mut [u8],
}

impl<'umem> HeadroomMut<'umem> {
    #[inline]
    pub fn contents(&self) -> &[u8] {
        self.buf
    }

    #[inline]
    pub fn cursor(&'umem mut self) -> Cursor<'umem> {
        Cursor::new(self.pos, self.buf)
    }
}

impl AsRef<[u8]> for HeadroomMut<'_> {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.buf
    }
}

impl Borrow<[u8]> for HeadroomMut<'_> {
    #[inline]
    fn borrow(&self) -> &[u8] {
        self.buf
    }
}

impl Deref for HeadroomMut<'_> {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.buf
    }
}

/// Data segment of a [`Umem`](crate::umem::Umem) frame.
#[derive(Debug)]
pub struct Data<'umem> {
    buf: &'umem [u8],
}

impl Data<'_> {
    #[inline]
    pub fn contents(&self) -> &[u8] {
        self.buf
    }
}

impl AsRef<[u8]> for Data<'_> {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.buf
    }
}

impl Borrow<[u8]> for Data<'_> {
    #[inline]
    fn borrow(&self) -> &[u8] {
        self.buf
    }
}

impl Deref for Data<'_> {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.buf
    }
}

/// Mutable data segment of a [`Umem`](crate::umem::Umem) frame that
/// allows writing via its [`cursor`](DataMut::cursor) method.
#[derive(Debug)]
pub struct DataMut<'umem> {
    pos: &'umem mut usize,
    buf: &'umem mut [u8],
}

impl<'umem> DataMut<'umem> {
    #[inline]
    pub fn contents(&self) -> &[u8] {
        self.buf
    }

    #[inline]
    pub fn cursor(&'umem mut self) -> Cursor<'umem> {
        Cursor::new(self.pos, self.buf)
    }
}

impl AsRef<[u8]> for DataMut<'_> {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.buf
    }
}

impl Borrow<[u8]> for DataMut<'_> {
    #[inline]
    fn borrow(&self) -> &[u8] {
        self.buf
    }
}

impl Deref for DataMut<'_> {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.buf
    }
}
