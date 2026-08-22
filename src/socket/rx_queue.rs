use std::io;

use crate::{
    ring::{self, XskRingCons},
    umem::frame::FrameDesc,
};

use super::{Socket, fd::Fd};

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

    /// Update `descs` with information on which [`Umem`] frames have
    /// received packets. Returns the number of elements of `descs`
    /// which have been updated.
    ///
    /// The number of entries updated will be less than or equal to
    /// the length of `descs`. Entries will be updated sequentially
    /// from the start of `descs` until the end.
    ///
    /// Once the contents of the consumed frames have been dealt with
    /// and are no longer required, the frames should eventually be
    /// added back on to either the [`FillQueue`] or the [`TxQueue`].
    ///
    /// # Safety
    ///
    /// The frames passed to this queue must belong to the same
    /// [`Umem`] that this `RxQueue` instance is tied to.
    ///
    /// [`Umem`]: crate::Umem
    /// [`FillQueue`]: crate::FillQueue
    /// [`TxQueue`]: crate::TxQueue
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
                let recv_pkt_desc =
                    unsafe { libxdp_sys::xsk_ring_cons__rx_desc(self.ring.as_ptr(), idx) };

                unsafe {
                    desc.addr = (*recv_pkt_desc).addr;
                    desc.lengths.data = (*recv_pkt_desc).len;
                    desc.lengths.headroom = 0;
                    desc.options = (*recv_pkt_desc).options;
                }

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
            let recv_pkt_desc =
                unsafe { libxdp_sys::xsk_ring_cons__rx_desc(self.ring.as_ptr(), idx) };

            unsafe {
                desc.addr = (*recv_pkt_desc).addr;
                desc.lengths.data = (*recv_pkt_desc).len;
                desc.lengths.headroom = 0;
                desc.options = (*recv_pkt_desc).options;
            }

            unsafe { libxdp_sys::xsk_ring_cons__release(self.ring.as_ptr(), cnt) };
        }

        cnt as usize
    }

    /// Same as [`consume`] but poll first to check if there is
    /// anything to read beforehand.
    ///
    /// # Safety
    ///
    /// See [`consume`].
    ///
    /// [`consume`]: RxQueue::consume
    #[inline]
    pub unsafe fn poll_and_consume(
        &mut self,
        descs: &mut [FrameDesc],
        poll_timeout: i32,
    ) -> io::Result<usize> {
        match self.poll(poll_timeout)? {
            true => Ok(unsafe { self.consume(descs) }),
            false => Ok(0),
        }
    }

    /// Same as [`poll_and_consume`] but for a single frame descriptor.
    ///
    /// # Safety
    ///
    /// See [`consume`].
    ///
    /// [`poll_and_consume`]: Self::poll_and_consume
    /// [`consume`]: Self::consume
    #[inline]
    pub unsafe fn poll_and_consume_one(
        &mut self,
        desc: &mut FrameDesc,
        poll_timeout: i32,
    ) -> io::Result<usize> {
        match self.poll(poll_timeout)? {
            true => Ok(unsafe { self.consume_one(desc) }),
            false => Ok(0),
        }
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
