use crate::ring::XskRingCons;

use super::{frame::FrameDesc, Umem};

/// Used to transfer ownership of [`Umem`](super::Umem) frames from
/// kernel-space to user-space.
///
/// Frames received in this queue are those that have been sent via
/// the [`TxQueue`](crate::socket::TxQueue).
///
/// For more information see the
/// [docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html#umem-completion-ring).
#[derive(Debug)]
pub struct CompQueue {
    ring: XskRingCons,
    _umem: Umem,
}

impl CompQueue {
    pub(crate) fn new(ring: XskRingCons, umem: Umem) -> Self {
        Self { ring, _umem: umem }
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
    #[inline]
    pub unsafe fn consume(&mut self, descs: &mut [FrameDesc]) -> usize {
        let nb = descs.len() as u32;

        if nb == 0 {
            return 0;
        }

        let mut idx = 0;

        let cnt = unsafe { libxdp_sys::xsk_ring_cons__peek(self.ring.as_mut(), nb, &mut idx) };

        if cnt > 0 {
            for desc in descs.iter_mut().take(cnt as usize) {
                let addr =
                    unsafe { *libxdp_sys::xsk_ring_cons__comp_addr(self.ring.as_ref(), idx) };

                desc.addr = addr as usize;
                desc.lengths.data = 0;
                desc.lengths.headroom = 0;
                desc.options = 0;

                idx += 1;
            }

            unsafe { libxdp_sys::xsk_ring_cons__release(self.ring.as_mut(), cnt) };
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

        let cnt = unsafe { libxdp_sys::xsk_ring_cons__peek(self.ring.as_mut(), 1, &mut idx) };

        if cnt > 0 {
            let addr = unsafe { *libxdp_sys::xsk_ring_cons__comp_addr(self.ring.as_ref(), idx) };

            desc.addr = addr as usize;
            desc.lengths.data = 0;
            desc.lengths.headroom = 0;
            desc.options = 0;

            unsafe { libxdp_sys::xsk_ring_cons__release(self.ring.as_mut(), cnt) };
        }

        cnt as usize
    }
}
