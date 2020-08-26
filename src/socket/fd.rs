use libc::{POLLIN, POLLOUT};

/// Wrapper around libc's `pollfd` struct. Required when polling.
#[derive(Clone)]
pub struct PollFd {
    pollfd: libc::pollfd,
}

impl PollFd {
    fn new(pollfd: libc::pollfd) -> Self {
        PollFd { pollfd }
    }

    pub(crate) fn pollfd(&mut self) -> &mut libc::pollfd {
        &mut self.pollfd
    }
}

/// Wrapper struct around some useful helper data for managing the socket.
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

    pub(crate) fn id(&self) -> i32 {
        self.id
    }

    pub(crate) fn pollin_fd(&mut self) -> &mut PollFd {
        &mut self.pollin_fd
    }

    pub(crate) fn pollout_fd(&mut self) -> &mut PollFd {
        &mut self.pollout_fd
    }
}
