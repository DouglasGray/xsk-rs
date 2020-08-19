use libc::EINTR;
use std::io;

use crate::{socket::Fd, util};

pub fn poll_read(socket_fd: &mut Fd, timeout_ms: i32) -> io::Result<bool> {
    let pollin_fd = socket_fd.pollin_fd() as *mut _;

    let ret = unsafe { libc::poll(pollin_fd, 1, timeout_ms) };

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

pub fn poll_write(socket_fd: &mut Fd, timeout_ms: i32) -> io::Result<bool> {
    let pollout_fd = socket_fd.pollout_fd() as *mut _;

    let ret = unsafe { libc::poll(pollout_fd, 1, timeout_ms) };

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
