//! Types for creating and using an AF_XDP [`Socket`].

pub mod fd;
use fd::Fd;

mod rx_queue;
pub use rx_queue::RxQueue;

mod tx_queue;
pub use tx_queue::TxQueue;

use libbpf_sys::{xsk_ring_cons, xsk_ring_prod, xsk_socket};
use std::{
    borrow::Borrow,
    error::Error,
    fmt, io,
    ptr::{self, NonNull},
    sync::Arc,
};

use crate::{
    config::{Interface, SocketConfig},
    umem::UmemInner,
};

use crate::umem::{CompQueue, FillQueue, Umem};

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
    pub fn new(
        config: SocketConfig,
        umem: &Umem,
        if_name: &Interface,
        queue_id: u32,
    ) -> Result<(TxQueue, RxQueue, Option<(FillQueue, CompQueue)>), SocketCreateError> {
        let mut xsk_ptr = ptr::null_mut();
        let mut tx_q = xsk_ring_prod::default();
        let mut rx_q = xsk_ring_cons::default();

        let (err, fq, cq) = unsafe {
            umem.with_parts(|xsk_umem, saved_fq_and_cq| {
                let (mut fq, mut cq) = saved_fq_and_cq
                    .unwrap_or_else(|| (xsk_ring_prod::default(), xsk_ring_cons::default()));

                let err = libbpf_sys::xsk_socket__create_shared(
                    &mut xsk_ptr,
                    if_name.as_cstr().as_ptr(),
                    queue_id,
                    xsk_umem,
                    &mut rx_q,
                    &mut tx_q,
                    &mut fq,
                    &mut cq,
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

        let tx_q = if tx_q.ring.is_null() {
            return Err(SocketCreateError {
                reason: "returned tx queue ring is null",
                err: io::Error::from_raw_os_error(err),
            });
        } else {
            TxQueue::new(tx_q, Arc::clone(&socket))
        };

        let rx_q = if rx_q.ring.is_null() {
            return Err(SocketCreateError {
                reason: "returned rx queue ring is null",
                err: io::Error::from_raw_os_error(err),
            });
        } else {
            RxQueue::new(rx_q, Arc::clone(&socket))
        };

        let fq_and_cq = match (fq.ring.is_null(), cq.ring.is_null()) {
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
