//! Types for representing and working with a [`Umem`](super::Umem)
//! frame.

mod cursor;
pub use cursor::Cursor;

use core::slice;
use std::{borrow::Borrow, cmp, ops::Deref};

use super::mmap::framed::FramedMmap;

/// A [`Umem`](super::Umem) frame descriptor.
///
/// Used to pass frame information between the kernel and
/// userspace. The `addr` field is an offset in bytes from the start
/// of the [`Umem`](super::Umem) and corresponds to the starting
/// address of the data segment of some frame. The `len` field
/// describes the length (in bytes) of any data stored in that frame,
/// starting from `addr`.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct FrameDesc {
    pub addr: usize,
    pub len: usize,
    pub options: u32,
}

/// The lengths of a frame's data and headroom segments.
#[derive(Debug, Default, Clone, Copy)]
struct SegmentLengths {
    headroom: usize,
    data: usize,
}

/// A single frame of some [`Umem`](super::Umem).
pub struct Frame {
    addr: usize,
    mtu: usize,
    lens: SegmentLengths,
    options: u32,
    framed_mmap: FramedMmap,
}

impl Frame {
    /// # Safety
    ///
    /// `addr` must be the starting address of the data segment of
    /// some frame belonging to `framed_mmap`.
    pub(super) unsafe fn new(addr: usize, mtu: usize, framed_mmap: FramedMmap) -> Self {
        Self {
            addr,
            mtu,
            lens: SegmentLengths::default(),
            options: 0,
            framed_mmap,
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
        cmp::min(self.lens.data, self.mtu)
    }

    /// Returns `true` if the length of the data segment (i.e. what
    /// was received from the kernel or will be transmitted to) is
    /// zero.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.lens.data == 0
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
        let (h, d) = unsafe { self.framed_mmap.get_unchecked(self.addr) };

        let headroom_len = cmp::min(self.lens.headroom, h.len);
        let data_len = cmp::min(self.lens.data, d.len);

        (
            Headroom {
                contents: unsafe { &slice::from_raw_parts(h.addr, headroom_len) },
            },
            Data {
                contents: unsafe { &slice::from_raw_parts(d.addr, data_len) },
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
        let (h, _d) = unsafe { self.framed_mmap.get_unchecked(self.addr) };

        let headroom_len = cmp::min(self.lens.headroom, h.len);

        Headroom {
            contents: unsafe { &slice::from_raw_parts(h.addr, headroom_len) },
        }
    }

    /// The frame's data segment
    ///
    /// # Safety
    ///
    /// See [`get`](Frame::get).
    #[inline]
    pub unsafe fn data(&self) -> Data {
        let (_h, d) = unsafe { self.framed_mmap.get_unchecked(self.addr) };

        let data_len = cmp::min(self.lens.data, d.len);

        Data {
            contents: unsafe { &slice::from_raw_parts(d.addr, data_len) },
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
        let (h, d) = unsafe { self.framed_mmap.get_unchecked_mut(self.addr) };

        (
            HeadroomMut {
                len: &mut self.lens.headroom,
                buf: unsafe { slice::from_raw_parts_mut(h.addr, h.len) },
            },
            DataMut {
                len: &mut self.lens.data,
                buf: unsafe { slice::from_raw_parts_mut(d.addr, d.len) },
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
        let (h, _d) = unsafe { self.framed_mmap.get_unchecked_mut(self.addr) };

        HeadroomMut {
            len: &mut self.lens.headroom,
            buf: unsafe { slice::from_raw_parts_mut(h.addr, h.len) },
        }
    }

    /// A mutable reference to the frame's data segment.
    ///
    /// # Safety
    ///
    /// See [`get_mut`](Frame::get_mut).
    #[inline]
    pub unsafe fn data_mut(&mut self) -> DataMut {
        let (_h, d) = unsafe { self.framed_mmap.get_unchecked_mut(self.addr) };

        DataMut {
            len: &mut self.lens.data,
            buf: unsafe { slice::from_raw_parts_mut(d.addr, d.len) },
        }
    }

    /// # Safety
    ///
    /// The address in `desc` must be the starting address of the
    /// data segment of a frame belonging to the same underlying
    /// [`Umem`](super::Umem) as this frame.
    #[inline]
    pub(crate) unsafe fn set_desc(&mut self, desc: &FrameDesc) {
        self.addr = desc.addr;
        self.options = desc.options;
        self.lens.data = desc.len; // Leave the headroom cursor position where it is
    }

    /// # Safety
    ///
    /// The provided `desc` must ultimately be passed to a ring that
    /// is associated with the same underlying [`Umem`](super::Umem)
    /// as this frame.
    #[inline]
    pub(crate) unsafe fn write_xdp_desc(&self, desc: &mut libbpf_sys::xdp_desc) {
        desc.addr = self.addr as u64;
        desc.options = self.options;
        desc.len = self.len() as u32;
    }
}

/// Headroom segment of a [`Umem`](crate::umem::Umem) frame.
#[derive(Debug)]
pub struct Headroom<'umem> {
    contents: &'umem [u8],
}

impl Headroom<'_> {
    /// Returns this headroom segment's contents, up to the current
    /// cursor position.
    ///
    /// Note that headroom cursor position isn't reset in between
    /// updates to the frame's descriptor. So, for example, if you write
    /// to this headroom and then transmit its frame, if you then use
    /// the same frame for receiving a packet then the headroom
    /// contents will be the same.
    #[inline]
    pub fn contents(&self) -> &[u8] {
        self.contents
    }
}

impl AsRef<[u8]> for Headroom<'_> {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.contents
    }
}

impl Borrow<[u8]> for Headroom<'_> {
    #[inline]
    fn borrow(&self) -> &[u8] {
        self.contents
    }
}

