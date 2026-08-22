use crate::{
    ring::{self, XskRingCons},
    socket::Socket,
};

use super::frame::FrameDesc;

/// Used to transfer ownership of [`Umem`](super::Umem) frames from
/// kernel-space to user-space.
///
/// Frames received in this queue are those that have been sent via
/// the [`TxQueue`](crate::socket::TxQueue).
///
/// Holding on to this queue keeps the socket it was created with
/// alive, since libxdp unmaps the comp ring only when the last socket
/// created from the same [`Umem`](super::Umem) and bound to the same
/// device and queue id is deleted. Dropping every other handle to
/// that socket therefore will not release the device; this queue and
/// the one returned alongside it have to go too.
///
/// For more information see the
/// [docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html#umem-completion-ring).
#[derive(Debug)]
pub struct CompQueue {
    ring: XskRingCons,
    _socket: Socket,
}

impl CompQueue {
    pub(crate) fn new(ring: XskRingCons, socket: Socket) -> Self {
        Self {
            ring,
            _socket: socket,
        }
    }

    /// Update `descs` with details of frames whose contents have been
    /// sent (after submission via the [`TxQueue`]) and may now be
    /// used again. Returns the number of elements of `descs` which
    /// have been updated.
    ///
    /// The number of entries updated will be less than or equal to
    /// the length of `descs`. Entries will be updated sequentially
    /// from the start of `descs` until the end.
    ///
    /// Free frames should eventually be added back on to either the
    /// [`FillQueue`] or the [`TxQueue`].
    ///
    /// # Safety
    ///
    /// The frames passed to this queue must belong to the same
    /// [`Umem`] that this `CompQueue` instance is tied to.
    ///
    /// [`TxQueue`]: crate::socket::TxQueue
    /// [`FillQueue`]: crate::FillQueue
    /// [`Umem`]: super::Umem
    #[inline]
    pub unsafe fn consume(&mut self, descs: &mut [FrameDesc]) -> usize {
        let nb = descs.len() as u32;

        if nb == 0 {
            return 0;
        }

        let mut idx = 0;

        let cnt = unsafe { libxdp_sys::xsk_ring_cons__peek(self.ring.as_ptr(), nb, &mut idx) };

        if cnt > 0 {
            for desc in descs.iter_mut().take(cnt as usize) {
                let addr =
                    unsafe { *libxdp_sys::xsk_ring_cons__comp_addr(self.ring.as_ptr(), idx) };

                desc.addr = addr;
                desc.lengths.data = 0;
                desc.lengths.headroom = 0;
                desc.options = 0;

                idx = idx.wrapping_add(1);
            }

            unsafe { libxdp_sys::xsk_ring_cons__release(self.ring.as_ptr(), cnt) };
        }

        cnt as usize
    }

    /// Same as [`consume`] but for a single frame descriptor.
    ///
    /// # Safety
    ///
    /// See [`consume`].
    ///
    /// [`consume`]: Self::consume
    #[inline]
    pub unsafe fn consume_one(&mut self, desc: &mut FrameDesc) -> usize {
        let mut idx = 0;

        let cnt = unsafe { libxdp_sys::xsk_ring_cons__peek(self.ring.as_ptr(), 1, &mut idx) };

        if cnt > 0 {
            let addr = unsafe { *libxdp_sys::xsk_ring_cons__comp_addr(self.ring.as_ptr(), idx) };

            desc.addr = addr;
            desc.lengths.data = 0;
            desc.lengths.headroom = 0;
            desc.options = 0;

            unsafe { libxdp_sys::xsk_ring_cons__release(self.ring.as_ptr(), cnt) };
        }

        cnt as usize
    }

    /// The number of entries ready to be consumed, up to `nb`.
    ///
    /// Answers from libxdp's cached producer position unless that
    /// cache is empty, in which case the current position is read.
    /// A cached position only ever trails the real one, so the count
    /// is never an overestimate.
    ///
    /// Note that passing the ring size does not give an exact count,
    /// since a non-empty cache is never refreshed. See
    /// [`nb_avail_exact`] for that.
    ///
    /// [`nb_avail_exact`]: Self::nb_avail_exact
    #[inline]
    pub fn nb_avail(&mut self, nb: u32) -> u32 {
        // SAFETY: the ring is initialised and `&mut self` excludes
        // any other access to it.
        unsafe { libxdp_sys::xsk_cons_nb_avail(self.ring.as_ptr(), nb) }
    }

    /// The number of entries ready to be consumed.
    ///
    /// Reads the current producer position rather than a cached one,
    /// so this can be used to watch a backlog without consuming it.
    /// See [`nb_avail`] for the cached count.
    ///
    /// [`nb_avail`]: Self::nb_avail
    #[inline]
    pub fn nb_avail_exact(&mut self) -> u32 {
        // SAFETY: the ring is initialised and `&mut self` excludes
        // any other access to it.
        unsafe { ring::cons_nb_avail_exact(self.ring.as_ptr()) }
    }
}
