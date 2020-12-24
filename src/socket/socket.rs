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

#[derive(Debug)]
pub enum SocketCreateError {
    InvalidIfName(NulError),
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
/// More details can be found in the [docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html)
pub struct Socket<'umem> {
    inner: Box<xsk_socket>,
    _marker: PhantomData<&'umem ()>,
}

/// The transmitting side of an AF_XDP socket.
///
/// More details can be found in the [docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html#tx-ring).
pub struct TxQueue<'umem> {
    inner: Box<xsk_ring_prod>,
    fd: Fd,
    _socket: Arc<Socket<'umem>>,
}

unsafe impl Send for TxQueue<'_> {}

/// The receiving side of an AF_XDP socket.
///
/// More details can be found in the [docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html#rx-ring).
pub struct RxQueue<'umem> {
    inner: Box<xsk_ring_cons>,
    fd: Fd,
    _socket: Arc<Socket<'umem>>,
}

unsafe impl Send for RxQueue<'_> {}

impl Socket<'_> {
    /// Create and bind a new AF_XDP socket to a given interface and queue id.
    ///
    /// May require root permissions to create and bind.
    pub fn new<'a, 'umem>(
        config: Config,
        umem: &mut Umem<'umem>,
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

        let if_name = CString::new(if_name).map_err(|e| SocketCreateError::InvalidIfName(e))?;

        let err = unsafe {
            libbpf_sys::xsk_socket__create(
                &mut xsk_ptr,
                if_name.as_ptr(),
                queue_id,
                umem.as_mut_ptr(),
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
            _socket: socket,
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
    /// Populate `descs` with information on packets received on the Rx ring.
    ///
    /// The number of entries updated will be less than or equal to the length of `descs`.
    /// Entries will be updated sequentially from the start of `descs` until the end.
    /// Returns the number of elements of `descs` which have been updated with received
    /// packet information, namely their frame address and length.
    ///
    /// Once the contents of the consumed frames have been dealt with and are no longer
    /// required, the frames should be added back on to either the
    /// [FillQueue](struct.FillQueue.html) or the [TxQueue](struct.TxQueue.html).
    pub fn consume(&mut self, descs: &mut [FrameDesc]) -> usize {
        // usize <-> u64 'as' conversions are ok as the crate's top level conditional
        // compilation flags (see lib.rs) guarantee that size_of<usize> = size_of<u64>
        let nb = descs.len() as u64;

        if nb == 0 {
            return 0;
        }

        let mut idx: u32 = 0;

        let cnt = unsafe { libbpf_sys::_xsk_ring_cons__peek(self.inner.as_mut(), nb, &mut idx) };

        if cnt > 0 {
            // Assuming 64-bit so u64 -> usize is ok
            for desc in descs.iter_mut().take(cnt.try_into().unwrap()) {
                unsafe {
                    let recv_pkt_desc =
                        libbpf_sys::_xsk_ring_cons__rx_desc(self.inner.as_mut(), idx);

                    desc.set_addr((*recv_pkt_desc).addr as usize);
                    desc.set_len((*recv_pkt_desc).len as usize);
                    desc.set_options((*recv_pkt_desc).options);
                }
                idx += 1;
            }

            unsafe { libbpf_sys::_xsk_ring_cons__release(self.inner.as_mut(), cnt) };
        }

        cnt.try_into().unwrap()
    }

    /// Same as `consume` but poll first to check if there is anything to read beforehand.
    pub fn poll_and_consume(
        &mut self,
        descs: &mut [FrameDesc],
        poll_timeout: i32,
    ) -> io::Result<usize> {
        match poll::poll_read(&mut self.fd(), poll_timeout)? {
            true => Ok(self.consume(descs)),
            false => Ok(0),
        }
    }

    /// Return the AF_XDP socket's file descriptor.
    ///
    /// Required for [poll_read](socket/poll/fn.poll_read.html)
    /// or [poll_write](socket/poll/fn.poll_write.html).
    pub fn fd(&mut self) -> &mut Fd {
        &mut self.fd
    }
}

impl TxQueue<'_> {
    /// Let the kernel know that the contents of frames in `descs` are ready to be transmitted.
    ///
    /// Note that if the length of `descs` is greater than the number of available spaces on the
    /// underlying ring buffer then no frames at all will be submitted for transmission.
    ///
    /// This function returns the number of frames submitted to the kernel for transmission. Due
    /// to the constraint mentioned in the above paragraph, this should always be the length of
    /// `descs` or `0`.
    ///
    /// Once the frames have been submitted they should not be used again until consumed again
    /// via the [CompQueue](struct.CompQueue.html)
    pub fn produce(&mut self, descs: &[FrameDesc]) -> usize {
        // usize <-> u64 'as' conversions are ok as the crate's top level conditional
        // compilation flags (see lib.rs) guarantee that size_of<usize> = size_of<u64>
        let nb: u64 = descs.len().try_into().unwrap();

        if nb == 0 {
            return 0;
        }

        let mut idx: u32 = 0;

        let cnt = unsafe { libbpf_sys::_xsk_ring_prod__reserve(self.inner.as_mut(), nb, &mut idx) };

        if cnt > 0 {
            for desc in descs.iter().take(cnt.try_into().unwrap()) {
                unsafe {
                    let send_pkt_desc =
                        libbpf_sys::_xsk_ring_prod__tx_desc(self.inner.as_mut(), idx);

                    (*send_pkt_desc).addr = desc.addr() as u64;
                    (*send_pkt_desc).len = desc.len() as u32; // Ok as desc.len() = frame_size: u32
                    (*send_pkt_desc).options = desc.options();
                }

                idx += 1;
            }

            unsafe { libbpf_sys::_xsk_ring_prod__submit(self.inner.as_mut(), cnt) };
        }

        cnt.try_into().unwrap()
    }

    /// Same as `produce` but wake up the kernel to continue processing
    /// produced frames (if required).
    ///
    /// For more details see the [docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html#xdp-use-need-wakeup-bind-flag).
    pub fn produce_and_wakeup(&mut self, descs: &[FrameDesc]) -> io::Result<usize> {
        let cnt = self.produce(descs);

        if self.needs_wakeup() {
            self.wakeup()?;
        }

        Ok(cnt)
    }

    /// Wake up the kernel to continue processing produced frames.
    ///
    /// See `produce_and_wakeup` for link to docs with further explanation.
    pub fn wakeup(&self) -> io::Result<()> {
        let ret =
            unsafe { libc::sendto(self.fd.id(), ptr::null(), 0, MSG_DONTWAIT, ptr::null(), 0) };

        if ret < 0 {
            match util::get_errno() {
                ENOBUFS | EAGAIN | EBUSY | ENETDOWN => (),
                _ => return Err(io::Error::last_os_error()),
            }
        }

        Ok(())
    }

    /// Check if the libbpf `NEED_WAKEUP` flag is set on the Tx ring.
    /// If so then this means a call to `wakeup` will be required to
    /// continue processing produced frames.
    ///
    /// See `produce_and_wakeup` for link to docs with further explanation.
    pub fn needs_wakeup(&self) -> bool {
        unsafe { libbpf_sys::_xsk_ring_prod__needs_wakeup(self.inner.as_ref()) != 0 }
    }

    /// Return the AF_XDP socket's file descriptor.
    ///
    /// Required for [poll_read](socket/poll/fn.poll_read.html)
    /// or [poll_write](socket/poll/fn.poll_write.html).
    pub fn fd(&mut self) -> &mut Fd {
        &mut self.fd
    }
}
