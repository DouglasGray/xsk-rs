//! Types for representing and working with a [`Umem`](super::Umem)
//! frame.

mod cursor;
pub use cursor::Cursor;

use core::slice;
use std::{borrow::Borrow, fmt, ops::Deref};

use super::{mmap::Mmap, FrameLayout};

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

/// The lengths of a frame's packet data and headroom segments.
#[derive(Debug, Default, Clone, Copy)]
struct SegmentLengths {
    headroom: usize,
    data: usize,
}

/// A single frame of some [`Umem`](super::Umem).
pub struct Frame {
    addr: usize,
    options: u32,
    lens: SegmentLengths,
    layout: FrameLayout,
    umem: Mmap,
}

impl Frame {
    /// Create a new frame which belongs to `umem`, whose packet data
    /// segment starts at `addr` and with a layout described by
    /// `frame_layout.`
    ///
    /// # Safety
    ///
    /// `addr` must be the starting address of the packet data segment
    /// of some frame belonging to `umem`.
    pub(super) unsafe fn new(addr: usize, frame_layout: FrameLayout, umem: Mmap) -> Self {
        Self {
            addr,
            options: 0,
            lens: SegmentLengths::default(),
            layout: frame_layout,
            umem,
        }
    }

    /// The starting address of this frame's packet data segment.
    #[inline]
    pub fn addr(&self) -> usize {
        self.addr
    }

    /// The current length of this frame's packet data segment.
    #[inline]
    pub fn len(&self) -> usize {
        self.lens.data
    }

    /// Returns `true` if the length of the packet data segment
    /// (i.e. what was received from the kernel or will be
    /// transmitted) is zero.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.lens.data == 0
    }

    /// Frame options.
    #[inline]
    pub fn options(&self) -> u32 {
        self.options
    }

    /// Set the frame options. Will be included in the descriptor
    /// passed to the kernel, along with the frame address and length.
    #[inline]
    pub fn set_options(&mut self, options: u32) {
        self.options = options
    }

    #[inline]
    /// Retrieve a pointer to the headroom segment of this frame.
    ///
    /// # Safety
    ///
    /// This frame's current `addr` must point to the start of a
    /// packet data segment of a frame comprising the underlying UMEM.
    unsafe fn headroom_ptr(&self) -> *mut u8 {
        let addr = self.addr - self.layout.frame_headroom;
        unsafe { self.umem.offset(addr) as *mut u8 }
    }

    #[inline]
    /// Retrieve a pointer to the packet data segment of this frame.
    ///
    /// # Safety
    ///
    /// This frame's current `addr` must point to the start of a
    /// packet data segment of a frame comprising the underlying UMEM.
    unsafe fn data_ptr(&self) -> *mut u8 {
        unsafe { self.umem.offset(self.addr) as *mut u8 }
    }

    /// The frame's headroom and packet data segments.
    ///
    /// # Safety
    ///
    /// The underlying [`Umem`](super::Umem) region this frame
    /// accesses must not be mutably accessed anywhere else at the
    /// same time, either in userspace or by the kernel.
    #[inline]
    pub unsafe fn segments(&self) -> (Headroom, Data) {
        // SAFETY: unsafe contract in constructor and `set_desc`
        // ensures that this frame's current `addr` points to the
        // start of a packet data segment in its underlying UMEM -
        // therefore the dereferenced slices are whole and valid
        // segments.
        //
        // The unsafe contract of this function also guarantees there
        // are no other mutable references to these slices at the same
        // time.
        unsafe { (self.headroom(), self.data()) }
    }

    /// The frame's headroom segment.
    ///
    /// # Safety
    ///
    /// See [`get`](Frame::segments).
    #[inline]
    pub unsafe fn headroom(&self) -> Headroom {
        // SAFETY: see `segments`.
        let headroom_ptr = unsafe { self.headroom_ptr() };

        Headroom {
            contents: unsafe { slice::from_raw_parts(headroom_ptr, self.lens.headroom) },
        }
    }

    /// The frame's packet data segment
    ///
    /// # Safety
    ///
    /// See [`get`](Frame::segments).
    #[inline]
    pub unsafe fn data(&self) -> Data {
        // SAFETY: see `segments`.
        let data_ptr = unsafe { self.data_ptr() };

        Data {
            contents: unsafe { slice::from_raw_parts(data_ptr, self.lens.data) },
        }
    }

    /// Mutable references to the frame's headroom and packet data
    /// segments.
    ///
    /// # Safety
    ///
    /// The underlying [`Umem`](super::Umem) region this frame
    /// accesses must not be mutably or immutably accessed anywhere
    /// else at the same time, either in userspace or by the kernel.
    #[inline]
    pub unsafe fn segments_mut(&mut self) -> (HeadroomMut, DataMut) {
        // SAFETY: unsafe contract in constructor and `set_desc`
        // ensures that this frame's current `addr` points to the
        // start of a packet data segment in its underlying UMEM -
        // therefore the dereferenced slices are whole and valid
        // segments.
        //
        // The unsafe contract of this function also guarantees there
        // are no other mutable or immutable references to these
        // slices at the same time.
        let headroom_ptr = unsafe { self.headroom_ptr() };
        let data_ptr = unsafe { self.data_ptr() };

        (
            HeadroomMut {
                len: &mut self.lens.headroom,
                buf: unsafe { slice::from_raw_parts_mut(headroom_ptr, self.layout.frame_headroom) },
            },
            DataMut {
                len: &mut self.lens.data,
                buf: unsafe { slice::from_raw_parts_mut(data_ptr, self.layout.mtu) },
            },
        )
    }

    /// A mutable reference to the frame's headroom segment.
    ///
    /// # Safety
    ///
    /// See [`get_mut`](Frame::segments_mut).
    #[inline]
    pub unsafe fn headroom_mut(&mut self) -> HeadroomMut {
        // SAFETY: see `segments_mut`.
        let headroom_ptr = unsafe { self.headroom_ptr() };

        HeadroomMut {
            len: &mut self.lens.headroom,
            buf: unsafe { slice::from_raw_parts_mut(headroom_ptr, self.layout.frame_headroom) },
        }
    }

    /// A mutable reference to the frame's packet data segment.
    ///
    /// # Safety
    ///
    /// See [`get_mut`](Frame::segments_mut).
    #[inline]
    pub unsafe fn data_mut(&mut self) -> DataMut {
        // SAFETY: see `segments_mut`.
        let data_ptr = unsafe { self.data_ptr() };

        DataMut {
            len: &mut self.lens.data,
            buf: unsafe { slice::from_raw_parts_mut(data_ptr, self.layout.mtu) },
        }
    }

    /// # Safety
    ///
    /// The address in `desc` must be the starting address of the
    /// packet data segment of a frame belonging to the same
    /// underlying [`Umem`](super::Umem) as this frame.
    #[inline]
    pub(crate) unsafe fn set_desc(&mut self, desc: &FrameDesc) {
        self.addr = desc.addr;
        self.options = desc.options;
        self.lens.data = desc.len;
        // Leave the headroom cursor position where it is
    }

    /// # Safety
    ///
    /// The provided `desc` must ultimately be passed to a ring that
    /// is tied to the same underlying [`Umem`](super::Umem) as this
    /// frame.
    #[inline]
    pub(crate) unsafe fn write_xdp_desc(&self, desc: &mut libbpf_sys::xdp_desc) {
        desc.addr = self.addr() as u64;
        desc.options = self.options();
        desc.len = self.len() as u32;
    }
}

