//! Types for creating and using an AF_XDP [`Socket`].

pub mod fd;
use fd::Fd;

mod rx_queue;
use libc::SOL_XDP;
pub use rx_queue::RxQueue;

mod tx_queue;
pub use tx_queue::TxQueue;

use libbpf_sys::{xdp_statistics, xsk_socket, XDP_STATISTICS};
use std::{
    borrow::Borrow,
    error::Error,
    fmt,
    io::{self, ErrorKind},
    mem,
    os::unix::prelude::AsRawFd,
    ptr::{self, NonNull},
    sync::Arc,
};

use crate::{
    config::{Interface, SocketConfig},
    ring::{XskRingCons, XskRingProd},
    umem::UmemInner,
};

use crate::umem::{CompQueue, FillQueue, Umem};

const XDP_STATISTICS_OPTLEN: u32 = mem::size_of::<xdp_statistics>() as u32;

/// Wrapper around a pointer to some AF_XDP socket. Guarantees that
/// the pointer is both non-null and unique.
struct XskSocket(NonNull<xsk_socket>);

impl XskSocket {
    /// # Safety
    ///
    /// Requires that there are no other copies or clones of `ptr`.
    unsafe fn new(ptr: NonNull<xsk_socket>) -> Self {
        Self(ptr)
    }
}

impl Drop for XskSocket {
    fn drop(&mut self) {
        unsafe {
            libbpf_sys::xsk_socket__delete(self.0.as_mut());
        }
    }
}

/// An AF_XDP socket.
///
/// More details can be found in the
/// [docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html)
pub struct Socket {
    fd: Fd,
    _inner: XskSocket,
    _umem: Arc<UmemInner>,
}

impl Socket {
    /// Create and bind a new AF_XDP socket to a given interface and
    /// queue id using the underlying UMEM.
    ///
    /// May require root permissions to create successfully.
    ///
    /// Whether you can expect the returned `Option<(FillQueue,
    /// CompQueue)>` to be [`Some`](Option::Some) or
    /// [`None`](Option::None) depends on a couple of things:
    ///
    ///  1. If the [`Umem`] is currently shared (i.e. being used for
    ///  >=1 AF_XDP sockets elsewhere):
    ///
    ///    - If the `(if_name, queue_id)` pair is not bound to,
    ///    expect [`Some`](Option::Some).
    ///
    ///    - If the `(if_name, queue_id)` pair is bound to, expect
    ///    [`None`](Option::None) and use the [`FillQueue`] and
    ///    [`CompQueue`] originally returned for this pair.
    ///
    ///  2. If the [`Umem`] is not currently shared, expect
    ///  [`Some`](Option::Some).
    ///
    /// For further details on using a shared [`Umem`] please see the
    /// [docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html#xdp-shared-umem-bind-flag).
    #[allow(clippy::new_ret_no_self)]
    #[allow(clippy::type_complexity)]
    pub fn new(
        config: SocketConfig,
        umem: &Umem,
        if_name: &Interface,
        queue_id: u32,
    ) -> Result<(TxQueue, RxQueue, Option<(FillQueue, CompQueue)>), SocketCreateError> {
        let mut xsk_ptr = ptr::null_mut();
        let mut tx_q = XskRingProd::default();
        let mut rx_q = XskRingCons::default();

        let (err, fq, cq) = unsafe {
            umem.with_ptr_and_saved_queues(|xsk_umem, saved_fq_and_cq| {
                let (mut fq, mut cq) = saved_fq_and_cq
                    .take()
                    .unwrap_or_else(|| (XskRingProd::default(), XskRingCons::default()));

                let err = libbpf_sys::xsk_socket__create_shared(
                    &mut xsk_ptr,
                    if_name.as_cstr().as_ptr(),
                    queue_id,
                    xsk_umem,
                    rx_q.as_mut(),
                    tx_q.as_mut(),
                    fq.as_mut(),
                    cq.as_mut(),
                    &config.into(),
                );

                (err, fq, cq)
            })
        };

        let xsk_socket = match NonNull::new(xsk_ptr) {
            Some(init_xsk) => unsafe { XskSocket::new(init_xsk) },
            None => {
                return Err(SocketCreateError {
                    reason: "returned socket pointer was null",
                    err: io::Error::from_raw_os_error(err),
                });
            }
        };

        if err != 0 {
            return Err(SocketCreateError {
                reason: "non-zero error code returned when creating AF_XDP socket",
                err: io::Error::from_raw_os_error(err),
            });
        }

        let fd = unsafe { libbpf_sys::xsk_socket__fd(xsk_socket.0.as_ref()) };

        if fd < 0 {
            return Err(SocketCreateError {
                reason: "failed to retrieve AF_XDP socket file descriptor",
                err: io::Error::from_raw_os_error(err),
            });
        }

        let socket = Arc::new(Socket {
            fd: Fd::new(fd),
            _inner: xsk_socket,
            _umem: Arc::clone(umem.inner()),
        });

        let tx_q = if tx_q.is_ring_null() {
            return Err(SocketCreateError {
                reason: "returned tx queue ring is null",
                err: io::Error::from_raw_os_error(err),
            });
        } else {
            TxQueue::new(tx_q, Arc::clone(&socket))
        };

        let rx_q = if rx_q.is_ring_null() {
            return Err(SocketCreateError {
                reason: "returned rx queue ring is null",
                err: io::Error::from_raw_os_error(err),
            });
        } else {
            RxQueue::new(rx_q, Arc::clone(&socket))
        };

        let fq_and_cq = match (fq.is_ring_null(), cq.is_ring_null()) {
            (true, true) => None,
            (false, false) => {
                let fq = FillQueue::new(fq, Arc::clone(umem.inner()));
                let cq = CompQueue::new(cq, Arc::clone(umem.inner()));

                Some((fq, cq))
            }
            _ => {
                return Err(SocketCreateError {
                    reason: "fill queue xor comp queue ring is null, either both or neither should be non-null",
                    err: io::Error::from_raw_os_error(err),
                });
            }
        };

        Ok((tx_q, rx_q, fq_and_cq))
    }
}

