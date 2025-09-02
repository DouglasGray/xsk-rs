//! Types for interacting with and creating a [`Umem`].

mod mem;
use mem::UmemRegion;

pub mod frame;
use frame::{Data, DataMut, FrameDesc, Headroom, HeadroomMut};

mod fill_queue;
pub use fill_queue::FillQueue;

mod comp_queue;
pub use comp_queue::CompQueue;

use libxdp_sys::xsk_umem;
use log::error;
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

/// Wrapper around a pointer to some [`Umem`].
#[derive(Debug)]
struct XskUmem(NonNull<xsk_umem>);

unsafe impl Send for XskUmem {}

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
        let err = unsafe { libxdp_sys::xsk_umem__delete(self.0.as_ptr()) };

        if err != 0 {
            error!(
                "failed to delete UMEM with error: {}",
                io::Error::from_raw_os_error(-err)
            );
        }
    }
}

/// Wraps the [`Umem`] pointer and any saved fill queue or comp queue
/// rings. These are required for creation of the socket.
///
/// When we create the [`Umem`] we pass it pointers to two rings - a
/// producer and consumer, representing the [`FillQueue`] and
/// [`CompQueue`] respectively. The `xsk_umem` C struct also keeps a
/// pair of pointers to these two queues and pops them when creating a
/// socket for the first time with this [`Umem`]. Hence we store them
/// here so we don't prematurely clear up the rings' memory between
/// creating the [`Umem`] and creating the socket.
#[derive(Debug)]
struct UmemInner {
    ptr: XskUmem,
    saved_fq_and_cq: Option<(Box<XskRingProd>, Box<XskRingCons>)>,
}

impl UmemInner {
    fn new(ptr: XskUmem, saved_fq_and_cq: Option<(Box<XskRingProd>, Box<XskRingCons>)>) -> Self {
        Self {
            ptr,
            saved_fq_and_cq,
        }
    }
}

/// A region of virtual contiguous memory divided into equal-sized
/// frames. It provides the underlying working memory for an AF_XDP
/// [`Socket`](crate::socket::Socket).
#[derive(Debug, Clone)]
pub struct Umem {
    // `inner` must appear before `mem` to ensure correct drop order.
    inner: Arc<Mutex<UmemInner>>,
    mem: UmemRegion,
}

impl Umem {
    /// Create a new `Umem` instance backed by an anonymous memory
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
        let mut fq: Box<XskRingProd> = Box::default();
        let mut cq: Box<XskRingCons> = Box::default();

        let err = unsafe {
            libxdp_sys::xsk_umem__create(
                &mut umem_ptr,
                mem.as_ptr(),
                mem.len() as u64,
                fq.as_mut().as_mut(), // double deref due to to Box
                cq.as_mut().as_mut(),
                &config.into(),
            )
        };

        if err != 0 {
            return Err(UmemCreateError {
                reason: "non-zero error code returned when creating UMEM",
                err: io::Error::from_raw_os_error(-err),
            });
        }

        let umem_ptr = match NonNull::new(umem_ptr) {
            Some(umem_ptr) => {
                // SAFETY: this is the only `XskUmem` instance for
                // this pointer, and no other pointers to the UMEM
                // exist.
                unsafe { XskUmem::new(umem_ptr) }
            }
            None => {
                return Err(UmemCreateError {
                    reason: "UMEM is null",
                    err: io::Error::from_raw_os_error(-err),
                });
            }
        };

        if fq.is_ring_null() {
            return Err(UmemCreateError {
                reason: "fill queue ring is null",
                err: io::Error::from_raw_os_error(-err),
            });
        };

