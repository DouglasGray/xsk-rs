//! Types for interacting with and creating a [`Umem`].

mod mmap;
use mmap::{
    framed::{FrameLayout, FramedMmap},
    Mmap,
};

pub mod frame;
use frame::Frame;

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
    /// Requires that there are no other copies or clones of `ptr`.
    unsafe fn new(ptr: NonNull<xsk_umem>) -> Self {
        Self(ptr)
    }

    fn as_mut(&mut self) -> &mut xsk_umem {
        // Safety: ok as we own the only pointer to the UMEM.
        unsafe { self.0.as_mut() }
    }
}

impl Drop for XskUmem {
    fn drop(&mut self) {
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
pub(crate) struct UmemInner {
    umem: Mutex<XskUmem>,
    saved_fq_and_cq: Mutex<Option<(XskRingProd, XskRingCons)>>,
    mmap: Arc<Mmap>,
}

impl UmemInner {
    fn new(
        umem: XskUmem,
        saved_fq_and_cq: Option<(XskRingProd, XskRingCons)>,
        mmap: Arc<Mmap>,
    ) -> Self {
        Self {
            umem: Mutex::new(umem),
            saved_fq_and_cq: Mutex::new(saved_fq_and_cq),
            mmap,
        }
    }
}

unsafe impl Send for UmemInner {}

/// A region of virtual contiguous memory divided into equal-sized
/// frames. It provides the underlying working memory for an AF_XDP
/// [`Socket`](crate::socket::Socket).
pub struct Umem {
    inner: Arc<UmemInner>,
}

impl Umem {
    /// Create a new UMEM instance backed by an anonymous memory
    /// mapped region.
    pub fn new(
        config: UmemConfig,
        frame_count: NonZeroU32,
        use_huge_pages: bool,
    ) -> Result<(Self, Vec<Frame>), UmemCreateError> {
        let frame_size = config.frame_size().get() as usize;
        let frame_count = frame_count.get() as usize;

        let mmap_len = frame_size * frame_count;

        let mut mmap = Mmap::new(mmap_len, use_huge_pages).map_err(|e| UmemCreateError {
            reason: "failed to create underlying mmap region",
            err: e,
        })?;

        let mut umem_ptr = ptr::null_mut();
        let mut fq = XskRingProd::default();
        let mut cq = XskRingCons::default();

        let err = unsafe {
            libbpf_sys::xsk_umem__create(
                &mut umem_ptr,
                mmap.as_mut(),
                mmap_len as u64,
                fq.as_mut(),
                cq.as_mut(),
                &config.into(),
            )
        };

        let xsk_umem = match NonNull::new(umem_ptr) {
            Some(init_umem) => unsafe { XskUmem::new(init_umem) },
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

        let inner = UmemInner::new(xsk_umem, Some((fq, cq)), Arc::new(mmap));

        let xdp_headroom = XDP_PACKET_HEADROOM as usize;
        let frame_headroom = config.frame_headroom() as usize;
        let mtu = frame_size - (xdp_headroom + frame_headroom);

        let framed_mmap = FramedMmap::new(
            FrameLayout {
                xdp_headroom,
                frame_headroom,
                mtu,
            },
            Arc::clone(&inner.mmap),
        )
        .map_err(|e| UmemCreateError {
            reason: "failed to split underlying mmap region into frames",
            err: e,
        })?;

        let mut frame_descs: Vec<Frame> = Vec::with_capacity(frame_count);

        for i in 0..frame_count {
            let addr = (i * frame_size) + xdp_headroom + frame_headroom;

            frame_descs.push(unsafe { Frame::new(addr, mtu, framed_mmap.clone()) });
        }

        let umem = Umem {
            inner: Arc::new(inner),
        };

        Ok((umem, frame_descs))
    }

    #[inline]
    pub(crate) fn inner(&self) -> &Arc<UmemInner> {
        &self.inner
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
        F: FnMut(&mut xsk_umem, &mut Option<(XskRingProd, XskRingCons)>) -> T,
    {
        let mut umem = self.inner.umem.lock().unwrap();
        let saved_fq_and_cq = &mut self.inner.saved_fq_and_cq.lock().unwrap();

        f(umem.as_mut(), saved_fq_and_cq)
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
