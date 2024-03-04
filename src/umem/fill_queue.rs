use std::io;

use crate::{ring::XskRingProd, socket::Fd};

use super::{frame::FrameDesc, Umem};

/// Used to transfer ownership of [`Umem`](super::Umem) frames from
/// user-space to kernel-space.
///
/// These frames will be used to receive packets, and will eventually
/// be returned via the [`RxQueue`](crate::socket::RxQueue).
///
/// For more information see the
/// [docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html#umem-fill-ring).
#[derive(Debug)]
pub struct FillQueue {
    ring: XskRingProd,
    _umem: Umem,
}

impl FillQueue {
    pub(crate) fn new(ring: XskRingProd, umem: Umem) -> Self {
        Self { ring, _umem: umem }
    }

    /// Let the kernel know that the [`Umem`] frames described by
    /// `descs` may be used to receive data. Returns the number of
    /// frames submitted to the kernel.
    ///
    /// Note that if the length of `descs` is greater than the number
    /// of available spaces on the underlying ring buffer then no
    /// frames at all will be handed over to the kernel.
    ///
    /// Once the frames have been submitted to this queue they should
    /// not be used again until consumed via the [`RxQueue`].
    ///
    /// # Safety
    ///
    /// This function is unsafe as it is possible to cause a data race
    /// if used improperly. For example, by simultaneously submitting
    /// the same frame descriptor to this `FillQueue` and the
    /// [`TxQueue`].
    ///
    /// Furthermore, the frames passed to this queue must belong to
    /// the same [`Umem`] that this `FillQueue` instance is tied to.
    ///
    /// [`TxQueue`]: crate::TxQueue
    /// [`RxQueue`]: crate::RxQueue
    #[inline]
    pub unsafe fn produce(&mut self, descs: &[FrameDesc]) -> usize {
        let nb = descs.len() as u32;

        if nb == 0 {
            return 0;
        }

        let mut idx = 0;

        let cnt = unsafe { libxdp_sys::xsk_ring_prod__reserve(self.ring.as_mut(), nb, &mut idx) };

        if cnt > 0 {
            for desc in descs.iter().take(cnt as usize) {
                unsafe {
                    *libxdp_sys::xsk_ring_prod__fill_addr(self.ring.as_mut(), idx) =
                        desc.addr as u64
                };

                idx += 1;
            }

            unsafe { libxdp_sys::xsk_ring_prod__submit(self.ring.as_mut(), cnt) };
        }

        cnt as usize
    }

    /// Same as [`produce`] but for a single frame descriptor.
    ///
    /// # Safety
    ///
    /// See [`produce`].
    ///
    /// [`produce`]: Self::produce
    #[inline]
    pub unsafe fn produce_one(&mut self, desc: &FrameDesc) -> usize {
        let mut idx = 0;

        let cnt = unsafe { libxdp_sys::xsk_ring_prod__reserve(self.ring.as_mut(), 1, &mut idx) };

        if cnt > 0 {
            unsafe {
                *libxdp_sys::xsk_ring_prod__fill_addr(self.ring.as_mut(), idx) = desc.addr as u64
            };

            unsafe { libxdp_sys::xsk_ring_prod__submit(self.ring.as_mut(), cnt) };
        }

        cnt as usize
    }

    /// Same as [`produce`] but wake up the kernel if required to let
    /// it know there are frames available that may be used to receive
    /// data.
    ///
    /// For more details see the
    /// [docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html#xdp-use-need-wakeup-bind-flag).
    ///
    /// # Safety
    ///
    /// See [`produce`].
    ///
    /// [`produce`]: Self::produce
    #[inline]
    pub unsafe fn produce_and_wakeup(
        &mut self,
        descs: &[FrameDesc],
        socket_fd: &mut Fd,
        poll_timeout: i32,
    ) -> io::Result<usize> {
        let cnt = unsafe { self.produce(descs) };

        if cnt > 0 && self.needs_wakeup() {
            self.wakeup(socket_fd, poll_timeout)?;
        }

        Ok(cnt)
    }

    /// Same as [`produce_and_wakeup`] but for a single frame
    /// descriptor.
    ///
    /// # Safety
    ///
    /// See [`produce`].
    ///
    /// [`produce_and_wakeup`]: Self::produce_and_wakeup
    /// [`produce`]: Self::produce
    #[inline]
    pub unsafe fn produce_one_and_wakeup(
        &mut self,
        desc: &FrameDesc,
        socket_fd: &mut Fd,
        poll_timeout: i32,
    ) -> io::Result<usize> {
        let cnt = unsafe { self.produce_one(desc) };

        if cnt > 0 && self.needs_wakeup() {
            self.wakeup(socket_fd, poll_timeout)?;
        }

        Ok(cnt)
    }

    /// Wake up the kernel to let it know it can continue using the
    /// fill ring to process received data.
    ///
    /// See [`produce_and_wakeup`] for link to docs with further
    /// explanation.
    ///
    /// [`produce_and_wakeup`]: Self::produce_and_wakeup
    #[inline]
    pub fn wakeup(&self, fd: &mut Fd, poll_timeout: i32) -> io::Result<()> {
        fd.poll_read(poll_timeout)?;
        Ok(())
    }

    /// Check if the [`XDP_USE_NEED_WAKEUP`] flag is set on the fill
    /// ring. If so then this means a call to [`wakeup`] will be
    /// required to continue processing received data.
    ///
    /// See [`produce_and_wakeup`] for a link to docs with further
    /// explanation.
    ///
    /// [`produce_and_wakeup`]: Self::produce_and_wakeup
    /// [`XDP_USE_NEED_WAKEUP`]: libxdp_sys::XDP_USE_NEED_WAKEUP
    /// [`wakeup`]: Self::wakeup
    #[inline]
    pub fn needs_wakeup(&self) -> bool {
        unsafe { libxdp_sys::xsk_ring_prod__needs_wakeup(self.ring.as_ref()) != 0 }
    }
}
