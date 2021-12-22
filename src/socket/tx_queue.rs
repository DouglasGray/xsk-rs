use libc::{EAGAIN, EBUSY, ENETDOWN, ENOBUFS, MSG_DONTWAIT};
use std::{io, os::unix::prelude::AsRawFd, ptr, sync::Arc};

use crate::{ring::XskRingProd, umem::frame::Frame, util};

use super::{fd::Fd, Socket};

/// The transmitting side of an AF_XDP [`Socket`].
///
/// More details can be found in the
/// [docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html#tx-ring).
pub struct TxQueue {
    ring: XskRingProd,
    fd: Fd,
    _socket: Arc<Socket>,
}

unsafe impl Send for TxQueue {}

impl TxQueue {
    pub(super) fn new(ring: XskRingProd, socket: Arc<Socket>) -> Self {
        Self {
            ring,
            fd: socket.fd.clone(),
            _socket: socket,
        }
    }

    /// Let the kernel know that the contents of `frames` are ready to
    /// be transmitted.
    ///
    /// Note that if the length of `frames` is greater than the number
    /// of available spaces on the underlying ring buffer then no
    /// frames at all will be submitted for transmission.
    ///
    /// This function returns the number of frames submitted to the
    /// kernel for transmission. Due to the constraint mentioned in
    /// the above paragraph, this should always be the length of
    /// `frames` or `0`.
    ///
    /// # Safety
    ///
    /// This function is unsafe as it is possible to cause a data race
    /// by simultaneously submitting the same frame descriptor to the
    /// tx ring and the fill ring, for example.  Once any frames have
    /// been submitted they should not be used again until consumed
    /// via the [`CompQueue`](crate::umem::CompQueue).
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
                let send_pkt_desc =
                    unsafe { libbpf_sys::_xsk_ring_prod__tx_desc(self.ring.as_mut(), idx) };

                unsafe { frame.write_xdp_desc(&mut *send_pkt_desc) };

                idx += 1;
            }

            unsafe { libbpf_sys::_xsk_ring_prod__submit(self.ring.as_mut(), cnt) };
        }

        cnt as usize
    }

    /// Same as [`produce`](TxQueue::produce) but wake up the kernel
    /// to continue processing produced frames (if required).
    ///
    /// For more details see the
    /// [docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html#xdp-use-need-wakeup-bind-flag).
    ///
    /// # Safety
    ///
    /// See [`produce`](TxQueue::produce).
    #[inline]
    pub unsafe fn produce_and_wakeup(&mut self, frames: &[Frame]) -> io::Result<usize> {
        let cnt = unsafe { self.produce(frames) };

        if self.needs_wakeup() {
            self.wakeup()?;
        }

        Ok(cnt)
    }

    /// Wake up the kernel to continue processing produced frames.
    ///
    /// See [`produce_and_wakeup`](TxQueue::produce_and_wakeup) for a
    /// link to docs with further explanation.
    #[inline]
    pub fn wakeup(&self) -> io::Result<()> {
        let ret = unsafe {
            libc::sendto(
                self.fd.as_raw_fd(),
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

    /// Check if the
    /// [`XDP_USE_NEED_WAKEUP`](libbpf_sys::XDP_USE_NEED_WAKEUP) flag
    /// is set on the tx ring.  If so then this means a call to
    /// [`wakeup`](TxQueue::wakeup) will be required to continue
    /// processing produced frames.
    ///
    /// See [`produce_and_wakeup`](TxQueue::produce_and_wakeup) for
    /// link to docs with further explanation.
    #[inline]
    pub fn needs_wakeup(&self) -> bool {
        unsafe { libbpf_sys::_xsk_ring_prod__needs_wakeup(self.ring.as_ref()) != 0 }
    }

    /// Polls the socket, returning `true` if it is ready to write.
    #[inline]
    pub fn poll(&mut self, poll_timeout: i32) -> io::Result<bool> {
        self.fd.poll_write(poll_timeout)
    }

    /// The [`Socket`]'s file descriptor.
    #[inline]
    pub fn fd(&self) -> &Fd {
        &self.fd
    }

    #[inline]
    pub fn fd_mut(&mut self) -> &mut Fd {
        &mut self.fd
    }
}
