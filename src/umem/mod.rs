//! Types for interacting with and creating a [`Umem`].

mod mmap;
use mmap::Mmap;

pub mod frame;
use frame::{Data, DataMut, FrameDesc, Headroom, HeadroomMut};

mod fill_queue;
pub use fill_queue::FillQueue;

mod comp_queue;
pub use comp_queue::CompQueue;

use libbpf_sys::{xsk_umem, XDP_PACKET_HEADROOM};
use std::{
    borrow::Borrow,
    error::Error,
    fmt, io,
    num::NonZeroU32,
    ptr::{self, NonNull},
    slice,
    sync::{Arc, Mutex},
};

use crate::{
    config::UmemConfig,
    ring::{XskRingCons, XskRingProd},
};

/// Wrapper around a pointer to some [`Umem`]. Guarantees
/// that the pointer is both non-null and unique.
#[derive(Debug)]
struct XskUmem(NonNull<xsk_umem>);

impl XskUmem {
    /// # Safety
    ///
    /// Only one instance of this struct may exist since it deletes
    /// the UMEM as part of its [`Drop`] impl. If there are copies or
    /// clones of `ptr` then care must be taken to ensure they aren't
    /// used once this struct goes out of scope, and that they don't
    /// delete the UMEM themselves.
    unsafe fn new(ptr: NonNull<xsk_umem>) -> Self {
        Self(ptr)
    }

    fn as_mut_ptr(&self) -> *mut xsk_umem {
        self.0.as_ptr()
    }
}

impl Drop for XskUmem {
    fn drop(&mut self) {
        // SAFETY: unsafe constructor contract guarantees that the
        // UMEM has not been deleted already.
        let err = unsafe { libbpf_sys::xsk_umem__delete(self.0.as_ptr()) };

        if err != 0 {
            log::error!("failed to delete umem with error code {}", err);
        }
    }
}

unsafe impl Send for XskUmem {}

/// Wraps the [`xsk_umem`] pointer and [`Mmap`]'d area together to
/// ensure they're dropped in tandem.
///
/// Note that `umem` must appear before `mmap` to ensure correct drop
/// order.
#[derive(Debug)]
struct UmemInner {
    umem_ptr: XskUmem,
    saved_fq_and_cq: Option<(XskRingProd, XskRingCons)>,
}

impl UmemInner {
    fn new(umem_ptr: XskUmem, saved_fq_and_cq: Option<(XskRingProd, XskRingCons)>) -> Self {
        Self {
            umem_ptr,
            saved_fq_and_cq,
        }
    }
}

#[derive(Debug, Clone)]
struct FramedMmap {
    layout: FrameLayout,
    mmap: Mmap,
}

impl FramedMmap {
    /// Retrieve a pointer to the headroom segment of this frame.
    ///
    /// # Safety
    ///
    /// `addr` must point to the start of a packet data segment of a
    /// frame within this UMEM.
    #[inline]
    unsafe fn headroom_ptr(&self, addr: usize) -> *mut u8 {
        let addr = addr - self.layout.frame_headroom;
        unsafe { self.mmap.offset(addr) as *mut u8 }
    }

    /// Retrieve a pointer to the packet data segment of this frame.
    ///
    /// # Safety
    ///
    /// `addr` must point to the start of a packet data segment of a
    /// frame within this UMEM.
    #[inline]
    unsafe fn data_ptr(&self, addr: usize) -> *mut u8 {
        unsafe { self.mmap.offset(addr) as *mut u8 }
    }

    /// The frame's headroom and packet data segments.
    ///
    /// # Safety
    ///
    /// `desc` must describe a frame that belongs to this
    /// [`Umem`]. Furthermore, the underlying frame described by
    /// `desc` must not be mutably accessed anywhere else at the same
    /// time, either in userspace or by the kernel.
    #[inline]
    unsafe fn frame(&self, desc: &FrameDesc) -> (Headroom, Data) {
        // SAFETY: unsafe contract in constructor and `set_desc`
        // ensures that this frame's current `addr` points to the
        // start of a packet data segment in its underlying UMEM -
        // therefore the dereferenced slices are whole and valid
        // segments.
        //
        // The unsafe contract of this function also guarantees there
        // are no other mutable references to these slices at the same
        // time.
        unsafe { (self.headroom(desc), self.data(desc)) }
    }

    /// The frame's headroom segment.
    ///
    /// # Safety
    ///
    /// See [`get`](Frame::segments).
    #[inline]
    unsafe fn headroom(&self, desc: &FrameDesc) -> Headroom {
        // SAFETY: see `segments`.
        let headroom_ptr = unsafe { self.headroom_ptr(desc.addr) };

        Headroom::new(unsafe { slice::from_raw_parts(headroom_ptr, desc.lengths.headroom) })
    }

