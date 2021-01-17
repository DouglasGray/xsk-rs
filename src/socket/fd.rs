use libc::{POLLIN, POLLOUT};

/// Wrapper around libc's `pollfd` struct.
#[derive(Clone)]
pub struct PollFd {
    pollfd: libc::pollfd,
}

impl PollFd {
    fn new(pollfd: libc::pollfd) -> Self {
        PollFd { pollfd }
    }

    #[inline]
    pub(crate) fn pollfd(&mut self) -> &mut libc::pollfd {
        &mut self.pollfd
    }
}

/// Wrapper struct around some useful helper data for managing the
/// socket.
#[derive(Clone)]
pub struct Fd {
    id: i32,
    pollin_fd: PollFd,
    pollout_fd: PollFd,
}

impl Fd {
    pub(crate) fn new(id: i32) -> Self {
        let pollin_fd = PollFd::new(libc::pollfd {
            fd: id,
            events: POLLIN,
            revents: 0,
        });

        let pollout_fd = PollFd::new(libc::pollfd {
            fd: id,
            events: POLLOUT,
            revents: 0,
        });

        Fd {
            id,
            pollin_fd,
            pollout_fd,
        }
    }

    #[inline]
    pub(crate) fn id(&self) -> i32 {
        self.id
    }

    #[inline]
    pub(crate) fn pollin_fd(&mut self) -> &mut PollFd {
        &mut self.pollin_fd
    }

    #[inline]
    pub(crate) fn pollout_fd(&mut self) -> &mut PollFd {
        &mut self.pollout_fd
    }
}
