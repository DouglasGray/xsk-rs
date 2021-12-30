//! Types for interacting with and creating a [`Umem`].

mod mem;
use mem::UmemRegion;

pub mod frame;
use frame::{Data, DataMut, FrameDesc, Headroom, HeadroomMut};

mod fill_queue;
pub use fill_queue::FillQueue;

mod comp_queue;
pub use comp_queue::CompQueue;

use libbpf_sys::xsk_umem;
use std::{
    borrow::Borrow,
    error::Error,
    fmt, io,
    num::NonZeroU32,
    ptr::{self, NonNull},
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

/// A region of virtual contiguous memory divided into equal-sized
/// frames. It provides the underlying working memory for an AF_XDP
/// [`Socket`](crate::socket::Socket).
#[derive(Debug, Clone)]
pub struct Umem {
    inner: Arc<Mutex<UmemInner>>,
    mem: UmemRegion,
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
        let frame_layout = config.into();

        let mem = UmemRegion::new(frame_count, frame_layout, use_huge_pages).map_err(|e| {
            UmemCreateError {
                reason: "failed to create mmap'd UMEM region",
                err: e,
            }
        })?;

        let mut umem_ptr = ptr::null_mut();
        let mut fq = XskRingProd::default();
        let mut cq = XskRingCons::default();

        let err = unsafe {
            libbpf_sys::xsk_umem__create(
                &mut umem_ptr,
                mem.as_ptr(),
                mem.len() as u64,
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

        let frame_count = frame_count.get() as usize;

        let mut frame_descs: Vec<FrameDesc> = Vec::with_capacity(frame_count);

        for i in 0..frame_count {
            let addr = (i * frame_layout.frame_size())
                + frame_layout.xdp_headroom
                + frame_layout.frame_headroom;

            frame_descs.push(FrameDesc::new(addr));
        }

        let umem = Umem {
            inner: Arc::new(Mutex::new(inner)),
            mem,
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
        unsafe { self.mem.frame(desc) }
    }

    /// The frame's headroom segment.
    ///
    /// # Safety
    ///
    /// See [`get`](Frame::segments).
    #[inline]
    pub unsafe fn headroom(&self, desc: &FrameDesc) -> Headroom {
        // SAFETY: see `frame`.
        unsafe { self.mem.headroom(desc) }
    }

    /// The frame's packet data segment
    ///
    /// # Safety
    ///
    /// See [`get`](Frame::segments).
    #[inline]
    pub unsafe fn data(&self, desc: &FrameDesc) -> Data {
        // SAFETY: see `frame`.
        unsafe { self.mem.data(desc) }
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
        unsafe { self.mem.frame_mut(desc) }
    }

    /// A mutable reference to the frame's headroom segment.
    ///
    /// # Safety
    ///
    /// See [`get_mut`](Frame::segments_mut).
    #[inline]
    pub unsafe fn headroom_mut<'a>(&'a self, desc: &'a mut FrameDesc) -> HeadroomMut<'a> {
        // SAFETY: see `frame_mut`.
        unsafe { self.mem.headroom_mut(desc) }
    }

    /// A mutable reference to the frame's packet data segment.
    ///
    /// # Safety
    ///
    /// See [`get_mut`](Frame::segments_mut).
    #[inline]
    pub unsafe fn data_mut<'a>(&'a self, desc: &'a mut FrameDesc) -> DataMut<'a> {
        // SAFETY: see `frame_mut`.
        unsafe { self.mem.data_mut(desc) }
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
pub struct FrameLayout {
    xdp_headroom: usize,
    frame_headroom: usize,
    mtu: usize,
}

impl FrameLayout {
    fn frame_size(&self) -> usize {
        self.xdp_headroom + self.frame_headroom + self.mtu
    }
}

impl From<UmemConfig> for FrameLayout {
    fn from(c: UmemConfig) -> Self {
        Self {
            xdp_headroom: c.xdp_headroom() as usize,
            frame_headroom: c.frame_headroom() as usize,
            mtu: c.mtu() as usize,
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn config_frame_size_equals_layout_frame_size() {
        todo!()
    }
}