impl fmt::Debug for Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Frame")
            .field("addr", &self.addr)
            .field("mtu", &self.layout)
            .field("lens", &self.lens)
            .field("options", &self.options)
            .finish()
    }
}

/// Headroom segment of a [`Umem`](crate::umem::Umem) frame.
#[derive(Debug)]
pub struct Headroom<'umem> {
    contents: &'umem [u8],
}

impl Headroom<'_> {
    /// Returns this segment's contents, up to its current length.
    ///
    /// Note that headroom length isn't changed in between updates to
    /// the frame's descriptor. So, for example, if you write to this
    /// headroom and then transmit its frame, if you then use the same
    /// frame for receiving a packet then the headroom contents will
    /// be the same.
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

impl HeadroomMut<'_> {
    /// Returns this segment's contents, up to its current length.
    ///
    /// Note that headroom length isn't changed in between updates to
    /// the frame's descriptor. So, for example, if you write to this
    /// headroom and then transmit its frame, if you then use the same
    /// frame for receiving a packet then the headroom contents will
    /// be the same.
    #[inline]
    pub fn contents(&self) -> &[u8] {
        &self.buf[..*self.len]
    }

    /// A cursor for writing to the underlying memory.
    #[inline]
    pub fn cursor(&mut self) -> Cursor<'_> {
        Cursor::new(self.len, self.buf)
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

/// Packet data segment of a [`Umem`](crate::umem::Umem) frame.
#[derive(Debug)]
pub struct Data<'umem> {
    contents: &'umem [u8],
}

impl Data<'_> {
    /// Returns this segment's contents, up to its current length.
    ///
    /// Will change as packets are sent or received using this frame.
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

impl DataMut<'_> {
    /// Returns this segment's contents, up to its current length.
    ///
    /// Will change as packets are sent or received using this frame.
    #[inline]
    pub fn contents(&self) -> &[u8] {
        &self.buf[..*self.len]
    }

    /// A cursor for writing to the underlying memory.
    #[inline]
    pub fn cursor(&mut self) -> Cursor<'_> {
        Cursor::new(self.len, self.buf)
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

#[cfg(test)]
mod tests {
    use core::slice;
    use std::io::{self, Write};

    use libbpf_sys::xdp_desc;

    use crate::umem::{Frame, Mmap};

    use super::*;

    #[test]
    fn writes_persist() {
        let layout = FrameLayout {
            _xdp_headroom: 0,
            frame_headroom: 512,
            mtu: 2048,
        };

        let frame_size = layout.frame_size();

        let mmap = Mmap::new(16 * frame_size, false).unwrap();

        let mut frame_0 =
            unsafe { Frame::new(0 * frame_size + layout.frame_headroom, layout, mmap.clone()) };

        let mut frame_1 =
            unsafe { Frame::new(1 * frame_size + layout.frame_headroom, layout, mmap.clone()) };

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
                    mmap.offset(0 * frame_size + layout.frame_headroom) as *const u8,
                    5,
                )
            },
            b"hello"
        );

        assert_eq!(
            unsafe {
                slice::from_raw_parts(
                    mmap.offset(1 * frame_size + layout.frame_headroom) as *const u8,
                    6,
                )
            },
            b"world!"
        );
    }

    #[test]
    fn writes_are_contiguous() {
        let layout = FrameLayout {
            _xdp_headroom: 4,
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

        let frame_size = layout.frame_size();

        let mmap = Mmap::new(frame_count * frame_size, false).unwrap();

        (0..frame_count).into_iter().for_each(|i| {
            let mut frame = unsafe {
                Frame::new(
                    (i * frame_size) + layout._xdp_headroom + layout.frame_headroom,
                    layout,
                    mmap.clone(),
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
        let mmap_region =
            unsafe { slice::from_raw_parts(mmap.as_mut_ptr() as *const u8, mmap.len()) };

        assert_eq!(mmap_region, expected_layout)
    }
}
