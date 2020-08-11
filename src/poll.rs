use libc::{EINTR, POLLIN, POLLOUT};
use std::io;

use crate::{socket::Fd, util};

pub fn poll_read(socket_fd: &Fd, timeout_ms: i32) -> io::Result<bool> {
    let mut pollfd = libc::pollfd {
        fd: socket_fd.id(),
        events: POLLIN,
        revents: 0,
    };

    let ret = unsafe { libc::poll(&mut pollfd, 1, timeout_ms) };

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

pub fn poll_write(socket_fd: &Fd, timeout_ms: i32) -> io::Result<bool> {
    let mut pollfd = libc::pollfd {
        fd: socket_fd.id(),
        events: POLLOUT,
        revents: 0,
    };

    let ret = unsafe { libc::poll(&mut pollfd, 1, timeout_ms) };

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
