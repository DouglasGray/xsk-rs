//! Types for interacting with and creating a [`Umem`].

mod mem;
use mem::UmemRegion;

pub mod frame;
use frame::{Data, DataMut, FrameDesc, Headroom, HeadroomMut};

mod fill_queue;
pub use fill_queue::FillQueue;

mod comp_queue;
pub use comp_queue::CompQueue;

use libxdp_sys::{xsk_socket, xsk_umem};
use log::error;
use std::{
    error::Error,
    fmt, io,
    num::NonZeroU32,
    ptr::{self, NonNull},
    sync::{Arc, Mutex, PoisonError},
};

use crate::{
    config::UmemConfig,
    ring::{XskRingCons, XskRingConsHandle, XskRingProd, XskRingProdHandle},
};

/// The [`Umem`] pointer, along with everything libxdp dereferences
/// when deleting it.
///
/// When we create the [`Umem`] we pass it pointers to two rings - a
/// producer and consumer, representing the [`FillQueue`] and
/// [`CompQueue`] respectively. The `xsk_umem` C struct also keeps a
/// pair of pointers to these two queues and pops them when creating a
/// socket for the first time with this [`Umem`]. Hence we store them
/// here so we don't prematurely clear up the rings' memory between
/// creating the [`Umem`] and creating the socket.
///
/// `xsk_umem__delete` dereferences any such saved pair to work out
/// which memory to unmap, so it has to outlive the call, as does the
/// frame region. Keeping them here rather than beside this struct is
/// what guarantees they do: [`Drop::drop`] runs before any field is
/// dropped, whatever order the fields are written in.
#[derive(Debug)]
struct UmemInner {
    ptr: NonNull<xsk_umem>,
    saved_fq_and_cq: Option<(XskRingProd, XskRingCons)>,
    // Handles to the fill and comp rings libxdp kept hold of when
    // creating a socket. It stores those pointers on the context
    // shared by every socket bound to the same device and queue id,
    // and dereferences them when the last of those sockets is
    // deleted, so they must outlive every socket created from this
    // UMEM - which they do, since each socket holds a `Umem` handle.
    //
    // Nothing is ever removed: there is no way to tell from here that
    // a context has gone, so a pair is retained for every context the
    // UMEM ever creates rather than for every one it currently has.
    ring_handles: Vec<(XskRingProdHandle, XskRingConsHandle)>,
    // A second handle on the frame region, held only to keep it
    // mapped for the deletion.
    _mem: UmemRegion,
}

unsafe impl Send for UmemInner {}

impl UmemInner {
    /// # Safety
    ///
    /// Only one instance of this struct may exist for `ptr` since it
    /// deletes the UMEM as part of its [`Drop`] impl. If there are
    /// copies or clones of `ptr` then care must be taken to ensure
    /// they aren't used once this struct goes out of scope, and that
    /// they don't delete the UMEM themselves.
    unsafe fn new(
        ptr: NonNull<xsk_umem>,
        saved_fq_and_cq: (XskRingProd, XskRingCons),
        mem: UmemRegion,
    ) -> Self {
        Self {
            ptr,
            saved_fq_and_cq: Some(saved_fq_and_cq),
            ring_handles: Vec::new(),
            _mem: mem,
        }
    }

    fn as_mut_ptr(&self) -> *mut xsk_umem {
        self.ptr.as_ptr()
    }
}

