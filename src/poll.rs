use libc::{EINTR, POLLIN, POLLOUT};
use std::io;

use crate::{get_errno, socket::Fd};

pub struct Milliseconds {
    pub(crate) count: i32,
}

impl Milliseconds {
    pub fn new(count: i32) -> Result<Self, &'static str> {
        if count < 0 {
            Err("Number of milliseconds must not be less than zero")
        } else {
            Ok(Milliseconds { count })
        }
    }
}

pub(crate) fn poll_read(socket_fd: &Fd, timeout: &Milliseconds) -> io::Result<bool> {
    let mut pollfd = libc::pollfd {
        fd: socket_fd.0,
        events: POLLIN,
        revents: 0,
    };

    let ret = unsafe { libc::poll(&mut pollfd, 1, timeout.count) };

    if ret < 0 {
        if get_errno() != EINTR {
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

pub(crate) fn poll_write(socket_fd: &Fd, timeout: &Milliseconds) -> io::Result<bool> {
    let mut pollfd = libc::pollfd {
        fd: socket_fd.0,
        events: POLLOUT,
        revents: 0,
    };

    let ret = unsafe { libc::poll(&mut pollfd, 1, timeout.count) };

    if ret < 0 {
        if get_errno() != EINTR {
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
