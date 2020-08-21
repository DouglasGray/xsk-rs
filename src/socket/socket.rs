use libbpf_sys::{xsk_ring_cons, xsk_ring_prod, xsk_socket, xsk_socket_config};
use libc::{EAGAIN, EBUSY, ENETDOWN, ENOBUFS, MSG_DONTWAIT};
use std::{
    convert::TryInto,
    ffi::{CString, NulError},
    io,
    marker::PhantomData,
    mem::MaybeUninit,
    ptr,
    sync::Arc,
};
use thiserror::Error;

use crate::{
    umem::{FrameDesc, Umem},
    util,
};

use super::{config::Config, fd::Fd, poll};

#[derive(Error, Debug)]
pub enum SocketCreateError {
    #[error("Interface name contains one or more null bytes")]
    InvalidIfName(#[from] NulError),
    #[error("OS or FFI call failed")]
    OsError {
        context: &'static str,
        io_err: io::Error,
    },
}

pub struct Socket<'umem> {
    inner: Box<xsk_socket>,
    _marker: PhantomData<&'umem ()>,
}

pub struct TxQueue<'umem> {
    inner: Box<xsk_ring_prod>,
    fd: Fd,
    _socket: Arc<Socket<'umem>>,
}

unsafe impl Send for TxQueue<'_> {}

pub struct RxQueue<'umem> {
    inner: Box<xsk_ring_cons>,
    fd: Fd,
    _socket: Arc<Socket<'umem>>,
}

unsafe impl Send for RxQueue<'_> {}

impl Socket<'_> {
    pub fn new<'a, 'umem>(
        config: Config,
        umem: &mut Umem<'umem>,
        if_name: &'a str,
        queue_id: u32,
    ) -> Result<(TxQueue<'umem>, RxQueue<'umem>), SocketCreateError> {
        let socket_create_config = xsk_socket_config {
            rx_size: config.rx_queue_size(),
            tx_size: config.tx_queue_size(),
            xdp_flags: config.xdp_flags().bits(),
            bind_flags: config.bind_flags().bits(),
            libbpf_flags: config.libbpf_flags().bits(),
        };

        let mut xsk_ptr: *mut xsk_socket = ptr::null_mut();
        let mut tx_q_ptr: MaybeUninit<xsk_ring_prod> = MaybeUninit::uninit();
        let mut rx_q_ptr: MaybeUninit<xsk_ring_cons> = MaybeUninit::uninit();

        let if_name = CString::new(if_name).map_err(|e| SocketCreateError::InvalidIfName(e))?;

        let err = unsafe {
            libbpf_sys::xsk_socket__create(
                &mut xsk_ptr,
                if_name.as_ptr(),
                queue_id,
                umem.as_mut_ptr(),
                rx_q_ptr.as_mut_ptr(),
                tx_q_ptr.as_mut_ptr(),
                &socket_create_config,
            )
        };

        if err != 0 {
            return Err(SocketCreateError::OsError {
                context: "Failed to create AF_XDP socket",
                io_err: io::Error::from_raw_os_error(err),
            });
        }

        let fd = unsafe { libbpf_sys::xsk_socket__fd(xsk_ptr) };

        if fd < 0 {
            unsafe {
                libbpf_sys::xsk_socket__delete(xsk_ptr);
            }

            return Err(SocketCreateError::OsError {
                context: "Could not retrieve AF_XDP socket file descriptor",
                io_err: io::Error::from_raw_os_error(err),
            });
        }

        let fd = Fd::new(fd);

        let socket = Arc::new(Socket {
            inner: unsafe { Box::from_raw(xsk_ptr) },
            _marker: PhantomData,
        });

        let tx_queue = TxQueue {
            inner: unsafe { Box::new(tx_q_ptr.assume_init()) },
            fd: fd.clone(),
            _socket: Arc::clone(&socket),
        };

        let rx_queue = RxQueue {
            inner: unsafe { Box::new(rx_q_ptr.assume_init()) },
            fd: fd,
            _socket: socket,
        };

        Ok((tx_queue, rx_queue))
    }
}