impl Drop for UmemInner {
    fn drop(&mut self) {
        // SAFETY: unsafe constructor contract guarantees that the
        // UMEM has not been deleted already, and everything libxdp is
        // about to dereference is a field of this struct, so none of
        // it has been dropped yet.
        let err = unsafe { libxdp_sys::xsk_umem__delete(self.as_mut_ptr()) };

        if err != 0 {
            error!(
                "failed to delete UMEM with error: {}",
                io::Error::from_raw_os_error(-err)
            );
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
                err: Some(e),
            }
        })?;

        let mut umem_ptr = ptr::null_mut();
        let fq = XskRingProd::default();
        let cq = XskRingCons::default();

        let err = unsafe {
            libxdp_sys::xsk_umem__create(
                &mut umem_ptr,
                mem.as_ptr(),
                mem.len() as u64,
                fq.as_ptr(),
                cq.as_ptr(),
                &config.into(),
            )
        };

        if err != 0 {
            return Err(UmemCreateError {
                reason: "non-zero error code returned when creating UMEM",
                err: Some(io::Error::from_raw_os_error(-err)),
            });
        }

        let Some(umem_ptr) = NonNull::new(umem_ptr) else {
            return Err(UmemCreateError {
                reason: "UMEM is null",
                err: None,
            });
        };

        let null_ring = if fq.is_ring_null() {
            Some("fill queue ring is null")
        } else if cq.is_ring_null() {
            Some("comp queue ring is null")
        } else {
            None
        };

        // SAFETY: this is the only `UmemInner` instance for this
        // pointer, and no other pointers to the UMEM exist.
        let inner = unsafe { UmemInner::new(umem_ptr, (fq, cq), mem.clone()) };

        if let Some(reason) = null_ring {
            // Reported from here rather than before the pair is
            // handed over so that dropping `inner` on the way out
            // deletes the UMEM instead of leaving it behind.
            return Err(UmemCreateError { reason, err: None });
        }

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
    /// create function a pointer to the UMEM along with the fill
    /// queue and completion queue rings to use, which are either
    /// those saved when the UMEM was created or a freshly allocated
    /// pair. What became of those rings is handed back, or the
    /// non-zero error code the create function returned, which must be
    /// the one produced by libxdp.
    ///
    /// Regarding the saved queues, this is a byproduct of how the
    /// UMEM is created in the C code and we save them here to avoid
    /// leaking memory.
    ///
    /// Wherever libxdp holds on to a ring it is given, a handle is
    /// retained for the lifetime of the UMEM, since the pointer is
    /// dereferenced again when the last socket using it is torn down.
    ///
    /// The saved pair is lent to the create function rather than taken
    /// from the UMEM, and only let go of once that function has
    /// succeeded, so `saved_fq_and_cq` holds a pair for exactly as long
    /// as libxdp's own `umem->fill_save` and `umem->comp_save` do. That
    /// has to stay true: handed a pair it did not save while one is
    /// still saved, libxdp copies the saved rings over the pair it was
    /// given.
    #[inline]
    pub(crate) fn with_ptr_and_fq_and_cq<F>(&self, f: F) -> Result<CtxRings, i32>
    where
        F: FnOnce(*mut xsk_umem, &mut XskRingProd, &mut XskRingCons) -> i32,
    {
        let mut inner = self.inner.lock().unwrap();

        // Handed over only when the UMEM has no saved pair, but
        // allocated either way so that the saved one can be borrowed
        // rather than moved out.
        let mut fresh = (XskRingProd::default(), XskRingCons::default());

        // Taken first because `as_mut_ptr` borrows the whole struct,
        // which the exclusive borrow of the rings below rules out.
        let umem_ptr = inner.as_mut_ptr();

        let (fq, cq) = inner.saved_fq_and_cq.as_mut().unwrap_or(&mut fresh);

        let err = f(umem_ptr, fq, cq);

        if err != 0 {
            // libxdp tears down any context it created before
            // failing, so it no longer holds these pointers - with
            // one exception. `umem->fill_save` and `umem->comp_save`
            // are only cleared on the success path, so a pair that
            // came from there is still referenced by the UMEM, and is
            // still where it was: both alive for `xsk_umem__delete`
            // and ready for a retry to hand libxdp the rings it is
            // expecting.
            return Err(err);
        }

        let (fq_null, cq_null) = (fq.is_ring_null(), cq.is_ring_null());

        // libxdp has cleared `umem->fill_save` and `umem->comp_save`,
        // so let go of the saved pair to match.
        let (fq, cq) = inner.saved_fq_and_cq.take().unwrap_or(fresh);

        // Two null rings mean libxdp put the socket on a context that
        // already had a pair, so it never stored these pointers and
        // there is nothing to keep alive. Any other combination may
        // still be dereferenced at teardown, so keep the pair.
        if !(fq_null && cq_null) {
            inner.ring_handles.push((fq.handle(), cq.handle()));
        }

        Ok(match (fq_null, cq_null) {
            (false, false) => CtxRings::New(fq, cq),
            (true, true) => CtxRings::Existing,
            _ => CtxRings::Mismatched,
        })
    }

    /// Deletes a socket created from this UMEM.
    ///
    /// Creating a socket and deleting one both walk and mutate
    /// libxdp's per-UMEM bookkeeping - `umem->refcount`, the list of
    /// contexts hanging off the UMEM and those contexts' own
    /// refcounts - none of which libxdp synchronises, so a delete has
    /// to take the same lock a create takes. A lost increment lets a
    /// context's rings be unmapped while another socket is still on
    /// it; a lost decrement leaves them mapped for good.
    ///
    /// # Safety
    ///
    /// `ptr` must point to a socket created from this UMEM that has
    /// not been deleted already, and everything libxdp dereferences
    /// during the deletion must still be alive.
    #[inline]
    pub(crate) unsafe fn delete_socket(&self, ptr: *mut xsk_socket) {
        // Panicking here would abort a drop that may already be
        // unwinding, so the lock is taken poisoned or not. Nothing
        // held under it can unwind, so poisoning is not expected in
        // the first place.
        let _guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);

        // SAFETY: guaranteed by this function's contract.
        unsafe { libxdp_sys::xsk_socket__delete(ptr) };
    }

    /// Returns a raw pointer to the beginning of the UMEM buffer
    /// (for use cases like memory mapping, e.g., WebAssembly)
    #[inline(always)]
    pub fn as_ptr(&self) -> *const u8 {
        self.mem.as_ptr() as *const u8
    }
}