    /// The frame's packet data segment
    ///
    /// # Safety
    ///
    /// See [`get`](Frame::segments).
    #[inline]
    unsafe fn data(&self, desc: &FrameDesc) -> Data {
        // SAFETY: see `segments`.
        let data_ptr = unsafe { self.data_ptr(desc.addr()) };

        Data::new(unsafe { slice::from_raw_parts(data_ptr, desc.lengths.data) })
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
    unsafe fn frame_mut<'a>(&'a self, desc: &'a mut FrameDesc) -> (HeadroomMut<'a>, DataMut<'a>) {
        // SAFETY: unsafe contract in constructor and `set_desc`
        // ensures that this frame's current `addr` points to the
        // start of a packet data segment in its underlying UMEM -
        // therefore the dereferenced slices are whole and valid
        // segments.
        //
        // The unsafe contract of this function also guarantees there
        // are no other mutable or immutable references to these
        // slices at the same time.
        let headroom_ptr = unsafe { self.headroom_ptr(desc.addr) };
        let data_ptr = unsafe { self.data_ptr(desc.addr) };

        let headroom =
            unsafe { slice::from_raw_parts_mut(headroom_ptr, self.layout.frame_headroom) };

        let data = unsafe { slice::from_raw_parts_mut(data_ptr, self.layout.mtu) };

        (
            HeadroomMut::new(&mut desc.lengths.headroom, headroom),
            DataMut::new(&mut desc.lengths.data, data),
        )
    }

    /// A mutable reference to the frame's headroom segment.
    ///
    /// # Safety
    ///
    /// See [`get_mut`](Frame::segments_mut).
    #[inline]
    unsafe fn headroom_mut<'a>(&'a self, desc: &'a mut FrameDesc) -> HeadroomMut<'a> {
        // SAFETY: see `segments_mut`.
        let headroom_ptr = unsafe { self.headroom_ptr(desc.addr) };

        let headroom =
            unsafe { slice::from_raw_parts_mut(headroom_ptr, self.layout.frame_headroom) };

        HeadroomMut::new(&mut desc.lengths.headroom, headroom)
    }

    /// A mutable reference to the frame's packet data segment.
    ///
    /// # Safety
    ///
    /// See [`get_mut`](Frame::segments_mut).
    #[inline]
    unsafe fn data_mut<'a>(&'a self, desc: &'a mut FrameDesc) -> DataMut<'a> {
        // SAFETY: see `segments_mut`.
        let data_ptr = unsafe { self.data_ptr(desc.addr) };

        let data = unsafe { slice::from_raw_parts_mut(data_ptr, self.layout.mtu) };

        DataMut::new(&mut desc.lengths.data, data)
    }
}

/// A region of virtual contiguous memory divided into equal-sized
/// frames. It provides the underlying working memory for an AF_XDP
/// [`Socket`](crate::socket::Socket).
#[derive(Debug, Clone)]
pub struct Umem {
    inner: Arc<Mutex<UmemInner>>,
    mmap: FramedMmap,
}

impl Umem {
    /// Create a new UMEM instance backed by an anonymous memory
    /// mapped region.
    ///
    /// Setting `use_huge_pages` to `true` will instructed `mmap()` to
    /// allocate the underlying memory using huge pages. If you are
    /// getting errors as a result of this, check that the
    /// `HugePages_Total` setting is non-zero when you run `cat
    /// /proc/meminfo`.
    pub fn new(
        config: UmemConfig,
        frame_count: NonZeroU32,
        use_huge_pages: bool,
    ) -> Result<(Self, Vec<FrameDesc>), UmemCreateError> {
        let frame_size = config.frame_size().get() as usize;
        let frame_count = frame_count.get() as usize;

        let mmap_len = frame_size * frame_count;

        let mmap = Mmap::new(mmap_len, use_huge_pages).map_err(|e| UmemCreateError {
            reason: "failed to create underlying mmap region",
            err: e,
        })?;

        let mut umem_ptr = ptr::null_mut();
        let mut fq = XskRingProd::default();
        let mut cq = XskRingCons::default();

        let err = unsafe {
            libbpf_sys::xsk_umem__create(
                &mut umem_ptr,
                mmap.as_mut_ptr(),
                mmap_len as u64,
                fq.as_mut(),
                cq.as_mut(),
                &config.into(),
            )
        };

        let umem_ptr = match NonNull::new(umem_ptr) {
            Some(umem_ptr) => {
                // SAFETY: this is the only `XskUmem` instance for
                // this pointer, and no other pointers to the UMEM
                // exist.
                unsafe { XskUmem::new(umem_ptr) }
            }
            None => {
                return Err(UmemCreateError {
                    reason: "returned UMEM pointer is null",
                    err: io::Error::from_raw_os_error(err),
                });
            }
        };

        if err != 0 {
            return Err(UmemCreateError {
                reason: "non-zero error code returned when creating UMEM",
                err: io::Error::from_raw_os_error(err),
            });
        }

        if fq.is_ring_null() {
            return Err(UmemCreateError {
                reason: "returned fill queue ring is null",
                err: io::Error::from_raw_os_error(err),
            });
        };

        if cq.is_ring_null() {
            return Err(UmemCreateError {
                reason: "returned comp queue ring is null",
                err: io::Error::from_raw_os_error(err),
            });
        }

        let inner = UmemInner::new(umem_ptr, Some((fq, cq)));

        let xdp_headroom = XDP_PACKET_HEADROOM as usize;
        let frame_headroom = config.frame_headroom() as usize;
        let mtu = frame_size - (xdp_headroom + frame_headroom);

        let frame_layout = FrameLayout {
            _xdp_headroom: xdp_headroom,
            frame_headroom,
            mtu,
        };

        let mut frame_descs: Vec<FrameDesc> = Vec::with_capacity(frame_count);

        for i in 0..frame_count {
            let addr = (i * frame_size) + xdp_headroom + frame_headroom;

            frame_descs.push(FrameDesc::new(addr));
        }

        let umem = Umem {
            inner: Arc::new(Mutex::new(inner)),
            mmap: FramedMmap {
                layout: frame_layout,
                mmap,
            },
        };

        Ok((umem, frame_descs))
    }

