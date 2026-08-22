mod mmap;
use mmap::Mmap;

use std::{
    io,
    num::NonZeroU32,
    ptr::NonNull,
    slice,
    sync::{Arc, Mutex},
};

use super::{
    FrameLayout,
    frame::{Data, DataMut, FrameDesc, Headroom, HeadroomMut},
};

/// A framed, memory mapped region which functions as the working
/// memory for some UMEM.
#[derive(Clone, Debug)]
pub struct UmemRegion {
    layout: FrameLayout,
    // Keep a copy of the pointer to the mmap region to avoid a double
    // deref, through for example an `Arc<Mmap>`. We know this won't
    // dangle since this struct holds an `Arc`d copy of the mmap
    // region.
    addr: NonNull<libc::c_void>,
    len: usize,
    _mmap: Arc<Mutex<Mmap>>,
}

unsafe impl Send for UmemRegion {}

// SAFETY: this impl is only safe in the context of this library and
// assuming the various unsafe requirements are upheld. Mutations to
// the memory region may occur concurrently but always in disjoint
// sections by either the user space process xor the kernel.
unsafe impl Sync for UmemRegion {}

impl UmemRegion {
    pub(super) fn new(
        frame_count: NonZeroU32,
        frame_layout: FrameLayout,
        use_huge_pages: bool,
    ) -> io::Result<Self> {
        let len = (frame_count.get() as usize) * (frame_layout.frame_size() as usize);

        let mmap = Mmap::new(len, use_huge_pages)?;

        Ok(Self {
            layout: frame_layout,
            addr: mmap.addr(),
            len,
            _mmap: Arc::new(Mutex::new(mmap)),
        })
    }

    /// The size of the underlying memory region.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Get a pointer to the start of the memory region.
    #[inline]
    pub fn as_ptr(&self) -> *mut libc::c_void {
        self.addr.as_ptr()
    }

    /// A pointer to the headroom segment of the frame described by
    /// `desc`.
    ///
    /// # Safety
    ///
    /// `desc` must describe a frame belonging to this [`UmemRegion`].
    ///
    /// Frame addresses are `u64` regardless of the host's pointer
    /// width, so reaching the frame means narrowing to a `usize`. On a
    /// 64-bit target that is a no-op; on a 32-bit one it holds because
    /// an address is an offset into a region this process has already
    /// mapped, and so cannot exceed its address space.
    ///
    /// That is a property of the addresses the kernel hands back, not
    /// something the type system pins down - unaligned chunk mode, for
    /// one, packs a frame offset into the top 16 bits of an address
    /// and would break it - so [`umem_offset`](Self::umem_offset)
    /// checks it rather than assuming it.
    #[inline]
    unsafe fn headroom_ptr(&self, desc: &FrameDesc) -> *mut u8 {
        let addr = self.umem_offset(desc) - self.layout.frame_headroom as usize;
        unsafe { self.as_ptr().add(addr) as *mut u8 }
    }

    /// `desc`'s address as an offset into this region.
    ///
    /// Panics in debug builds if the address does not fit a `usize`,
    /// which on a 64-bit target it always does. See
    /// [`headroom_ptr`](Self::headroom_ptr) for why it holds on a
    /// 32-bit one, and why it is worth checking anyway: truncating
    /// here would not fail, it would silently address the wrong frame.
    #[inline]
    fn umem_offset(&self, desc: &FrameDesc) -> usize {
        debug_assert!(
            desc.addr <= usize::MAX as u64,
            "frame address {} does not fit a usize on this target",
            desc.addr
        );

        desc.addr as usize
    }

    /// A pointer to the headroom segment of the frame described to by
    /// `desc`.
    ///
    /// # Safety
    ///
    /// `desc` must describe a frame belonging to this [`UmemRegion`].
    ///
    /// See [`headroom_ptr`](Self::headroom_ptr) on the narrowing.
    #[inline]
    unsafe fn data_ptr(&self, desc: &FrameDesc) -> *mut u8 {
        unsafe { self.as_ptr().add(self.umem_offset(desc)) as *mut u8 }
    }

    /// See docs for [`super::Umem::frame`].
    #[inline]
    pub unsafe fn frame(&self, desc: &FrameDesc) -> (Headroom<'_>, Data<'_>) {
        // SAFETY: see `super::Umem::frame`
        unsafe { (self.headroom(desc), self.data(desc)) }
    }

    /// See docs for [`super::Umem::headroom`].
    #[inline]
    pub unsafe fn headroom(&self, desc: &FrameDesc) -> Headroom<'_> {
        // SAFETY: see `frame`.
        let headroom_ptr = unsafe { self.headroom_ptr(desc) };

        Headroom::new(unsafe {
            slice::from_raw_parts(headroom_ptr, desc.lengths.headroom as usize)
        })
    }

    /// See docs for [`super::Umem::data`].
    #[inline]
    pub unsafe fn data(&self, desc: &FrameDesc) -> Data<'_> {
        // SAFETY: see `frame`.
        let data_ptr = unsafe { self.data_ptr(desc) };

        Data::new(unsafe { slice::from_raw_parts(data_ptr, desc.lengths.data as usize) })
    }

    /// See docs for [`super::Umem::frame_mut`].
    #[inline]
    pub unsafe fn frame_mut<'a>(
        &'a self,
        desc: &'a mut FrameDesc,
    ) -> (HeadroomMut<'a>, DataMut<'a>) {
        // SAFETY: see `super::Umem::frame_mut`
        let headroom_ptr = unsafe { self.headroom_ptr(desc) };
        let data_ptr = unsafe { self.data_ptr(desc) };

        let headroom =
            unsafe { slice::from_raw_parts_mut(headroom_ptr, self.layout.frame_headroom as usize) };

        let data = unsafe { slice::from_raw_parts_mut(data_ptr, self.layout.mtu as usize) };

        (
            HeadroomMut::new(&mut desc.lengths.headroom, headroom),
            DataMut::new(&mut desc.lengths.data, data),
        )
    }

    /// See docs for [`super::Umem::headroom_mut`].
    #[inline]
    pub unsafe fn headroom_mut<'a>(&'a self, desc: &'a mut FrameDesc) -> HeadroomMut<'a> {
        // SAFETY: see `frame_mut`.
        let headroom_ptr = unsafe { self.headroom_ptr(desc) };

        let headroom =
            unsafe { slice::from_raw_parts_mut(headroom_ptr, self.layout.frame_headroom as usize) };

        HeadroomMut::new(&mut desc.lengths.headroom, headroom)
    }

    /// See docs for [`super::Umem::data_mut`].
    #[inline]
    pub unsafe fn data_mut<'a>(&'a self, desc: &'a mut FrameDesc) -> DataMut<'a> {
        // SAFETY: see `frame_mut`.
        let data_ptr = unsafe { self.data_ptr(desc) };

        let data = unsafe { slice::from_raw_parts_mut(data_ptr, self.layout.mtu as usize) };

        DataMut::new(&mut desc.lengths.data, data)
    }
}
