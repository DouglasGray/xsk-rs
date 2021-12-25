use std::{fmt, sync::Arc};

use crate::ring::XskRingCons;

use super::{
    frame::{Frame, FrameDesc},
    UmemInner,
};

/// Used to transfer ownership of [`Umem`](super::Umem) frames from
/// kernel-space to user-space.
///
/// Frames received in this queue are those that have been sent via
/// the [`TxQueue`](crate::socket::TxQueue).
///
/// For more information see the
/// [docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html#umem-completion-ring).
pub struct CompQueue {
    ring: XskRingCons,
    _umem: Arc<UmemInner>,
}

impl CompQueue {
    pub(crate) fn new(ring: XskRingCons, umem: Arc<UmemInner>) -> Self {
        Self { ring, _umem: umem }
    }

    /// Update `frames` with frames whose contents have been sent
    /// (after submission via the [`TxQueue`](crate::socket::TxQueue))
    /// and may now be used again. Returns the number of elements of
    /// `frames` which have been updated.
    ///
    /// The number of entries updated will be less than or equal to
    /// the length of `frames`. Entries will be updated sequentially
    /// from the start of `frames` until the end.
    ///
    /// Free frames should be added back on to either the
    /// [`FillQueue`](crate::FillQueue) for data receipt or the
    /// [`TxQueue`](crate::TxQueue) for data transmission.
    ///
    /// # Safety
    ///
    /// The frames passed to this queue must belong to the same
    /// [`Umem`](super::Umem) that this `CompQueue` instance is tied
    /// to.
    #[inline]
    pub unsafe fn consume(&mut self, frames: &mut [Frame]) -> usize {
        let nb = frames.len() as u64;

        if nb == 0 {
            return 0;
        }

        let mut idx: u32 = 0;

        let cnt = unsafe { libbpf_sys::_xsk_ring_cons__peek(self.ring.as_mut(), nb, &mut idx) };

        if cnt > 0 {
            let mut data_desc = FrameDesc::default();

            for frame in frames.iter_mut().take(cnt as usize) {
                let addr: u64 =
                    unsafe { *libbpf_sys::_xsk_ring_cons__comp_addr(self.ring.as_ref(), idx) };

                data_desc.addr = addr as usize;
                data_desc.len = 0;
                data_desc.options = 0;

                // SAFETY: unsafe contract of this function guarantees
                // this frame belongs to the same UMEM as this queue,
                // so descriptor values will be valid.
                unsafe { frame.set_desc(&data_desc) };

                idx += 1;
            }

            unsafe { libbpf_sys::_xsk_ring_cons__release(self.ring.as_mut(), cnt) };
        }

        cnt as usize
    }
}

unsafe impl Send for CompQueue {}

impl fmt::Debug for CompQueue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CompQueue").finish()
    }
}