    /// The frame's headroom and packet data segments.
    ///
    /// # Safety
    ///
    /// The underlying [`Umem`](super::Umem) region this frame
    /// accesses must not be mutably accessed anywhere else at the
    /// same time, either in userspace or by the kernel.
    #[inline]
    pub unsafe fn frame(&self, desc: &FrameDesc) -> (Headroom, Data) {
        // SAFETY: unsafe contract in constructor and `set_desc`
        // ensures that this frame's current `addr` points to the
        // start of a packet data segment in its underlying UMEM -
        // therefore the dereferenced slices are whole and valid
        // segments.
        //
        // The unsafe contract of this function also guarantees there
        // are no other mutable references to these slices at the same
        // time.
        unsafe { self.mmap.frame(desc) }
    }

    /// The frame's headroom segment.
    ///
    /// # Safety
    ///
    /// See [`get`](Frame::segments).
    #[inline]
    pub unsafe fn headroom(&self, desc: &FrameDesc) -> Headroom {
        // SAFETY: see `frame`.
        unsafe { self.mmap.headroom(desc) }
    }

    /// The frame's packet data segment
    ///
    /// # Safety
    ///
    /// See [`get`](Frame::segments).
    #[inline]
    pub unsafe fn data(&self, desc: &FrameDesc) -> Data {
        // SAFETY: see `frame`.
        unsafe { self.mmap.data(desc) }
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
    pub unsafe fn frame_mut<'a>(
        &'a self,
        desc: &'a mut FrameDesc,
    ) -> (HeadroomMut<'a>, DataMut<'a>) {
        // SAFETY: unsafe contract in constructor and `set_desc`
        // ensures that this frame's current `addr` points to the
        // start of a packet data segment in its underlying UMEM -
        // therefore the dereferenced slices are whole and valid
        // segments.
        //
        // The unsafe contract of this function also guarantees there
        // are no other mutable or immutable references to these
        // slices at the same time.
        unsafe { self.mmap.frame_mut(desc) }
    }

    /// A mutable reference to the frame's headroom segment.
    ///
    /// # Safety
    ///
    /// See [`get_mut`](Frame::segments_mut).
    #[inline]
    pub unsafe fn headroom_mut<'a>(&'a self, desc: &'a mut FrameDesc) -> HeadroomMut<'a> {
        // SAFETY: see `frame_mut`.
        unsafe { self.mmap.headroom_mut(desc) }
    }

    /// A mutable reference to the frame's packet data segment.
    ///
    /// # Safety
    ///
    /// See [`get_mut`](Frame::segments_mut).
    #[inline]
    pub unsafe fn data_mut<'a>(&'a self, desc: &'a mut FrameDesc) -> DataMut<'a> {
        // SAFETY: see `frame_mut`.
        unsafe { self.mmap.data_mut(desc) }
    }

    /// Intended to be called on socket creation, this passes the
    /// create function a pointer to the UMEM and any saved fill queue
    /// or completion queue.
    ///
    /// Regarding the saved queues, this is a byproduct of how the
    /// UMEM is created in the C code and we save them here to avoid
    /// leaking memory.
    #[inline]
    pub(crate) fn with_ptr_and_saved_queues<F, T>(&self, mut f: F) -> T
    where
        F: FnMut(*mut xsk_umem, &mut Option<(XskRingProd, XskRingCons)>) -> T,
    {
        let mut inner = self.inner.lock().unwrap();

        f(inner.umem_ptr.as_mut_ptr(), &mut inner.saved_fq_and_cq)
    }
}

/// Error detailing why [`Umem`] creation failed.
#[derive(Debug)]
pub struct UmemCreateError {
    reason: &'static str,
    err: io::Error,
}

impl fmt::Display for UmemCreateError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.reason)
    }
}

impl Error for UmemCreateError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.err.borrow())
    }
}

/// Dimensions of a [`Umem`] frame.
#[derive(Debug, Clone, Copy)]
struct FrameLayout {
    _xdp_headroom: usize,
    frame_headroom: usize,
    mtu: usize,
}

impl FrameLayout {
    #[cfg(test)]
    fn frame_size(&self) -> usize {
        self._xdp_headroom + self.frame_headroom + self.mtu
    }
}