/// What libxdp did with the fill and comp rings it was handed when
/// creating a socket.
#[derive(Debug)]
pub(crate) enum CtxRings {
    /// It set up a context with them, so they are this socket's to
    /// use and to hand on to any socket that later joins the same
    /// context.
    New(XskRingProd, XskRingCons),
    /// It put the socket on a context that already had a pair,
    /// leaving these rings untouched. The pair originally returned
    /// for that context is the one to use.
    Existing,
    /// It populated one ring and not the other, which it should never
    /// do.
    Mismatched,
}

/// Error detailing why [`Umem`] creation failed.
#[derive(Debug)]
pub struct UmemCreateError {
    reason: &'static str,
    err: Option<io::Error>,
}

impl fmt::Display for UmemCreateError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.reason)
    }
}

impl Error for UmemCreateError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.err.as_ref().map(|err| err as _)
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

    /// The offset from the start of the [`Umem`] of the frame that
    /// `addr` falls in.
    ///
    /// Masking rather than subtracting a fixed offset from `addr`, so
    /// that a frame is still located correctly when its packet does
    /// not start where [`Umem::new`] put it - which is the case for
    /// any frame an XDP program has called `bpf_xdp_adjust_head` on.
    /// The mask is valid because the kernel only accepts a power of
    /// two frame size for an aligned UMEM, which is the only kind this
    /// crate creates, and [`UmemConfigBuilder::build`] rejects
    /// anything else.
    ///
    /// [`UmemConfigBuilder::build`]: crate::config::UmemConfigBuilder::build
    #[inline]
    fn frame_start(&self, addr: usize) -> usize {
        debug_assert!(
            self.frame_size().is_power_of_two(),
            "the mask below only finds a frame when the frame size is a power of two"
        );

        addr & !(self.frame_size() - 1)
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

    fn test_layout() -> FrameLayout {
        FrameLayout {
            xdp_headroom: 256,
            frame_headroom: 64,
            mtu: 1728,
        }
    }

    #[test]
    fn frame_start_of_an_address_umem_new_assigns() {
        let layout = test_layout();
        let frame_size = layout.frame_size();

        for i in 0..4 {
            let addr = (i * frame_size) + layout.xdp_headroom + layout.frame_headroom;

            assert_eq!(layout.frame_start(addr), i * frame_size);
        }
    }

    #[test]
    fn frame_start_is_unchanged_when_the_packet_has_been_moved_back() {
        let layout = test_layout();
        let frame_size = layout.frame_size();

        // An XDP program can move the packet back as far as
        // `data_hard_start`, which sits at the end of the frame
        // headroom, so a shift is never more than the XDP headroom.
        for i in 0..4 {
            let addr = (i * frame_size) + layout.xdp_headroom + layout.frame_headroom;

            for shift in 0..=layout.xdp_headroom {
                assert_eq!(layout.frame_start(addr - shift), i * frame_size);
            }
        }
    }

    #[test]
    fn frame_start_recovers_the_frame_from_any_address_within_it() {
        let layout = test_layout();
        let frame_size = layout.frame_size();

        for i in 0..4 {
            for offset in 0..frame_size {
                assert_eq!(
                    layout.frame_start((i * frame_size) + offset),
                    i * frame_size
                );
            }
        }
    }
}
