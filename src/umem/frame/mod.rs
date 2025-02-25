//! Types for representing and working with a [`Umem`](super::Umem)
//! frame.

mod cursor;
pub use cursor::Cursor;

use std::{
    borrow::{Borrow, BorrowMut},
    ops::{Deref, DerefMut},
};

/// The length (in bytes) of data in a frame's packet data and
/// headroom segments.
///
/// Not to be confused with the [`frame_headroom`] and [`mtu`], the
/// lengths here describe the amount of data that has been written to
/// either segment, either by the kernel or by the user. Hence they
/// vary as frames are used to send and receive data.
///
/// The two sets of values are related however, in that `headroom`
/// will always be less than or equal to [`frame_headroom`], and
/// `data` less than or equal to [`mtu`].
///
/// [`frame_headroom`]: crate::config::UmemConfig::frame_headroom
/// [`mtu`]: crate::config::UmemConfig::mtu
#[derive(Debug, Default, Clone, Copy)]
pub struct SegmentLengths {
    pub(crate) headroom: usize,
    pub(crate) data: usize,
}

impl SegmentLengths {
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
/// userspace. `addr` is an offset in bytes from the start of the
/// [`Umem`](super::Umem) and corresponds to the starting address of
/// the packet data segment of some frame. `lengths` describes the
/// length (in bytes) of any data stored in the frame's headroom or
/// data segments.
#[derive(Debug, Clone, Copy)]
pub struct FrameDesc {
    pub(crate) addr: usize,
    pub(crate) options: u32,
    pub(crate) lengths: SegmentLengths,
}

impl FrameDesc {
    /// Creates a new frame descriptor.
    ///
    /// `addr` must be the starting address of the packet data segment
    /// of some [`Umem`](super::Umem) frame.
    pub(super) fn new(addr: usize) -> Self {
        Self {
            addr,
            options: 0,
            lengths: SegmentLengths::default(),
        }
    }

    /// The starting address of the packet data segment of the frame
    /// pointed at by this descriptor.
    #[inline]
    pub fn addr(&self) -> usize {
        self.addr
    }

    /// Current headroom and packet data lengths for the frame pointed
    /// at by this descriptor.
    #[inline]
    pub fn lengths(&self) -> &SegmentLengths {
        &self.lengths
    }

    /// Frame options.
    #[inline]
    pub fn options(&self) -> u32 {
        self.options
    }

    /// Set the frame options.
    #[inline]
    pub fn set_options(&mut self, options: u32) {
        self.options = options
    }

    #[inline]
    pub(crate) fn write_xdp_desc(&self, desc: &mut libxdp_sys::xdp_desc) {
        desc.addr = self.addr as u64;
        desc.options = self.options;
        desc.len = self.lengths.data as u32;
    }
}

impl Default for FrameDesc {
    /// Creates an empty frame descriptor with an address of zero and
    /// segment lengths also set to zero.
    ///
    /// Since the address of any descriptors created this way is
    /// always zero, before using them to write to the [`Umem`] they
    /// should first be 'initialised' by passing them to either the
    /// [`RxQueue`] or the [`CompQueue`], so they can be populated
    /// with the details of a free frame.
    ///
    /// [`Umem`]: crate::Umem
    /// [`RxQueue`]: crate::RxQueue
    /// [`CompQueue`]: crate::CompQueue
    fn default() -> Self {
        Self {
            addr: 0,
            options: 0,
            lengths: Default::default(),
        }
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

/// Mutable headroom segment of a [`Umem`](crate::umem::Umem) frame.
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
    #[inline]
    pub fn contents(&self) -> &[u8] {
        &self.buf[..*self.len]
    }

    /// Returns a mutable view of this segment's contents, up to its
    /// current length.
    #[inline]
    pub fn contents_mut(&mut self) -> &mut [u8] {
        &mut self.buf[..*self.len]
    }

    /// A cursor for writing to this segment.
    ///
    /// Modifications via the cursor will change the length of the
    /// segment, i.e. the headroom length of the frame descriptor. If
    /// in-place modifications just need to be made then
    /// [`contents_mut`] may be sufficient.
    ///
    /// [`contents_mut`]: Self::contents_mut
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

impl AsMut<[u8]> for HeadroomMut<'_> {
    #[inline]
    fn as_mut(&mut self) -> &mut [u8] {
        self.contents_mut()
    }
}

impl Borrow<[u8]> for HeadroomMut<'_> {
    #[inline]
    fn borrow(&self) -> &[u8] {
        self.contents()
    }
}

impl BorrowMut<[u8]> for HeadroomMut<'_> {
    #[inline]
    fn borrow_mut(&mut self) -> &mut [u8] {
        self.contents_mut()
    }
}