/// Error detailing why [`Socket`] creation failed.
#[derive(Debug)]
pub struct SocketCreateError {
    reason: &'static str,
    err: io::Error,
}

impl fmt::Display for SocketCreateError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.reason)
    }
}

impl Error for SocketCreateError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.err.borrow())
    }
}

/// AF_XDP socket statistics.
#[derive(Default, Debug, Clone, Copy)]
pub struct XdpStatistics(xdp_statistics);

impl XdpStatistics {
    #[inline]
    fn retrieve(fd: &Fd) -> io::Result<XdpStatistics> {
        let mut stats = xdp_statistics::default();

        let mut optlen = XDP_STATISTICS_OPTLEN;

        let err = unsafe {
            libc::getsockopt(
                fd.as_raw_fd(),
                SOL_XDP,
                XDP_STATISTICS as i32,
                &mut stats as *mut _ as *mut libc::c_void,
                &mut optlen,
            )
        };

        if err != 0 {
            return Err(io::Error::from_raw_os_error(err));
        }

        if optlen == XDP_STATISTICS_OPTLEN {
            Ok(XdpStatistics(stats))
        } else {
            Err(io::Error::new(
                ErrorKind::Other,
                "`optlen` returned from `getsockopt` does not match passed buffer length",
            ))
        }
    }

    #[inline]
    pub fn rx_dropped(&self) -> u64 {
        self.0.rx_dropped
    }

    #[inline]
    pub fn rx_invalid_descs(&self) -> u64 {
        self.0.rx_invalid_descs
    }

    #[inline]
    pub fn tx_invalid_descs(&self) -> u64 {
        self.0.tx_invalid_descs
    }

    #[inline]
    pub fn rx_ring_full(&self) -> u64 {
        self.0.rx_ring_full
    }

    #[inline]
    pub fn rx_fill_ring_empty_descs(&self) -> u64 {
        self.0.rx_fill_ring_empty_descs
    }

    #[inline]
    pub fn tx_ring_empty_descs(&self) -> u64 {
        self.0.tx_ring_empty_descs
    }
}
