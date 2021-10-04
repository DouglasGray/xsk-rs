use libbpf_sys::xsk_ring_cons;
use std::sync::Arc;

use super::{
    frame::{Frame, FrameDesc},
    UmemInner,
};

/// Used to transfer ownership of [`Umem`](super::Umem) frames from
/// kernel-space to user-space.
///
/// Frames received in this queue are those that have been sent via
/// the [`TxQueue`](super::TxQueue).
///
/// For more information see the
/// [docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html#umem-completion-ring)
pub struct CompQueue {
    ring: xsk_ring_cons,
    _umem: Arc<UmemInner>,
}

impl CompQueue {
    pub(crate) fn new(ring: xsk_ring_cons, umem: Arc<UmemInner>) -> Self {
        Self { ring, _umem: umem }
    }

    /// Update `frames` with frames whose contents have been sent
    /// (after submission via the [`TxQueue`](crate::socket::TxQueue) and may
    /// now be used again.
    ///
    /// The number of entries updated will be less than or equal to
    /// the length of `frames`. Entries will be updated sequentially
    /// from the start of `frames` until the end.
    ///
    /// Returns the number of elements of `frames` which have been
    /// updated.
    ///
    /// Free frames should be added back on to either the
    /// [`FillQueue`](super::FillQueue) for data receipt or the
    /// [`TxQueue`](crate::socket::TxQueue) for data transmission.
    ///
    /// # Safety
    ///
    /// The underlying [`Umem`](super::Umem) of the passed `frames`
    /// and this [`CompQueue`] must be the same.
    #[inline]
    pub unsafe fn consume(&mut self, frames: &mut [Frame]) -> usize {
        let nb = frames.len() as u64;

        if nb == 0 {
            return 0;
        }

        let mut idx: u32 = 0;

        let cnt = unsafe { libbpf_sys::_xsk_ring_cons__peek(&mut self.ring, nb, &mut idx) };

        if cnt > 0 {
            let mut data_desc = FrameDesc::default();

            for frame in frames.iter_mut().take(cnt as usize) {
                let addr: u64 =
                    unsafe { *libbpf_sys::_xsk_ring_cons__comp_addr(&mut self.ring, idx) };

                data_desc.addr = addr as usize;
                data_desc.len = 0;
                data_desc.options = 0;

                unsafe { frame.set_desc(&data_desc) };

                idx += 1;
            }

            unsafe { libbpf_sys::_xsk_ring_cons__release(&mut self.ring, cnt) };
        }

        cnt as usize
    }
}

unsafe impl Send for CompQueue {}