impl Deref for Headroom<'_> {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.contents
    }
}

/// Mutable headroom segment of a [`Umem`](crate::umem::Umem) frame
/// that allows writing via its [`cursor`](HeadroomMut::cursor)
/// method.
#[derive(Debug)]
pub struct HeadroomMut<'umem> {
    len: &'umem mut usize,
    buf: &'umem mut [u8],
}

impl<'umem> HeadroomMut<'umem> {
    /// Returns this headroom segment's contents, up to the current
    /// cursor position.
    ///
    /// Note that headroom cursor position isn't reset in between
    /// updates to the frame's descriptor. So, for example, if you write
    /// to this headroom and then transmit its frame, if you then use
    /// the same frame for receiving a packet then the headroom
    /// contents will be the same.
    #[inline]
    pub fn contents(&self) -> &[u8] {
        let len = cmp::min(*self.len, self.buf.len());
        &self.buf[..len]
    }

    /// A cursor for writing to the underlying memory.
    #[inline]
    pub fn cursor(&'umem mut self) -> Cursor<'umem> {
        Cursor::new(self.len, self.buf)
    }

    /// Returns a reference to this segment's underlying memory and a
    /// `usize` which is the length of whatever data is currently
    /// contained in this segment.
    ///
    /// If you write to this slice then you must also update the
    /// accompanying `usize` accordingly. You can use the
    /// [`cursor`](HeadroomMut::cursor) method to avoid having to do
    /// this manually.
    #[inline]
    pub fn buf_and_len(&'umem mut self) -> (&mut [u8], &mut usize) {
        (&mut self.buf, &mut self.len)
    }
}

impl AsRef<[u8]> for HeadroomMut<'_> {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.contents()
    }
}

impl Borrow<[u8]> for HeadroomMut<'_> {
    #[inline]
    fn borrow(&self) -> &[u8] {
        self.contents()
    }
}

impl Deref for HeadroomMut<'_> {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.contents()
    }
}

/// Data segment of a [`Umem`](crate::umem::Umem) frame.
#[derive(Debug)]
pub struct Data<'umem> {
    contents: &'umem [u8],
}

impl Data<'_> {
    /// Returns this data segment's contents, up to the current
    /// cursor position.
    #[inline]
    pub fn contents(&self) -> &[u8] {
        self.contents
    }
}

impl AsRef<[u8]> for Data<'_> {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.contents
    }
}

impl Borrow<[u8]> for Data<'_> {
    #[inline]
    fn borrow(&self) -> &[u8] {
        self.contents
    }
}

impl Deref for Data<'_> {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.contents
    }
}

/// Mutable data segment of a [`Umem`](crate::umem::Umem) frame that
/// allows writing via its [`cursor`](DataMut::cursor) method.
#[derive(Debug)]
pub struct DataMut<'umem> {
    len: &'umem mut usize,
    buf: &'umem mut [u8],
}

impl<'umem> DataMut<'umem> {
    /// Returns this data segment's contents, up to the current
    /// cursor position.
    #[inline]
    pub fn contents(&self) -> &[u8] {
        let len = cmp::min(*self.len, self.buf.len());
        &self.buf[..len]
    }

    /// A cursor for writing to the underlying memory.
    #[inline]
    pub fn cursor(&'umem mut self) -> Cursor<'umem> {
        Cursor::new(self.len, self.buf)
    }

    /// Returns a reference to this segment's underlying memory and a
    /// `usize` which is the length of whatever data is currently
    /// contained in this segment.
    ///
    /// If you write to this slice then you must also update the
    /// accompanying `usize` accordingly. You can use the
    /// [`cursor`](DataMut::cursor) method to avoid having to do
    /// this manually.
    #[inline]
    pub fn buf_and_len(&'umem mut self) -> (&mut [u8], &mut usize) {
        (&mut self.buf, &mut self.len)
    }
}

impl AsRef<[u8]> for DataMut<'_> {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.contents()
    }
}

impl Borrow<[u8]> for DataMut<'_> {
    #[inline]
    fn borrow(&self) -> &[u8] {
        self.contents()
    }
}

impl Deref for DataMut<'_> {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.contents()
    }
}