impl Drop for Socket<'_> {
    fn drop(&mut self) {
        unsafe {
            libbpf_sys::xsk_socket__delete(self.inner.as_mut());
        }
    }
}

impl RxQueue<'_> {
    /// Consume frames with received packets and add their info to [descs].
    /// Number of frames consumed will be less than or equal to the length of [descs].
    /// Returns the number of frames consumed (and therefore updated in [descs]).
    pub fn consume(&mut self, descs: &mut [FrameDesc]) -> usize {
        // usize -> u64 ok
        let nb: u64 = descs.len().try_into().unwrap();

        if nb == 0 {
            return 0;
        }

        let mut idx: u32 = 0;

        let cnt = unsafe { libbpf_sys::_xsk_ring_cons__peek(self.inner.as_mut(), nb, &mut idx) };

        if cnt > 0 {
            // Assuming 64-bit so u64 -> usize is ok
            for desc in descs.iter_mut().take(cnt.try_into().unwrap()) {
                unsafe {
                    let recv_pkt_desc =
                        libbpf_sys::_xsk_ring_cons__rx_desc(self.inner.as_mut(), idx);

                    desc.set_addr((*recv_pkt_desc).addr);
                    desc.set_len((*recv_pkt_desc).len);
                    desc.set_options((*recv_pkt_desc).options);
                }
                idx += 1;
            }

            unsafe { libbpf_sys::_xsk_ring_cons__release(self.inner.as_mut(), cnt) };
        }

        cnt.try_into().unwrap()
    }

    pub fn wakeup_and_consume(
        &mut self,
        descs: &mut [FrameDesc],
        poll_timeout: i32,
    ) -> io::Result<usize> {
        match poll::poll_read(&mut self.fd(), poll_timeout)? {
            true => Ok(self.consume(descs)),
            false => Ok(0),
        }
    }

    pub fn fd(&mut self) -> &mut Fd {
        &mut self.fd
    }
}

impl TxQueue<'_> {
    pub fn produce(&mut self, descs: &[FrameDesc]) -> usize {
        // Assuming 64-bit architecture so usize -> u64 / u32 -> u64 should be fine
        let nb: u64 = descs.len().try_into().unwrap();

        if nb == 0 {
            return 0;
        }

        let mut idx: u32 = 0;

        let cnt = unsafe { libbpf_sys::_xsk_ring_prod__reserve(self.inner.as_mut(), nb, &mut idx) };

        if cnt > 0 {
            for desc in descs.iter().take(cnt.try_into().unwrap()) {
                unsafe {
                    let send_pkt_desc =
                        libbpf_sys::_xsk_ring_prod__tx_desc(self.inner.as_mut(), idx);

                    (*send_pkt_desc).addr = desc.addr();
                    (*send_pkt_desc).len = desc.len();
                    (*send_pkt_desc).options = desc.options();
                }

                idx += 1;
            }

            unsafe { libbpf_sys::_xsk_ring_prod__submit(self.inner.as_mut(), cnt) };
        }

        cnt.try_into().unwrap()
    }

    pub fn produce_and_wakeup(&mut self, descs: &[FrameDesc]) -> io::Result<usize> {
        let cnt = self.produce(descs);

        if self.needs_wakeup() {
            self.wakeup()?;
        }

        Ok(cnt)
    }

    pub fn wakeup(&self) -> io::Result<()> {
        let ret =
            unsafe { libc::sendto(self.fd.id(), ptr::null(), 0, MSG_DONTWAIT, ptr::null(), 0) };

        if ret < 0 {
            match util::get_errno() {
                ENOBUFS | EAGAIN | EBUSY | ENETDOWN => (),
                _ => return Err(io::Error::last_os_error()),
            }
        }

        Ok(())
    }

    pub fn needs_wakeup(&self) -> bool {
        unsafe {
            if libbpf_sys::_xsk_ring_prod__needs_wakeup(self.inner.as_ref()) != 0 {
                true
            } else {
                false
            }
        }
    }

    pub fn fd(&mut self) -> &mut Fd {
        &mut self.fd
    }
}
