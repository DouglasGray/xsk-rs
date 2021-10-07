use libbpf_sys::{xsk_ring_cons, xsk_ring_prod, xsk_socket, xsk_socket_config};
use libc::{EAGAIN, EBUSY, ENETDOWN, ENOBUFS, MSG_DONTWAIT};
use std::{
    convert::TryInto,
    error::Error,
    ffi::{CString, NulError},
    fmt, io,
    marker::PhantomData,
    mem::MaybeUninit,
    ptr,
    sync::Arc,
};

use crate::{
    umem::{FrameDesc, Umem},
    util,
};

use super::{config::Config, fd::Fd, poll};
use crate::umem::Frame;
use std::convert::TryFrom;

#[derive(Debug)]
pub enum SocketCreateError {
    /// Null byte found in the provided interface name.
    InvalidIfName(NulError),
    /// `context` provides some more information at what point in the
    /// socket build process the OS error occurred.
    OsError {
        context: &'static str,
        io_err: io::Error,
    },
}

impl fmt::Display for SocketCreateError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use SocketCreateError::*;
        match self {
            InvalidIfName(_) => write!(f, "interface name contains null bytes"),
            OsError { context, .. } => write!(f, "OS or ffi call failed: {}", context),
        }
    }
}

impl Error for SocketCreateError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        use SocketCreateError::*;
        match self {
            InvalidIfName(nul_err) => Some(nul_err),
            OsError { io_err, .. } => Some(io_err),
        }
    }
}

/// An AF_XDP socket.
///
/// More details can be found in the
/// [docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html)
pub struct Socket<'umem> {
    inner: Box<xsk_socket>,
    umem: Arc<Umem<'umem>>,
    _marker: PhantomData<&'umem ()>,
}

/// The transmitting side of an AF_XDP socket.
///
/// More details can be found in the
/// [docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html#tx-ring).
pub struct TxQueue<'umem> {
    inner: Box<xsk_ring_prod>,
    fd: Fd,
    _socket: Arc<Socket<'umem>>,
}

unsafe impl Send for TxQueue<'_> {}

/// The receiving side of an AF_XDP socket.
///
/// More details can be found in the
/// [docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html#rx-ring).
pub struct RxQueue<'umem> {
    inner: Box<xsk_ring_cons>,
    fd: Fd,
    socket: Arc<Socket<'umem>>,
}

unsafe impl Send for RxQueue<'_> {}

impl Socket<'_> {
    /// Create and bind a new AF_XDP socket to a given interface and
    /// queue id.
    ///
    /// May require root permissions to create and bind.
    pub fn new<'a, 'umem>(
        config: Config,
        umem: Arc<Umem<'umem>>,
        if_name: &'a str,
        queue_id: u32,
    ) -> Result<(TxQueue<'umem>, RxQueue<'umem>), SocketCreateError> {
        let socket_create_config = xsk_socket_config {
            rx_size: config.rx_queue_size(),
            tx_size: config.tx_queue_size(),
            xdp_flags: config.xdp_flags().bits(),
            bind_flags: config.bind_flags().bits(),
            libbpf_flags: config.libbpf_flags().bits(),
        };

        let mut xsk_ptr: *mut xsk_socket = ptr::null_mut();
        let mut tx_q_ptr: MaybeUninit<xsk_ring_prod> = MaybeUninit::uninit();
        let mut rx_q_ptr: MaybeUninit<xsk_ring_cons> = MaybeUninit::uninit();

        let if_name = CString::new(if_name).map_err(SocketCreateError::InvalidIfName)?;

        let err = unsafe {
            libbpf_sys::xsk_socket__create(
                &mut xsk_ptr,
                if_name.as_ptr(),
                queue_id,
                (*umem).as_ptr() as *mut _, // ToDo: check safety
                rx_q_ptr.as_mut_ptr(),
                tx_q_ptr.as_mut_ptr(),
                &socket_create_config,
            )
        };

        if err != 0 {
            return Err(SocketCreateError::OsError {
                context: "failed to create AF_XDP socket via xsk_socket__create()",
                io_err: io::Error::from_raw_os_error(err),
            });
        }

        let fd = unsafe { libbpf_sys::xsk_socket__fd(xsk_ptr) };

        if fd < 0 {
            unsafe {
                libbpf_sys::xsk_socket__delete(xsk_ptr);
            }

            return Err(SocketCreateError::OsError {
                context: "could not retrieve AF_XDP socket file descriptor via xsk_socket__fd()",
                io_err: io::Error::from_raw_os_error(err),
            });
        }

        let fd = Fd::new(fd);

        let socket = Arc::new(Socket {
            inner: unsafe { Box::from_raw(xsk_ptr) },
            umem,
            _marker: PhantomData,
        });

        let tx_queue = TxQueue {
            inner: unsafe { Box::new(tx_q_ptr.assume_init()) },
            fd: fd.clone(),
            _socket: Arc::clone(&socket),
        };

        let rx_queue = RxQueue {
            inner: unsafe { Box::new(rx_q_ptr.assume_init()) },
            fd,
            socket,
        };

        Ok((tx_queue, rx_queue))
    }
}

