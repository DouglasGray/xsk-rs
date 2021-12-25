use std::{fmt, io, sync::Arc};

use crate::{ring::XskRingProd, socket::Fd};

use super::{frame::Frame, UmemInner};

/// Used to transfer ownership of [`Umem`](super::Umem) frames from
/// user-space to kernel-space.
///
/// These frames will be used to receive packets, and will eventually
/// be returned via the [`RxQueue`](crate::socket::RxQueue).
///
/// For more information see the
/// [docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html#umem-fill-ring).
pub struct FillQueue {
    ring: XskRingProd,
    _umem: Arc<UmemInner>,
}

impl FillQueue {
    pub(crate) fn new(ring: XskRingProd, umem: Arc<UmemInner>) -> Self {
        Self { ring, _umem: umem }
    }

    /// Let the kernel know that the provided `frames` may be used to
    /// receive data. Returns the number of frames submitted to the
    /// kernel.
    ///
    /// Note that if the length of `frames` is greater than the number
    /// of available spaces on the underlying ring buffer then no
    /// frames at all will be handed over to the kernel.
    ///
    /// # Safety
    ///
    /// This function is unsafe as it is possible to cause a data race
    /// if used improperly. For example, by simultaneously submitting
    /// the same frame descriptor to this `FillQueue` and the
    /// [`TxQueue`](crate::TxQueue). Once the frames have been
    /// submitted to this queue they should not be used again until
    /// consumed via the [`RxQueue`](crate::RxQueue).
    ///
    /// Furthermore, the frames passed to this queue must belong to
    /// the same [`Umem`](super::Umem) that this instance is tied to.
    #[inline]
    pub unsafe fn produce(&mut self, frames: &[Frame]) -> usize {
        let nb = frames.len() as u64;

        if nb == 0 {
            return 0;
        }

        let mut idx: u32 = 0;

        let cnt = unsafe { libbpf_sys::_xsk_ring_prod__reserve(self.ring.as_mut(), nb, &mut idx) };

        if cnt > 0 {
            for frame in frames.iter().take(cnt as usize) {
                unsafe {
                    *libbpf_sys::_xsk_ring_prod__fill_addr(self.ring.as_mut(), idx) =
                        frame.addr() as u64
                };

                idx += 1;
            }

            unsafe { libbpf_sys::_xsk_ring_prod__submit(self.ring.as_mut(), cnt) };
        }

        cnt as usize
    }

    /// Same as [`produce`](FillQueue::produce) but wake up the kernel
    /// (if required) to let it know there are frames available that
    /// may be used to receive data.
    ///
    /// For more details see the
    /// [docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html#xdp-use-need-wakeup-bind-flag).
    ///
    /// # Safety
    ///
    /// See [`produce`](FillQueue::produce).
    #[inline]
    pub unsafe fn produce_and_wakeup(
        &mut self,
        frames: &[Frame],
        socket_fd: &mut Fd,
        poll_timeout: i32,
    ) -> io::Result<usize> {
        let cnt = unsafe { self.produce(frames) };

        if cnt > 0 && self.needs_wakeup() {
            self.wakeup(socket_fd, poll_timeout)?;
        }

        Ok(cnt)
    }

    /// Wake up the kernel to let it know it can continue using the
    /// fill ring to process received data.
    ///
    /// See [`produce_and_wakeup`](FillQueue::produce_and_wakeup) for
    /// link to docs with further explanation.
    #[inline]
    pub fn wakeup(&self, fd: &mut Fd, poll_timeout: i32) -> io::Result<()> {
        fd.poll_read(poll_timeout)?;
        Ok(())
    }

    /// Check if the
    /// [`XDP_USE_NEED_WAKEUP`](libbpf_sys::XDP_USE_NEED_WAKEUP) flag
    /// is set on the fill ring. If so then this means a call to
    /// [`wakeup`](FillQueue::wakeup) will be required to continue
    /// processing received data.
    ///
    /// See [`produce_and_wakeup`](FillQueue::produce_and_wakeup) for
    /// a link to docs with further explanation.
    #[inline]
    pub fn needs_wakeup(&self) -> bool {
        unsafe { libbpf_sys::_xsk_ring_prod__needs_wakeup(self.ring.as_ref()) != 0 }
    }
}

unsafe impl Send for FillQueue {}

impl fmt::Debug for FillQueue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FillQueue").finish()
    }
}
