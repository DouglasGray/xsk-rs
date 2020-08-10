use libbpf_sys::{xsk_ring_cons, xsk_ring_prod, xsk_socket, xsk_socket_config};
use libc::{EAGAIN, EBUSY, ENETDOWN, ENOBUFS, MSG_DONTWAIT};
use std::{cmp, convert::TryInto, ffi::CString, io, mem::MaybeUninit, ptr};

use crate::{
    poll::{poll_read, Milliseconds},
    umem::{FrameDesc, Umem},
    util,
};

use super::config::Config;

pub struct Fd(i32);

impl Fd {
    pub(crate) fn descriptor(&self) -> i32 {
        self.0
    }
}

pub struct TxQueue {
    inner: Box<xsk_ring_prod>,
    socket_fd: Fd,
}

pub struct RxQueue {
    inner: Box<xsk_ring_cons>,
    socket_fd: Fd,
}

pub struct Socket {
    inner: Box<xsk_socket>,
    fd: Fd,
}

impl Socket {
    pub fn new(
        if_name: &str,
        queue_id: u32,
        config: Config,
        umem: &mut Umem,
    ) -> io::Result<(Socket, TxQueue, RxQueue)> {
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

        let if_name = CString::new(if_name)
            .expect("Failed to construct CString from provided interface name");

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
            eprintln!("Failed to create and bind socket");
            return Err(io::Error::from_raw_os_error(err));
        }

        let fd = unsafe { libbpf_sys::xsk_socket__fd(xsk_ptr) };

        if fd < 0 {
            eprintln!("Failed while retrieving socket file descriptor");

            unsafe {
                libbpf_sys::xsk_socket__delete(xsk_ptr);
            }

            return Err(io::Error::from_raw_os_error(fd));
        }

        let socket = Socket {
            inner: unsafe { Box::from_raw(xsk_ptr) },
            fd: Fd(fd),
        };

        let tx_queue = TxQueue {
            inner: unsafe { Box::new(tx_q_ptr.assume_init()) },
            socket_fd: Fd(fd),
        };

        let rx_queue = RxQueue {
            inner: unsafe { Box::new(rx_q_ptr.assume_init()) },
            socket_fd: Fd(fd),
        };

        Ok((socket, tx_queue, rx_queue))
    }

    pub fn file_descriptor(&self) -> &Fd {
        &self.fd
    }
}

impl Drop for Socket {
    fn drop(&mut self) {
        unsafe {
            libbpf_sys::xsk_socket__delete(self.inner.as_mut());
        }
    }
}

impl RxQueue {
    /// Consume frames with received packets and add their info to [descs].
    /// Number of frames consumed will be less than or equal to the length of [descs].
    /// Returns the number of frames consumed (and therefore updated in [descs]).
    pub fn consume(&mut self, descs: &mut [FrameDesc]) -> u64 {
        // usize -> u64 ok
        let nb: u64 = descs.len().try_into().unwrap();

        if nb == 0 {
            return 0;
        }

        let mut idx: u32 = 0;

        let cnt = unsafe { libbpf_sys::_xsk_ring_cons__peek(self.inner.as_mut(), nb, &mut idx) };

        // Assuming 64-bit so u64 -> usize is ok
        for desc in descs.iter_mut().take(cnt.try_into().unwrap()) {
            unsafe {
                let recv_pkt_desc = libbpf_sys::_xsk_ring_cons__rx_desc(self.inner.as_mut(), idx);

                desc.set_addr((*recv_pkt_desc).addr);
                desc.set_len((*recv_pkt_desc).len);
                desc.set_options((*recv_pkt_desc).options);
            }
            idx += 1;
        }

        if cnt > 0 {
            unsafe { libbpf_sys::_xsk_ring_cons__release(self.inner.as_mut(), cnt) };
        }

        cnt
    }

    pub fn poll_and_consume(
        &mut self,
        descs: &mut [FrameDesc],
        poll_timeout: &Milliseconds,
    ) -> io::Result<Option<u64>> {
        match poll_read(&self.socket_fd, poll_timeout)? {
            true => Ok(Some(self.consume(descs))),
            false => Ok(None),
        }
    }
}

impl TxQueue {
    pub fn produce(&mut self, descs: &[FrameDesc]) -> u64 {
        // First determine how many slots are free. Need to do this because if we try to reserve
        // more than is available in 'xsk_ring_prod__reserve' it will reserve nothing and return 0
        let nb: u64 = unsafe { libbpf_sys::_xsk_prod_nb_free(self.inner.as_mut(), 0) }
            .try_into()
            .unwrap();

        // Assuming 64-bit architecture so usize -> u64 / u32 -> u64 should be fine
        let nb = cmp::min(nb, descs.len().try_into().unwrap());

        if nb == 0 {
            return 0;
        }

        let mut idx: u32 = 0;

        let cnt = unsafe { libbpf_sys::_xsk_ring_prod__reserve(self.inner.as_mut(), nb, &mut idx) };

        for desc in descs.iter().take(cnt.try_into().unwrap()) {
            unsafe {
                let send_pkt_desc = libbpf_sys::_xsk_ring_prod__tx_desc(self.inner.as_mut(), idx);

                (*send_pkt_desc).addr = desc.addr();
                (*send_pkt_desc).len = desc.len();
                (*send_pkt_desc).options = desc.options();
            }

            idx += 1;
        }

        if cnt > 0 {
            unsafe { libbpf_sys::_xsk_ring_prod__submit(self.inner.as_mut(), cnt) };
        }

        cnt
    }

    pub fn produce_and_wakeup(&mut self, descs: &[FrameDesc]) -> io::Result<u64> {
        let cnt = self.produce(descs);

        if self.needs_wakeup() {
            let ret = unsafe {
                libc::sendto(
                    self.socket_fd.0,
                    ptr::null(),
                    0,
                    MSG_DONTWAIT,
                    ptr::null(),
                    0,
                )
            };

            if ret < 0 {
                match util::get_errno() {
                    ENOBUFS | EAGAIN | EBUSY | ENETDOWN => (),
                    _ => return Err(io::Error::last_os_error()),
                }
            }
        }

        Ok(cnt)
    }

    fn needs_wakeup(&self) -> bool {
        unsafe {
            if libbpf_sys::_xsk_ring_prod__needs_wakeup(self.inner.as_ref()) != 0 {
                true
            } else {
                false
            }
        }
    }
}
