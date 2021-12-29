//! Types for representing and working with a [`Umem`](super::Umem)
//! frame.

mod cursor;
pub use cursor::Cursor;

use std::{borrow::Borrow, ops::Deref};

/// The lengths of a frame's packet data and headroom segments.
#[derive(Debug, Default, Clone, Copy)]
pub struct FrameLengths {
    pub(crate) headroom: usize,
    pub(crate) data: usize,
}

impl FrameLengths {
    /// Current length of the headroom segment.
    #[inline]
    pub fn headroom(&self) -> usize {
        self.headroom
    }

    /// Current length of the packet data segment.
    #[inline]
    pub fn data(&self) -> usize {
        self.data
    }
}

/// A [`Umem`](super::Umem) frame descriptor.
///
/// Used to pass frame information between the kernel and
/// userspace. The `addr` field is an offset in bytes from the start
/// of the [`Umem`](super::Umem) and corresponds to the starting
/// address of the data segment of some frame. The `len` field
/// describes the length (in bytes) of any data stored in that frame,
/// starting from `addr`.
#[derive(Debug)]
pub struct FrameDesc {
    pub(crate) addr: usize,
    pub(crate) options: u32,
    pub(crate) lengths: FrameLengths,
}

impl FrameDesc {
    /// Create a new frame which belongs to `umem`, whose packet data
    /// segment starts at `addr` and with a layout described by
    /// `frame_layout.`
    ///
    /// # Safety
    ///
    /// `addr` must be the starting address of the packet data segment
    /// of some frame belonging to `umem`.
    pub(super) fn new(addr: usize) -> Self {
        Self {
            addr,
            options: 0,
            lengths: FrameLengths::default(),
        }
    }

    /// The starting address of this frame's packet data segment.
    #[inline]
    pub fn addr(&self) -> usize {
        self.addr
    }

    /// Current headroom and packet data lengths for this frame.
    #[inline]
    pub fn lengths(&self) -> &FrameLengths {
        &self.lengths
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
    pub(crate) fn write_xdp_desc(&self, desc: &mut libbpf_sys::xdp_desc) {
        desc.addr = self.addr() as u64;
        desc.options = self.options();
        desc.len = self.lengths.data as u32;
    }
}

/// Headroom segment of a [`Umem`](crate::umem::Umem) frame.
#[derive(Debug)]
pub struct Headroom<'umem> {
    contents: &'umem [u8],
}

impl<'umem> Headroom<'umem> {
    pub(super) fn new(contents: &'umem [u8]) -> Self {
        Self { contents }
    }

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

impl<'umem> HeadroomMut<'umem> {
    pub(super) fn new(len: &'umem mut usize, buf: &'umem mut [u8]) -> Self {
        Self { len, buf }
    }

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

impl<'umem> Data<'umem> {
    pub(super) fn new(contents: &'umem [u8]) -> Self {
        Self { contents }
    }

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

impl<'umem> DataMut<'umem> {
    pub(super) fn new(len: &'umem mut usize, buf: &'umem mut [u8]) -> Self {
        Self { len, buf }
    }

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

    use crate::umem::{FrameDesc, FrameLayout, FramedMmap, Mmap};

    #[test]
    fn writes_persist() {
        let layout = FrameLayout {
            _xdp_headroom: 0,
            frame_headroom: 512,
            mtu: 2048,
        };

        let frame_size = layout.frame_size();

        let mmap = FramedMmap {
            layout,
            mmap: Mmap::new(16 * frame_size, false).unwrap(),
        };

        let mut desc_0 = FrameDesc::new(0 * frame_size + layout.frame_headroom);

        let mut desc_1 = FrameDesc::new(1 * frame_size + layout.frame_headroom);

        let mut xdp_desc = xdp_desc::default();

        unsafe { mmap.data_mut(&mut desc_0) }
            .cursor()
            .write_all(b"hello")
            .unwrap();

        desc_0.write_xdp_desc(&mut xdp_desc);

        assert_eq!(
            xdp_desc.addr,
            (0 * frame_size + layout.frame_headroom) as u64
        );
        assert_eq!(xdp_desc.len, 5);
        assert_eq!(xdp_desc.options, 0);

        unsafe { mmap.data_mut(&mut desc_1) }
            .cursor()
            .write_all(b"world!")
            .unwrap();

        desc_1.write_xdp_desc(&mut xdp_desc);

        assert_eq!(
            xdp_desc.addr,
            (1 * frame_size + layout.frame_headroom) as u64
        );
        assert_eq!(xdp_desc.len, 6);
        assert_eq!(xdp_desc.options, 0);

        assert_eq!(
            unsafe {
                slice::from_raw_parts(
                    mmap.mmap.offset(0 * frame_size + layout.frame_headroom) as *const u8,
                    5,
                )
            },
            b"hello"
        );

        assert_eq!(
            unsafe {
                slice::from_raw_parts(
                    mmap.mmap.offset(1 * frame_size + layout.frame_headroom) as *const u8,
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

        let mmap = FramedMmap {
            layout,
            mmap: Mmap::new(frame_count * frame_size, false).unwrap(),
        };

        (0..frame_count).into_iter().for_each(|i| {
            let mut desc =
                FrameDesc::new((i * frame_size) + layout._xdp_headroom + layout.frame_headroom);

            let (mut headroom, mut data) = unsafe { mmap.frame_mut(&mut desc) };

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
            unsafe { slice::from_raw_parts(mmap.mmap.as_mut_ptr() as *const u8, mmap.mmap.len()) };

        assert_eq!(mmap_region, expected_layout)
    }
}