        if cq.is_ring_null() {
            return Err(UmemCreateError {
                reason: "comp queue ring is null",
                err: io::Error::from_raw_os_error(-err),
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

    /// The headroom and packet data segments of the `Umem` frame
    /// pointed at by `desc`. Contents are read-only.
    ///
    /// # Safety
    ///
    /// `desc` must correspond to a frame belonging to this
    /// `Umem`. Passing the descriptor of another `Umem` is very
    /// likely to result in incorrect memory access, by either
    /// straddling frames or accessing memory outside the underlying
    /// `Umem` area.
    ///
    /// Furthermore, the memory region accessed must not be mutably
    /// accessed anywhere else at the same time, either in userspace
    /// or by the kernel. To ensure this, care should be taken not to
    /// use the frame after submission to either the [`TxQueue`] or
    /// [`FillQueue`] until received over the [`CompQueue`] or
    /// [`RxQueue`] respectively.
    ///
    /// [`TxQueue`]: crate::TxQueue
    /// [`RxQueue`]: crate::RxQueue
    #[inline]
    pub unsafe fn frame(&self, desc: &FrameDesc) -> (Headroom<'_>, Data<'_>) {
        // SAFETY: We know from the unsafe contract of this function that:
        // a. Accessing the headroom and data segment identified by
        // `desc` is valid, since it describes a frame in this UMEM.
        // b. This access is sound since there are no mutable
        // references to the headroom and data segments.
        unsafe { self.mem.frame(desc) }
    }

    /// The headroom segment of the `Umem` frame pointed at by
    /// `desc`. Contents are read-only.
    ///
    /// # Safety
    ///
    /// See [`frame`](Self::frame).
    #[inline]
    pub unsafe fn headroom(&self, desc: &FrameDesc) -> Headroom<'_> {
        // SAFETY: see `frame`.
        unsafe { self.mem.headroom(desc) }
    }

    /// The data segment of the `Umem` frame pointed at by
    /// `desc`. Contents are read-only.
    ///
    /// # Safety
    ///
    /// See [`frame`](Self::frame).
    #[inline]
    pub unsafe fn data(&self, desc: &FrameDesc) -> Data<'_> {
        // SAFETY: see `frame`.
        unsafe { self.mem.data(desc) }
    }

    /// The headroom and packet data segments of the `Umem` frame
    /// pointed at by `desc`. Contents are writeable.
    ///
    /// # Safety
    ///
    /// `desc` must correspond to a frame belonging to this
    /// `Umem`. Passing the descriptor of another `Umem` is very
    /// likely to result in incorrect memory access, by either
    /// straddling frames or accessing memory outside the underlying
    /// `Umem` area.
    ///
    /// Furthermore, the memory region accessed must not be mutably or
    /// immutably accessed anywhere else at the same time, either in
    /// userspace or by the kernel. To ensure this, care should be
    /// taken not to use the frame after submission to either the
    /// [`TxQueue`] or [`FillQueue`] until received over the
    /// [`CompQueue`] or [`RxQueue`] respectively.
    ///
    /// [`TxQueue`]: crate::TxQueue
    /// [`RxQueue`]: crate::RxQueue
    #[inline]
    pub unsafe fn frame_mut<'a>(
        &'a self,
        desc: &'a mut FrameDesc,
    ) -> (HeadroomMut<'a>, DataMut<'a>) {
        // SAFETY: We know from the unsafe contract of this function that:
        // a. Accessing the headroom and data segment identified by
        // `desc` is valid, since it describes a frame in this UMEM.
        // b. This access is sound since there are no other mutable or
        // immutable references to the headroom and data segments.
        unsafe { self.mem.frame_mut(desc) }
    }

    /// The headroom segment of the `Umem` frame pointed at by
    /// `desc`. Contents are writeable.
    ///
    /// # Safety
    ///
    /// See [`frame_mut`](Self::frame_mut).
    #[inline]
    pub unsafe fn headroom_mut<'a>(&'a self, desc: &'a mut FrameDesc) -> HeadroomMut<'a> {
        // SAFETY: see `frame_mut`.
        unsafe { self.mem.headroom_mut(desc) }
    }

    /// The data segment of the `Umem` frame pointed at by
    /// `desc`. Contents are writeable.
    ///
    /// # Safety
    ///
    /// See [`frame_mut`](Self::frame_mut).
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
        F: FnMut(*mut xsk_umem, &mut Option<(Box<XskRingProd>, Box<XskRingCons>)>) -> T,
    {
        let mut inner = self.inner.lock().unwrap();

        f(inner.ptr.as_mut_ptr(), &mut inner.saved_fq_and_cq)
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
    use std::convert::TryInto;

    use crate::config::{UmemConfigBuilder, XDP_UMEM_MIN_CHUNK_SIZE};

    use super::*;

    #[test]
    fn config_frame_size_equals_layout_frame_size() {
        let config = UmemConfigBuilder::new()
            .frame_headroom(512)
            .frame_size(XDP_UMEM_MIN_CHUNK_SIZE.try_into().unwrap())
            .build()
            .unwrap();

        let layout: FrameLayout = config.into();

        assert_eq!(config.frame_size().get() as usize, layout.frame_size())
    }
}
