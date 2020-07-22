use libbpf_sys::{
    xsk_ring_cons, xsk_ring_prod, xsk_socket, xsk_socket_config, XDP_FLAGS_UPDATE_IF_NOEXIST,
    XDP_USE_NEED_WAKEUP,
};
use libc::{EAGAIN, EBUSY, ENETDOWN, ENOBUFS, MSG_DONTWAIT};
use std::{cmp, collections::VecDeque, ffi::CString, io, mem::MaybeUninit, ptr};

use crate::{poll_read, umem::Umem, Fd, Milliseconds};

pub struct FrameDesc {
    pub addr: u64,
    pub len: u32,
    pub options: u32,
}

pub struct TxQueue {
    inner: Box<xsk_ring_prod>,
}

pub struct RxQueue {
    inner: Box<xsk_ring_cons>,
}

pub struct Socket {
    inner: Box<xsk_socket>,
    fd: Fd,
}

pub struct SocketConfig {
    pub if_name: String,
    pub queue_id: u32,
    pub rx_queue_size: u32,
    pub tx_queue_size: u32,
}

fn get_errno() -> i32 {
    unsafe { *libc::__errno_location() }
}

impl Socket {
    pub fn new(config: SocketConfig, umem: &mut Umem) -> io::Result<(Socket, TxQueue, RxQueue)> {
        let xsk_config = xsk_socket_config {
            rx_size: config.rx_queue_size,
            tx_size: config.tx_queue_size,
            xdp_flags: XDP_FLAGS_UPDATE_IF_NOEXIST,
            bind_flags: XDP_USE_NEED_WAKEUP as u16,
            libbpf_flags: 0,
        };

        let if_name = CString::new(config.if_name)
            .expect("Failed constructing CString from provided interface name");

        let mut xsk_ptr: *mut xsk_socket = ptr::null_mut();
        let mut tx_q_ptr: MaybeUninit<xsk_ring_prod> = MaybeUninit::uninit();
        let mut rx_q_ptr: MaybeUninit<xsk_ring_cons> = MaybeUninit::uninit();

        let err = unsafe {
            libbpf_sys::xsk_socket__create(
                &mut xsk_ptr,
                if_name.as_ptr(),
                config.queue_id,
                umem.as_mut_ptr(),
                rx_q_ptr.as_mut_ptr(),
                tx_q_ptr.as_mut_ptr(),
                &xsk_config,
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
        };

        let rx_queue = RxQueue {
            inner: unsafe { Box::new(rx_q_ptr.assume_init()) },
        };

        Ok((socket, tx_queue, rx_queue))
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
    pub fn consume(&mut self, descs: &mut VecDeque<FrameDesc>, nb: usize) -> usize {
        let mut idx: u32 = 0;

        let nb = cmp::min(descs.len(), nb) as u64;

        let cnt =
            unsafe { libbpf_sys::_xsk_ring_cons__peek(self.inner.as_mut(), nb, &mut idx) as usize };

        for _ in 0..cnt {
            unsafe {
                let recv_pkt_desc = libbpf_sys::_xsk_ring_cons__rx_desc(self.inner.as_mut(), idx);

                let addr = (*recv_pkt_desc).addr;
                let len = (*recv_pkt_desc).len;
                let options = (*recv_pkt_desc).options;

                descs.push_back(FrameDesc { addr, len, options })
            }
            idx += 1;
        }

        if cnt > 0 {
            unsafe { libbpf_sys::_xsk_ring_cons__release(self.inner.as_mut(), cnt as u64) };
        }

        cnt
    }

    pub fn poll_and_consume(
        &mut self,
        descs: &mut VecDeque<FrameDesc>,
        nb: usize,
        socket_fd: &Fd,
        poll_timeout: &Milliseconds,
    ) -> io::Result<Option<usize>> {
        match poll_read(socket_fd, poll_timeout)? {
            Some(()) => Ok(Some(self.consume(descs, nb))),
            None => Ok(None),
        }
    }
}

impl TxQueue {
    pub fn produce(&mut self, descs: &mut VecDeque<FrameDesc>, nb: usize) -> usize {
        let mut idx: u32 = 0;
        let nb = cmp::min(descs.len(), nb);

        let cnt = unsafe {
            libbpf_sys::_xsk_ring_prod__reserve(self.inner.as_mut(), nb as u64, &mut idx) as usize
        };

        for _ in 0..cnt {
            // Ensured above that cnt <= descs.len()
            let desc = descs.pop_front().unwrap();

            unsafe {
                let send_pkt_desc = libbpf_sys::_xsk_ring_prod__tx_desc(self.inner.as_mut(), idx);

                (*send_pkt_desc).addr = desc.addr;
                (*send_pkt_desc).len = desc.len;
                (*send_pkt_desc).options = desc.options;
            }

            idx += 1;
        }

        if cnt > 0 {
            unsafe { libbpf_sys::_xsk_ring_prod__submit(self.inner.as_mut(), cnt as u64) };
        }

        cnt
    }

    pub fn produce_and_wakeup(
        &mut self,
        descs: &mut VecDeque<FrameDesc>,
        nb: usize,
        socket_fd: &Fd,
    ) -> io::Result<usize> {
        let cnt = self.produce(descs, nb);

        if self.needs_wakeup() {
            let ret =
                unsafe { libc::sendto(socket_fd.0, ptr::null(), 0, MSG_DONTWAIT, ptr::null(), 0) };

            if ret < 0 {
                match get_errno() {
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