impl Drop for Socket<'_> {
    fn drop(&mut self) {
        unsafe {
            libbpf_sys::xsk_socket__delete(self.inner.as_mut());
        }
    }
}

impl RxQueue<'_> {
    /// Populate `descs` with information on packets received on the
    /// rx ring.
    ///
    /// The number of entries updated will be less than or equal to
    /// the length of `descs`.  Entries will be updated sequentially
    /// from the start of `descs` until the end.  Returns the number
    /// of elements of `descs` which have been updated with received
    /// packet information, namely their frame address and length.
    ///
    /// Once the contents of the consumed frames have been dealt with
    /// and are no longer required, the frames should eventually be
    /// added back on to either the [FillQueue](struct.FillQueue.html)
    /// or the [TxQueue](struct.TxQueue.html).

    // ToDo: This could be renamed to receive?
    #[inline]
    pub fn consume(&mut self) -> Vec<Frame> {
        let mut start_idx: u32 = 0;

        // u64::MAX -> Try to get all frames
        let cnt = unsafe {
            libbpf_sys::_xsk_ring_cons__peek(self.inner.as_mut(), u64::MAX, &mut start_idx)
        };
        let start_idx = start_idx; // Remove mut
        let cnt: u32 = cnt.try_into().expect("size_t fits into usize");

        let mut frames = Vec::with_capacity(cnt.try_into().expect("u32 fits into a usize"));

        if cnt > 0 {
            let mmap_area = self.socket.umem.mmap_area();

            for idx in (0..cnt).map(|idx| idx + start_idx) {
                let mut desc = FrameDesc::new();
                unsafe {
                    let recv_pkt_desc =
                        libbpf_sys::_xsk_ring_cons__rx_desc(self.inner.as_mut(), idx);

                    desc.set_addr((*recv_pkt_desc).addr as usize);
                    desc.set_len((*recv_pkt_desc).len as usize);
                    desc.set_options((*recv_pkt_desc).options);
                }

                // ToDo: Is this detailed enough?
                // Safety: The kernel can only give us frames back that we previously gave to it via
                // the fill queue. Thus the desc we receive mut be unique.
                let frame = unsafe { Frame::new(Arc::clone(mmap_area), desc) };
                frames.push(frame);
            }

            unsafe { libbpf_sys::_xsk_ring_cons__release(self.inner.as_mut(), cnt.into()) };
        }

        frames
    }

    /// Same as `consume` but poll first to check if there is anything
    /// to read beforehand.
    #[inline]
    pub fn poll_and_consume(&mut self, poll_timeout: i32) -> io::Result<Vec<Frame>> {
        match poll::poll_read(&mut self.fd(), poll_timeout)? {
            true => Ok(self.consume()),
            false => Ok(vec![]),
        }
    }

    /// Return the AF_XDP socket's file descriptor.
    ///
    /// Required for [poll_read](socket/poll/fn.poll_read.html) or
    /// [poll_write](socket/poll/fn.poll_write.html).
    #[inline]
    pub fn fd(&mut self) -> &mut Fd {
        &mut self.fd
    }
}

