use std::io;

use crate::{ring::XskRingCons, umem::frame::FrameDesc};

use super::{fd::Fd, Socket};

/// The receiving side of an AF_XDP [`Socket`].
///
/// More details can be found in the
/// [docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html#rx-ring).
#[derive(Debug)]
pub struct RxQueue {
    ring: XskRingCons,
    socket: Socket,
}

impl RxQueue {
    pub(super) fn new(ring: XskRingCons, socket: Socket) -> Self {
        Self { ring, socket }
    }

    /// Populate `frames` with information on packets received on the
    /// rx ring. Returns the number of elements of `frames` which have
    /// been updated with received packet information, namely their
    /// frame address and length.
    ///
    /// The number of entries updated will be less than or equal to
    /// the length of `frames`. Entries will be updated sequentially
    /// from the start of `frames` until the end.
    ///
    /// Once the contents of the consumed frames have been dealt with
    /// and are no longer required, the frames should eventually be
    /// added back on to either the [`FillQueue`](crate::FillQueue) or
    /// the [`TxQueue`](crate::TxQueue).
    ///
    /// # Safety
    ///
    /// The frames passed to this queue must belong to the same
    /// [`Umem`](super::Umem) that this `RxQueue` instance is tied to.
    #[inline]
    pub unsafe fn consume(&mut self, descs: &mut [FrameDesc]) -> usize {
        let nb = descs.len() as u64;

        if nb == 0 {
            return 0;
        }

        let mut idx = 0;

        let cnt = unsafe { libbpf_sys::_xsk_ring_cons__peek(self.ring.as_mut(), nb, &mut idx) };

        if cnt > 0 {
            for desc in descs.iter_mut().take(cnt as usize) {
                let recv_pkt_desc =
                    unsafe { libbpf_sys::_xsk_ring_cons__rx_desc(self.ring.as_ref(), idx) };

                unsafe {
                    desc.addr = (*recv_pkt_desc).addr as usize;
                    desc.lengths.data = (*recv_pkt_desc).len as usize;
                    desc.lengths.headroom = 0;
                    desc.options = (*recv_pkt_desc).options;
                }

                idx += 1;
            }

            unsafe { libbpf_sys::_xsk_ring_cons__release(self.ring.as_mut(), cnt) };
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
        frames: &mut [FrameDesc],
        poll_timeout: i32,
    ) -> io::Result<usize> {
        match self.poll(poll_timeout)? {
            true => Ok(unsafe { self.consume(frames) }),
            false => Ok(0),
        }
    }

    /// Polls the socket, returning `true` if there is data to read.
    #[inline]
    pub fn poll(&mut self, poll_timeout: i32) -> io::Result<bool> {
        self.socket.fd.poll_read(poll_timeout)
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
