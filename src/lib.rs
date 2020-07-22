#![allow(dead_code)]

use libbpf_sys::{
    xsk_ring_cons, xsk_ring_prod, xsk_socket, xsk_socket_config, xsk_umem, xsk_umem_config,
    XDP_FLAGS_UPDATE_IF_NOEXIST, XDP_USE_NEED_WAKEUP,
};
use libc::{
    EAGAIN, EBUSY, EINTR, ENETDOWN, ENOBUFS, MAP_ANONYMOUS, MAP_FAILED, MAP_HUGETLB, MAP_PRIVATE,
    MSG_DONTWAIT, POLLIN, POLLOUT, PROT_READ, PROT_WRITE,
};
use std::{cmp, collections::VecDeque, ffi::CString, io, mem, mem::MaybeUninit, ptr};

mod socket;
mod umem;

pub struct Milliseconds {
    count: i32,
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

pub struct MmapArea {
    pub frame_count: usize,
    pub frame_size: usize,
    mem_ptr: *mut libc::c_void,
}

pub struct MmapAreaConfig {
    pub frame_count: usize,
    pub frame_size: usize,
    pub use_huge_pages: bool,
}

pub struct CompQueue {
    inner: Box<xsk_ring_cons>,
}

pub struct FillQueue {
    inner: Box<xsk_ring_prod>,
}

pub struct Umem {
    inner: Box<xsk_umem>,
    mmap_area: MmapArea,
}

pub struct UmemConfig {
    pub fill_queue_size: u32,
    pub comp_queue_size: u32,
    pub frame_headroom: u32,
}

pub struct TxQueue {
    inner: Box<xsk_ring_prod>,
}

pub struct RxQueue {
    inner: Box<xsk_ring_cons>,
}

pub struct Fd(i32);

pub struct Socket {
    inner: Box<xsk_socket>,
    fd: Fd,
}

pub struct SocketConfig {
    rx_queue_size: u32,
    tx_queue_size: u32,
}

pub struct FrameDesc {
    pub addr: u64,
    pub len: u32,
    pub options: u32,
}

fn get_errno() -> i32 {
    unsafe { *libc::__errno_location() }
}

fn poll_read(socket_fd: &Fd, timeout: &Milliseconds) -> io::Result<Option<()>> {
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
            return Ok(None);
        }
    }

    if ret == 0 {
        Ok(None)
    } else {
        Ok(Some(()))
    }
}

