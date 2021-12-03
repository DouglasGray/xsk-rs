use libbpf_sys::xsk_ring_cons;
use std::{io, sync::Arc};

use crate::umem::frame::{Frame, FrameDesc};

use super::{
    fd::{Fd, PollEvent},
    Socket,
};

/// The receiving side of an AF_XDP [`Socket`].
///
/// More details can be found in the
/// [docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html#rx-ring).
pub struct RxQueue {
    ring: xsk_ring_cons,
    fd: Fd,
    _socket: Arc<Socket>,
}

unsafe impl Send for RxQueue {}

impl RxQueue {
    pub(super) fn new(ring: xsk_ring_cons, socket: Arc<Socket>) -> Self {
        Self {
            ring,
            fd: socket.fd,
            _socket: socket,
        }
    }

    /// Populate `frames` with information on packets received on the
    /// rx ring.
    ///
    /// The number of entries updated will be less than or equal to
    /// the length of `frames`. Entries will be updated sequentially
    /// from the start of `frames` until the end.
    ///
    /// Returns the number of elements of `frames` which have been
    /// updated with received packet information, namely their frame
    /// address and length.
    ///
    /// Once the contents of the consumed frames have been dealt with
    /// and are no longer required, the frames should eventually be
    /// added back on to either the [`FillQueue`](crate::FillQueue)
    /// or the [`TxQueue`](crate::TxQueue).
    ///
    /// # Safety
    ///
    /// The underlying [`Umem`](crate::Umem) of the passed `frames` and this
    /// [`RxQueue`] must be the same.
    #[inline]
    pub unsafe fn consume(&mut self, frames: &mut [Frame]) -> usize {
        let nb = frames.len() as u64;

        if nb == 0 {
            return 0;
        }

        let mut idx: u32 = 0;

        let cnt = unsafe { libbpf_sys::_xsk_ring_cons__peek(&mut self.ring, nb, &mut idx) };

        if cnt > 0 {
            let mut desc = FrameDesc::default();

            for frame in frames.iter_mut().take(cnt as usize) {
                let recv_pkt_desc = unsafe { libbpf_sys::_xsk_ring_cons__rx_desc(&self.ring, idx) };

                unsafe {
                    desc.addr = (*recv_pkt_desc).addr as usize;
                    desc.len = (*recv_pkt_desc).len as usize;
                    desc.options = (*recv_pkt_desc).options;
                }

                unsafe {
                    frame.set_desc(&desc);
                }

                idx += 1;
            }

            unsafe { libbpf_sys::_xsk_ring_cons__release(&mut self.ring, cnt) };
        }

        cnt as usize
    }

    /// Same as [`consume`](RxQueue::consume) but poll first to check
    /// if there is anything to read beforehand.
    ///
    /// # Safety
    ///
    /// See [`consume`](RxQueue::consume).
    #[inline]
    pub unsafe fn poll_and_consume(
        &mut self,
        frames: &mut [Frame],
        poll_timeout: i32,
    ) -> io::Result<usize> {
        match self.fd.poll(PollEvent::Read, poll_timeout)? {
            true => Ok(unsafe { self.consume(frames) }),
            false => Ok(0),
        }
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
