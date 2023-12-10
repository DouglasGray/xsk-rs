use libc::{EAGAIN, EBUSY, ENETDOWN, ENOBUFS, MSG_DONTWAIT};
use std::{io, os::unix::prelude::AsRawFd, ptr};

use crate::{ring::XskRingProd, umem::frame::FrameDesc, util};

use super::{fd::Fd, Socket};

/// The transmitting side of an AF_XDP [`Socket`].
///
/// More details can be found in the
/// [docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html#tx-ring).
#[derive(Debug)]
pub struct TxQueue {
    ring: XskRingProd,
    socket: Socket,
}

impl TxQueue {
    pub(super) fn new(ring: XskRingProd, socket: Socket) -> Self {
        Self { ring, socket }
    }

    /// Let the kernel know that the frames described by `descs` are
    /// ready to be transmitted. Returns the number of frames
    /// submitted to the kernel.
    ///
    /// Note that if the length of `descs` is greater than the number
    /// of available spaces on the underlying ring buffer then no
    /// frames at all will be submitted for transmission.
    ///
    /// Once the frames have been submitted to this queue they should
    /// not be used again until consumed via the [`CompQueue`].
    ///
    /// # Safety
    ///
    /// This function is unsafe as it is possible to cause a data race
    /// if used improperly. For example, by simultaneously submitting
    /// the same frame to this `TxQueue` and the [`FillQueue`].
    ///
    /// Furthermore, the frames passed to this queue must belong to
    /// the same [`Umem`] that this `TxQueue` instance is tied to.
    ///
    /// [`FillQueue`]: crate::FillQueue
    /// [`CompQueue`]: crate::CompQueue
    /// [`Umem`]: crate::Umem
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
                let send_pkt_desc =
                    unsafe { libxdp_sys::xsk_ring_prod__tx_desc(self.ring.as_mut(), idx) };

                // SAFETY: unsafe contract of this function guarantees
                // `desc` describes a frame belonging to the same UMEM as
                // this queue.
                unsafe { desc.write_xdp_desc(&mut *send_pkt_desc) };

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
            let send_pkt_desc =
                unsafe { libxdp_sys::xsk_ring_prod__tx_desc(self.ring.as_mut(), idx) };

            // SAFETY: unsafe contract of this function guarantees
            // `desc` describes a frame belonging to the same UMEM as
            // this queue.
            unsafe { desc.write_xdp_desc(&mut *send_pkt_desc) };

            unsafe { libxdp_sys::xsk_ring_prod__submit(self.ring.as_mut(), cnt) };
        }

        cnt as usize
    }

    /// Same as [`produce`] but wake up the kernel to continue
    /// processing produced frames (if required).
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
    pub unsafe fn produce_and_wakeup(&mut self, descs: &[FrameDesc]) -> io::Result<usize> {
        let cnt = unsafe { self.produce(descs) };

        if self.needs_wakeup() {
            self.wakeup()?;
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
    pub unsafe fn produce_one_and_wakeup(&mut self, desc: &FrameDesc) -> io::Result<usize> {
        let cnt = unsafe { self.produce_one(desc) };

        if self.needs_wakeup() {
            self.wakeup()?;
        }

        Ok(cnt)
    }

    /// Wake up the kernel to continue processing produced frames.
    ///
    /// See [`produce_and_wakeup`] for a link to docs with further
    /// explanation.
    ///
    /// [`produce_and_wakeup`]: Self::produce_and_wakeup
    #[inline]
    pub fn wakeup(&self) -> io::Result<()> {
        let ret = unsafe {
            libc::sendto(
                self.socket.fd.as_raw_fd(),
                ptr::null(),
                0,
                MSG_DONTWAIT,
                ptr::null(),
                0,
            )
        };

        if ret < 0 {
            match util::get_errno() {
                ENOBUFS | EAGAIN | EBUSY | ENETDOWN => (),
                _ => return Err(io::Error::last_os_error()),
            }
        }

        Ok(())
    }

    /// Check if the [`XDP_USE_NEED_WAKEUP`] flag is set on the tx
    /// ring. If so then this means a call to [`wakeup`] will be
    /// required to continue processing produced frames.
    ///
    /// See [`produce_and_wakeup`] for link to docs with further
    /// explanation.
    ///
    /// [`XDP_USE_NEED_WAKEUP`]: libxdp_sys::XDP_USE_NEED_WAKEUP
    /// [`wakeup`]: Self::wakeup
    /// [`produce_and_wakeup`]: Self::produce_and_wakeup
    #[inline]
    pub fn needs_wakeup(&self) -> bool {
        unsafe { libxdp_sys::xsk_ring_prod__needs_wakeup(self.ring.as_ref()) != 0 }
    }

    /// Polls the socket, returning `true` if it is ready to write.
    #[inline]
    pub fn poll(&mut self, poll_timeout: i32) -> io::Result<bool> {
        self.socket.fd.poll_write(poll_timeout)
    }

    /// A reference to the underlying [`Socket`]'s file descriptor.
    #[inline]
    pub fn fd(&self) -> &Fd {
        &self.socket.fd
    }

    /// A mutable reference to the underlying [`Socket`]'s file descriptor.
    #[inline]
    pub fn fd_mut(&mut self) -> &mut Fd {
        &mut self.socket.fd
    }
}