fn poll_write(socket_fd: &Fd, timeout: &Milliseconds) -> io::Result<Option<()>> {
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
            return Ok(None);
        }
    }

    if ret == 0 {
        Ok(None)
    } else {
        Ok(Some(()))
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

impl FillQueue {
    pub fn produce(&mut self, addrs: &mut VecDeque<u64>, nb: usize) -> usize {
        let mut idx: u32 = 0;
        let nb = cmp::min(addrs.len(), nb) as u64;

        let cnt = unsafe {
            libbpf_sys::_xsk_ring_prod__reserve(self.inner.as_mut(), nb, &mut idx) as usize
        };

        for _ in 0..cnt {
            // Ensured above that cnt <= addrs.len()
            let addr = addrs.pop_front().unwrap();

            unsafe {
                *libbpf_sys::_xsk_ring_prod__fill_addr(self.inner.as_mut(), idx) = addr;
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
        addrs: &mut VecDeque<u64>,
        nb: usize,
        socket_fd: &Fd,
        poll_timeout: &Milliseconds,
    ) -> io::Result<usize> {
        let cnt = self.produce(addrs, nb);

        if cnt > 0 && self.needs_wakeup() {
            poll_read(socket_fd, poll_timeout)?;
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

impl CompQueue {
    pub fn consume(&mut self, addrs: &mut VecDeque<u64>, nb: usize) -> usize {
        let mut idx: u32 = 0;
        let nb = cmp::min(addrs.len(), nb) as u64;

        let cnt =
            unsafe { libbpf_sys::_xsk_ring_cons__peek(self.inner.as_mut(), nb, &mut idx) as usize };

        for _ in 0..cnt {
            let addr = unsafe { *libbpf_sys::_xsk_ring_cons__comp_addr(self.inner.as_mut(), idx) };

            addrs.push_back(addr);

            idx += 1;
        }

        if cnt > 0 {
            unsafe { libbpf_sys::_xsk_ring_cons__release(self.inner.as_mut(), cnt as u64) };
        }

        cnt
    }
}

impl Socket {
    pub fn new(
        if_name: &str,
        queue_id: u32,
        umem: &mut Umem,
        config: &SocketConfig,
    ) -> io::Result<(Socket, TxQueue, RxQueue)> {
        let config = xsk_socket_config {
            rx_size: config.rx_queue_size,
            tx_size: config.tx_queue_size,
            xdp_flags: XDP_FLAGS_UPDATE_IF_NOEXIST,
            bind_flags: XDP_USE_NEED_WAKEUP as u16,
            libbpf_flags: 0,
        };

        let if_name = CString::new(if_name)
            .expect("Failed constructing CString from provided interface name");

        let mut xsk_ptr: *mut xsk_socket = ptr::null_mut();
        let mut tx_q_ptr: MaybeUninit<xsk_ring_prod> = MaybeUninit::uninit();
        let mut rx_q_ptr: MaybeUninit<xsk_ring_cons> = MaybeUninit::uninit();

        let err = unsafe {
            libbpf_sys::xsk_socket__create(
                &mut xsk_ptr,
                if_name.as_ptr(),
                queue_id,
                umem.inner.as_mut(),
                rx_q_ptr.as_mut_ptr(),
                tx_q_ptr.as_mut_ptr(),
                &config,
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

impl Umem {
    pub fn new(
        config: &UmemConfig,
        mmap_area: MmapArea,
    ) -> io::Result<(Umem, FillQueue, CompQueue)> {
        let fill_size = config.fill_queue_size.next_power_of_two();
        let comp_size = config.comp_queue_size.next_power_of_two();

        let config = xsk_umem_config {
            fill_size,
            comp_size,
            frame_size: mmap_area.frame_size as u32,
            frame_headroom: config.frame_headroom,
            flags: 0,
        };

        let mut umem_ptr: *mut xsk_umem = ptr::null_mut();
        let mut fq_ptr: MaybeUninit<xsk_ring_prod> = MaybeUninit::uninit();
        let mut cq_ptr: MaybeUninit<xsk_ring_cons> = MaybeUninit::uninit();

        let size = mmap_area.len() as u64;

        let err = unsafe {
            libbpf_sys::xsk_umem__create(
                &mut umem_ptr,
                mmap_area.mem_ptr,
                size,
                fq_ptr.as_mut_ptr(),
                cq_ptr.as_mut_ptr(),
                &config,
            )
        };

        if err != 0 {
            return Err(io::Error::from_raw_os_error(err));
        }

        let umem = Umem {
            inner: unsafe { Box::from_raw(umem_ptr) },
            mmap_area,
        };

        let fill_queue = FillQueue {
            inner: unsafe { Box::new(fq_ptr.assume_init()) },
        };

        let comp_queue = CompQueue {
            inner: unsafe { Box::new(cq_ptr.assume_init()) },
        };

        Ok((umem, fill_queue, comp_queue))
    }
}

impl Drop for Umem {
    fn drop(&mut self) {
        let err = unsafe { libbpf_sys::xsk_umem__delete(self.inner.as_mut()) };

        if err != 0 {
            eprintln!("xsk_umem__delete failed: {}", err);
        }
    }
}

impl MmapArea {
    pub fn new(config: &MmapAreaConfig) -> io::Result<Self> {
        let frame_count = config.frame_count.next_power_of_two();
        let frame_size = config.frame_size.next_power_of_two();

        let addr = ptr::null_mut();
        let prot = PROT_READ | PROT_WRITE;
        let file = -1;
        let offset = 0;

        let mut flags = MAP_ANONYMOUS | MAP_PRIVATE;
        if config.use_huge_pages {
            flags |= MAP_HUGETLB;
        }

        let ptr_inc_size = mem::size_of::<libc::c_void>();

        assert!(frame_size % ptr_inc_size == 0);

        let mem_ptr = unsafe {
            libc::mmap(
                addr,
                frame_count * frame_size,
                prot,
                flags,
                file,
                offset as libc::off_t,
            )
        };

        if mem_ptr == MAP_FAILED {
            Err(io::Error::last_os_error())
        } else {
            Ok(MmapArea {
                frame_count,
                frame_size,
                mem_ptr,
            })
        }
    }

    pub fn len(&self) -> usize {
        self.frame_count * self.frame_size
    }
}

impl Drop for MmapArea {
    fn drop(&mut self) {
        let err = unsafe { libc::munmap(self.mem_ptr, self.len()) };

        if err != 0 {
            eprintln!("munmap failed: {}", err);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn check_new_mmap_works() {
        let config = MmapAreaConfig {
            frame_count: 256,
            frame_size: 2048,
            use_huge_pages: false,
        };

        MmapArea::new(&config).expect("Creating memory mapped area failed");
    }

    #[test]
    fn check_new_umem_works() {
        let mmap_config = MmapAreaConfig {
            frame_count: 256,
            frame_size: 2048,
            use_huge_pages: false,
        };

        let mmap_area = MmapArea::new(&mmap_config).expect("Creating memory mapped area failed");

        let umem_config = UmemConfig {
            fill_queue_size: 2048,
            comp_queue_size: 2048,
            frame_headroom: 0,
        };

        Umem::new(&umem_config, mmap_area).expect("Initialisation of UMEM failed");
    }

    // #[test]
    // fn check_new_xsk_socket_works() {
    //     let mmap_config = MmapAreaConfig {
    //         frame_count: 256,
    //         frame_size: 2048,
    //         use_huge_pages: false,
    //     };

    //     let mmap_area = MmapArea::new(mmap_config).expect("Creating mmap-ed area failed");

    //     let umem_config = UmemConfig {
    //         fill_queue_size: 2048,
    //         comp_queue_size: 2048,
    //         frame_headroom: 0,
    //     };

    //     let (mut umem, _fq, _cq) =
    //         Umem::new(umem_config, &mmap_area).expect("Initialisation of UMEM failed");

    //     let socket_config = SocketConfig {
    //         rx_queue_size: 2048,
    //         tx_queue_size: 2048,
    //     };

    //     Socket::new("lo", 0, &mut umem, socket_config).expect("Failed to create and bind socket");
    // }

    #[test]
    fn check_needs_wakeup_flag_affects_tx_q_and_fill_q() {
        let mmap_config = MmapAreaConfig {
            frame_count: 256,
            frame_size: 2048,
            use_huge_pages: false,
        };

        let mmap_area = MmapArea::new(&mmap_config).expect("Creating memory mapped area failed");

        let umem_config = UmemConfig {
            fill_queue_size: 2048,
            comp_queue_size: 2048,
            frame_headroom: 0,
        };

        let (mut umem, fq, _cq) =
            Umem::new(&umem_config, mmap_area).expect("Initialisation of UMEM failed");

        let socket_config = SocketConfig {
            rx_queue_size: 2048,
            tx_queue_size: 2048,
        };

        let (_socket, tx_q, _rx_q) = Socket::new("lo", 0, &mut umem, &socket_config)
            .expect("Failed to create and bind socket");

        assert!(fq.needs_wakeup());
        assert!(tx_q.needs_wakeup());
    }
}
