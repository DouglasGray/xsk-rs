use libc::{EINTR, POLLIN, POLLOUT};
use std::{convert::TryInto, io};

use crate::{socket::Fd, util};

pub struct Milliseconds {
    count: i32,
}

impl Milliseconds {
    pub fn new(count: u16) -> Self {
        Milliseconds {
            count: count.try_into().unwrap(),
        }
    }

    pub fn count(&self) -> i32 {
        self.count
    }
}

pub(crate) fn poll_read(socket_fd: &Fd, timeout: &Milliseconds) -> io::Result<bool> {
    let mut pollfd = libc::pollfd {
        fd: socket_fd.id(),
        events: POLLIN,
        revents: 0,
    };

    let ret = unsafe { libc::poll(&mut pollfd, 1, timeout.count()) };

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

pub(crate) fn poll_write(socket_fd: &Fd, timeout: &Milliseconds) -> io::Result<bool> {
    let mut pollfd = libc::pollfd {
        fd: socket_fd.id(),
        events: POLLOUT,
        revents: 0,
    };

    let ret = unsafe { libc::poll(&mut pollfd, 1, timeout.count()) };

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_u16_max_works_when_creating_milliseconds() {
        Milliseconds::new(u16::MAX);
    }
}
