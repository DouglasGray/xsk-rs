//! File descriptor utilities.

use libc::{EINTR, POLLIN, POLLOUT};
use std::{
    io,
    os::unix::prelude::{AsRawFd, RawFd},
};

use crate::util;

#[derive(Debug, Clone, Copy)]
pub enum PollEvent {
    Read,
    Write,
}

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

/// A pollable socket file descriptor.
#[derive(Clone, Copy)]
pub struct Fd {
    id: i32,
    pollfd_read: PollFd,
    pollfd_write: PollFd,
}

impl Fd {
    pub(super) fn new(id: i32) -> Self {
        let pollin_fd = PollFd(libc::pollfd {
            fd: id,
            events: POLLIN,
            revents: 0,
        });

        let pollout_fd = PollFd(libc::pollfd {
            fd: id,
            events: POLLOUT,
            revents: 0,
        });

        Fd {
            id,
            pollfd_read: pollin_fd,
            pollfd_write: pollout_fd,
        }
    }

    #[inline]
    pub fn poll(&mut self, event: PollEvent, timeout_ms: i32) -> io::Result<bool> {
        match event {
            PollEvent::Read => self.pollfd_read.poll(timeout_ms),
            PollEvent::Write => self.pollfd_write.poll(timeout_ms),
        }
    }
}

impl AsRawFd for Fd {
    /// The inner file descriptor.
    ///
    /// May be required, for example, in the case where the default
    /// libbpf program has not been loaded (using the
    /// `XSK_LIBBPF_FLAGS__INHIBIT_PROG_LOAD` flag) and the socket's
    /// file descriptor must be available to register it in the
    /// `XSKMAP`.
    #[inline]
    fn as_raw_fd(&self) -> RawFd {
        self.id
    }
}