impl TxQueue<'_> {
    /// Let the kernel know that the contents of frames in `descs` are
    /// ready to be transmitted.
    ///
    /// Note that if the length of `descs` is greater than the number
    /// of available spaces on the underlying ring buffer then no
    /// frames at all will be submitted for transmission.
    ///
    /// This function returns the number of frames submitted to the
    /// kernel for transmission. Due to the constraint mentioned in
    /// the above paragraph, this should always be the length of
    /// `descs` or `0`.
    ///
    /// # Safety
    ///
    /// This function is safe as the invariants on Frame assure that we only get valid non-aliasing
    /// frame descriptors.

    // ToDo: Maybe this could be renamed to transmit?
    #[inline]
    #[must_use = "produce() returns the frames that have not been sent, as there was not enough room. These should be tried again later!"]
    pub fn produce(&mut self, mut frames: Vec<Frame>) -> Vec<Frame> {
        if frames.is_empty() {
            return frames;
        }

        let num_frames = frames
            .len()
            .try_into()
            .expect("number of frames fits size_t");

        let mut start_idx: u32 = 0;
        // Safety: ToDo
        let cnt = unsafe {
            libbpf_sys::_xsk_ring_prod__reserve(self.inner.as_mut(), num_frames, &mut start_idx)
        };
        let start_idx = start_idx; // Remove mut
        let cnt: usize = cnt.try_into().expect("size_t fits into usize");

        assert!(
            u64::try_from(cnt).is_ok(),
            "Number of packets should fit into a u64"
        );
        assert!(
            cnt as u64 <= num_frames,
            "Kernel should at maximum return the number of frames we asked for"
        );

        let umem_mmap_area = self._socket.umem.mmap_area();

        if cnt > 0 {
            // ToDo: Check if drain is correct here
            for (idx, frame) in frames.drain(..cnt).enumerate() {
                assert!(
                    Arc::ptr_eq(frame.mmap_area(), umem_mmap_area),
                    "a `Socket` can only take `Frame`s pointing into its `Umem`"
                );

                // Safety: ToDo
                let idx: u32 = idx.try_into().expect("number of frames fits u32");
                let send_pkt_desc = unsafe {
                    libbpf_sys::_xsk_ring_prod__tx_desc(self.inner.as_mut(), start_idx + idx)
                };

                let desc = frame.frame_desc();
                // Safety: libbpf assures that this is a valid pointer until submit is called
                unsafe {
                    (*send_pkt_desc).addr = desc.addr() as u64;
                    (*send_pkt_desc).len = desc.len() as u32; // Ok as desc.len() = frame_size: u32
                    (*send_pkt_desc).options = desc.options();
                }

                // ToDo: Do something with frame instead of dropping to avoid atomic decrease in ARC on every send
            }

            // Safety: ToDo
            unsafe {
                libbpf_sys::_xsk_ring_prod__submit(self.inner.as_mut(), cnt as u64);
            }
        }

        frames
    }

    /// Same as `produce` but wake up the kernel to continue
    /// processing produced frames (if required).
    ///
    /// For more details see the
    /// [docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html#xdp-use-need-wakeup-bind-flag).
    #[inline]
    #[must_use = "produce_and_wakeup() returns the frames that have not been sent, as there was not enough room. These should be tried again later!"]
    pub fn produce_and_wakeup(&mut self, frames: Vec<Frame>) -> io::Result<Vec<Frame>> {
        let remaining = self.produce(frames);

        if self.needs_wakeup() {
            self.wakeup()?;
        }

        Ok(remaining)
    }

    /// Wake up the kernel to continue processing produced frames.
    ///
    /// See `produce_and_wakeup` for link to docs with further
    /// explanation.
    #[inline]
    pub fn wakeup(&self) -> io::Result<()> {
        let ret =
            unsafe { libc::sendto(self.fd.fd(), ptr::null(), 0, MSG_DONTWAIT, ptr::null(), 0) };

        if ret < 0 {
            match util::get_errno() {
                ENOBUFS | EAGAIN | EBUSY | ENETDOWN => (),
                _ => return Err(io::Error::last_os_error()),
            }
        }

        Ok(())
    }

    /// Check if the libbpf `NEED_WAKEUP` flag is set on the tx ring.
    /// If so then this means a call to `wakeup` will be required to
    /// continue processing produced frames.
    ///
    /// See `produce_and_wakeup` for link to docs with further
    /// explanation.
    #[inline]
    pub fn needs_wakeup(&self) -> bool {
        unsafe { libbpf_sys::_xsk_ring_prod__needs_wakeup(self.inner.as_ref()) != 0 }
    }

    /// Return the AF_XDP socket's file descriptor.
    ///
    /// Required for [poll_read](socket/poll/fn.poll_read.html) or
    /// [poll_write](socket/poll/fn.poll_write.html).
    #[inline]
    pub fn fd(&mut self) -> &mut Fd {
        &mut self.fd
    }
}