impl Deref for HeadroomMut<'_> {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.contents()
    }
}

impl DerefMut for HeadroomMut<'_> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.contents_mut()
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
    pub fn contents(&self) -> &'umem [u8] {
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

/// Mutable packet data segment of a [`Umem`](crate::umem::Umem)
/// frame.
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

    /// Returns a mutable view of this segment's contents, up to its
    /// current length.
    ///
    /// Will change as packets are sent or received using this frame.
    #[inline]
    pub fn contents_mut(&mut self) -> &mut [u8] {
        &mut self.buf[..*self.len]
    }

    /// A cursor for writing to this segment.
    ///
    /// Modifications via the cursor will change the length of the
    /// segment, i.e. the data length of the frame descriptor, and in
    /// this case the size of the packet that will be submitted. If
    /// in-place modifications just need to be made then
    /// [`contents_mut`] may be sufficient.
    ///
    /// [`contents_mut`]: Self::contents_mut
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

impl AsMut<[u8]> for DataMut<'_> {
    #[inline]
    fn as_mut(&mut self) -> &mut [u8] {
        self.contents_mut()
    }
}

impl Borrow<[u8]> for DataMut<'_> {
    #[inline]
    fn borrow(&self) -> &[u8] {
        self.contents()
    }
}

impl BorrowMut<[u8]> for DataMut<'_> {
    #[inline]
    fn borrow_mut(&mut self) -> &mut [u8] {
        self.contents_mut()
    }
}

impl Deref for DataMut<'_> {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.contents()
    }
}

impl DerefMut for DataMut<'_> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.contents_mut()
    }
}

#[cfg(test)]
mod tests {
    use core::slice;
    use std::{
        convert::TryInto,
        io::{self, Write},
    };

    use libxdp_sys::xdp_desc;

    use crate::umem::{FrameDesc, FrameLayout, UmemRegion};

    #[test]
    fn writes_persist() {
        let layout = FrameLayout {
            xdp_headroom: 0,
            frame_headroom: 512,
            mtu: 2048,
        };

        let frame_count = 16.try_into().unwrap();
        let frame_size = layout.frame_size();

        let umem_region = UmemRegion::new(frame_count, layout, false).unwrap();

        let mut desc_0 = FrameDesc::new(0 * frame_size + layout.frame_headroom);

        let mut desc_1 = FrameDesc::new(1 * frame_size + layout.frame_headroom);

        let mut xdp_desc = xdp_desc {
            addr: 0,
            len: 0,
            options: 0,
        };

        unsafe { umem_region.data_mut(&mut desc_0) }
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

        unsafe { umem_region.data_mut(&mut desc_1) }
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
                    umem_region
                        .as_ptr()
                        .add(0 * frame_size + layout.frame_headroom)
                        as *const u8,
                    5,
                )
            },
            b"hello"
        );

        assert_eq!(
            unsafe {
                slice::from_raw_parts(
                    umem_region
                        .as_ptr()
                        .add(1 * frame_size + layout.frame_headroom)
                        as *const u8,
                    6,
                )
            },
            b"world!"
        );
    }

    #[test]
    fn writes_are_contiguous() {
        let layout = FrameLayout {
            xdp_headroom: 4,
            frame_headroom: 8,
            mtu: 12,
        };

        let frame_count = 4.try_into().unwrap();
        let umem_region = UmemRegion::new(frame_count, layout, false).unwrap();

        // An arbitrary layout
        let xdp_headroom_segment = [0, 0, 0, 0];
        let frame_headroom_segment = [1, 1, 1, 1, 1, 1, 1, 1];
        let data_segment = [2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2];

        let mut cursor = io::Cursor::new(Vec::new());

        cursor.write_all(&xdp_headroom_segment).unwrap();
        cursor.write_all(&frame_headroom_segment).unwrap();
        cursor.write_all(&data_segment).unwrap();

        let base_layout: Vec<u8> = cursor.into_inner();

        let expected_layout: Vec<u8> = (0..frame_count.get() as u8)
            .into_iter()
            .map(|i| {
                base_layout
                    .iter()
                    .map(|el| el * (i + 1))
                    .collect::<Vec<_>>()
            })
            .flatten()
            .collect();

        (0..frame_count.get() as usize).into_iter().for_each(|i| {
            let mut desc = FrameDesc::new(
                (i * layout.frame_size()) + layout.xdp_headroom + layout.frame_headroom,
            );

            let (mut headroom, mut data) = unsafe { umem_region.frame_mut(&mut desc) };

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
            unsafe { slice::from_raw_parts(umem_region.as_ptr() as *const u8, umem_region.len()) };

        assert_eq!(mmap_region, expected_layout)
    }
}
