use libc::EINTR;
use std::io;

use super::{Fd, PollFd};
use crate::util;

#[inline]
fn poll(fd: &mut PollFd, timeout_ms: i32) -> io::Result<bool> {
    let ret = unsafe { libc::poll(fd.pollfd(), 1, timeout_ms) };

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

/// Check if anything is available to read on the socket.
#[inline]
pub fn poll_read(fd: &mut Fd, timeout_ms: i32) -> io::Result<bool> {
    poll(fd.pollin_fd(), timeout_ms)
}

/// Check if the socket is available to write.
#[inline]
pub fn poll_write(fd: &mut Fd, timeout_ms: i32) -> io::Result<bool> {
    poll(fd.pollout_fd(), timeout_ms)
}
