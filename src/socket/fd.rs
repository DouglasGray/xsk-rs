//! File descriptor utilities.

use libc::{EINTR, POLLIN, POLLOUT, SOL_XDP};
use libxdp_sys::{xdp_statistics, XDP_STATISTICS};
use std::{
    fmt,
    io::{self, ErrorKind},
    mem,
    os::unix::prelude::{AsRawFd, RawFd},
};

use crate::util;

const XDP_STATISTICS_SIZEOF: u32 = mem::size_of::<xdp_statistics>() as u32;

#[derive(Clone, Copy)]
struct PollFd(libc::pollfd);

impl PollFd {
    #[inline]
    fn poll(&mut self, timeout_ms: i32) -> io::Result<bool> {
        let ret = unsafe { libc::poll(&mut self.0, 1, timeout_ms) };

        if ret < 0 {
            if util::get_errno() != EINTR {
                return Err(io::Error::last_os_error());
            } else {
                return Ok(false);
            }
        }

        if ret == 0 {
            Ok(false)
        } else {
            Ok(true)
        }
    }
}

/// A pollable AF_XDP [`Socket`](crate::Socket) file descriptor.
pub struct Fd {
    id: i32,
    pollfd_read: PollFd,
    pollfd_write: PollFd,
}

impl Fd {
    pub(super) fn new(id: i32) -> Self {
        let pollfd_read = PollFd(libc::pollfd {
            fd: id,
            events: POLLIN,
            revents: 0,
        });

        let pollfd_write = PollFd(libc::pollfd {
            fd: id,
            events: POLLOUT,
            revents: 0,
        });

        Fd {
            id,
            pollfd_read,
            pollfd_write,
        }
    }

    pub(super) fn clone(&self) -> Self {
        Self {
            id: self.id,
            pollfd_read: self.pollfd_read,
            pollfd_write: self.pollfd_write,
        }
    }

    #[inline]
    pub(crate) fn poll_read(&mut self, timeout_ms: i32) -> io::Result<bool> {
        self.pollfd_read.poll(timeout_ms)
    }

    #[inline]
    pub(crate) fn poll_write(&mut self, timeout_ms: i32) -> io::Result<bool> {
        self.pollfd_write.poll(timeout_ms)
    }

    /// Returns [`Socket`](crate::Socket) statistics.
    #[inline]
    pub fn xdp_statistics(&self) -> io::Result<XdpStatistics> {
        let mut stats = XdpStatistics::default();

        let mut optlen = XDP_STATISTICS_SIZEOF;

        let err = unsafe {
            libc::getsockopt(
                self.as_raw_fd(),
                SOL_XDP,
                XDP_STATISTICS as i32,
                &mut stats.0 as *mut _ as *mut libc::c_void,
                &mut optlen,
            )
        };

        if err != 0 {
            return Err(io::Error::last_os_error());
        }

        if optlen == XDP_STATISTICS_SIZEOF {
            Ok(stats)
        } else {
            Err(io::Error::new(
                ErrorKind::Other,
                "`optlen` returned from `getsockopt` does not match `xdp_statistics` struct size",
            ))
        }
    }
}

impl fmt::Debug for Fd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Fd").field("id", &self.id).finish()
    }
}

impl AsRawFd for Fd {
    /// The inner file descriptor.
    ///
    /// May be required, for example, in the case where the default
    /// libbpf program has not been loaded (using the
    /// [`XSK_LIBXDP_FLAGS_INHIBIT_PROG_LOAD`] flag) and the socket's
    /// file descriptor must be available to register it in the
    /// `XSKMAP`.
    ///
    /// [`XSK_LIBXDP_FLAGS_INHIBIT_PROG_LOAD`]: crate::config::LibxdpFlags::XSK_LIBXDP_FLAGS_INHIBIT_PROG_LOAD
    #[inline]
    fn as_raw_fd(&self) -> RawFd {
        self.id
    }
}

/// AF_XDP [`Socket`](crate::Socket) statistics.
///
/// Can be retrieved by calling [`xdp_statistics`](Fd::xdp_statistics).
#[derive(Debug, Clone, Copy)]
pub struct XdpStatistics(xdp_statistics);

impl Default for XdpStatistics {
    fn default() -> Self {
        Self(xdp_statistics {
            rx_dropped: 0,
            rx_invalid_descs: 0,
            tx_invalid_descs: 0,
            rx_ring_full: 0,
            rx_fill_ring_empty_descs: 0,
            tx_ring_empty_descs: 0,
        })
    }
}

impl XdpStatistics {
    /// Received packets dropped due to an invalid descriptor.
    #[inline]
    pub fn rx_invalid_descs(&self) -> u64 {
        self.0.rx_invalid_descs
    }

    /// Received packets dropped due to rx ring being full.
    #[inline]
    pub fn rx_ring_full(&self) -> u64 {
        self.0.rx_ring_full
    }

    /// Received packets dropped for other reasons.
    #[inline]
    pub fn rx_dropped(&self) -> u64 {
        self.0.rx_dropped
    }

    /// Packets to be sent but dropped due to an invalid desccriptor.
    #[inline]
    pub fn tx_invalid_descs(&self) -> u64 {
        self.0.tx_invalid_descs
    }

    /// Items failed to be retrieved from fill ring.
    #[inline]
    pub fn rx_fill_ring_empty_descs(&self) -> u64 {
        self.0.rx_fill_ring_empty_descs
    }

    /// Items failed to be retrieved from tx ring.
    #[inline]
    pub fn tx_ring_empty_descs(&self) -> u64 {
        self.0.tx_ring_empty_descs
    }
}
