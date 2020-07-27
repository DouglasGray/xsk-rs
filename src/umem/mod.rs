use libbpf_sys::{xsk_ring_cons, xsk_ring_prod, xsk_umem, xsk_umem_config};
use std::{cmp, collections::VecDeque, convert::TryInto, io, mem::MaybeUninit, ptr};

use crate::{
    poll::{poll_read, Milliseconds},
    socket::Fd,
};

mod mmap;

use mmap::MmapArea;

pub struct FrameDesc {
    pub addr: u64,
    pub len: u32,
    pub options: u32,
}

pub struct UmemBuilder {
    config: UmemConfig,
}

pub struct UmemBuilderWithMmap {
    config: UmemConfig,
    mmap_area: MmapArea,
}

pub struct Umem {
    inner: Box<xsk_umem>,
    mmap_area: MmapArea,
}

pub struct CompQueue {
    inner: Box<xsk_ring_cons>,
}

pub struct FillQueue {
    inner: Box<xsk_ring_prod>,
}

pub struct UmemConfig {
    frame_count: u32,
    frame_size: u32,
    fill_queue_size: u32,
    comp_queue_size: u32,
    frame_headroom: u32,
    use_huge_pages: bool,
}

impl UmemBuilder {
    pub fn create_mmap(self) -> io::Result<UmemBuilderWithMmap> {
        // Assuming a 64-bit architecture and as frame_count and frame_size are both u32,
        // casting to u64 (assuming their product doesn't overflow) should be fine
        let mmap_len: u64 = (self.config.frame_count)
            .checked_mul(self.config.frame_size)
            .expect(
                format!(
                "u32 overflow while calculating mmap len (frame_count * frame_size) = ({} * {})",
                &self.config.frame_count, &self.config.frame_size
            )
                .as_str(),
            )
            .try_into()
            .unwrap();

        let mmap_area = MmapArea::new(mmap_len, self.config.use_huge_pages)?;

        Ok(UmemBuilderWithMmap {
            config: self.config,
            mmap_area,
        })
    }
}

impl UmemBuilderWithMmap {
    pub fn create_umem(mut self) -> io::Result<(Umem, FillQueue, CompQueue)> {
        let fill_size = self.config.fill_queue_size.next_power_of_two();
        let comp_size = self.config.comp_queue_size.next_power_of_two();

        let umem_create_config = xsk_umem_config {
            fill_size,
            comp_size,
            frame_size: self.config.frame_size,
            frame_headroom: self.config.frame_headroom,
            flags: 0,
        };

        let mut umem_ptr: *mut xsk_umem = ptr::null_mut();
        let mut fq_ptr: MaybeUninit<xsk_ring_prod> = MaybeUninit::uninit();
        let mut cq_ptr: MaybeUninit<xsk_ring_cons> = MaybeUninit::uninit();

        let err = unsafe {
            libbpf_sys::xsk_umem__create(
                &mut umem_ptr,
                self.mmap_area.as_mut_ptr(),
                self.mmap_area.len,
                fq_ptr.as_mut_ptr(),
                cq_ptr.as_mut_ptr(),
                &umem_create_config,
            )
        };

        if err != 0 {
            return Err(io::Error::from_raw_os_error(err));
        }

        // Assuming 64-bit architecture so casting from u32 to u64 should be ok
        let umem_frame_descs: Vec<FrameDesc> =
            Vec::with_capacity(self.config.frame_count.try_into().unwrap());

        // Assuming 64-bit architecture so casting from u32 to u64 in 'addr' should be ok
        for i in 0..self.config.frame_count {
            let addr = (i * self.config.frame_size).try_into().unwrap();
            let len = self.config.frame_size;
            let options = 0;
            let frame_desc = FrameDesc { addr, len, options };
        }

        let umem = Umem {
            inner: unsafe { Box::from_raw(umem_ptr) },
            mmap_area: self.mmap_area,
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

impl Umem {
    pub fn new(config: UmemConfig) -> UmemBuilder {
        UmemBuilder { config }
    }

    pub(crate) fn as_mut_ptr(&mut self) -> *mut xsk_umem {
        self.inner.as_mut()
    }

    pub fn access_data_at_frame(&self, frame_desc: &FrameDesc) -> &[u8] {
        unsafe {
            let frame_ptr = self.mmap_area.as_ptr().offset(frame_desc.addr as isize);
            std::slice::from_raw_parts(frame_ptr as *const u8, frame_desc.len as usize)
        }
    }

    pub fn access_data_at_frame_mut(&mut self, frame_desc: &FrameDesc) -> &mut [u8] {
        unsafe {
            let frame_ptr = self.mmap_area.as_mut_ptr().offset(frame_desc.addr as isize);
            std::slice::from_raw_parts_mut(frame_ptr as *mut u8, frame_desc.len as usize)
        }
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

impl FillQueue {
    pub fn produce(&mut self, addrs: &mut VecDeque<u64>, nb: u64) -> u64 {
        let mut idx: u32 = 0;

        // Assuming 64-bit architecture so usize -> u64 should be fine
        let nb = cmp::min(nb, addrs.len().try_into().unwrap());

        let cnt = unsafe { libbpf_sys::_xsk_ring_prod__reserve(self.inner.as_mut(), nb, &mut idx) };

        for _ in 0..cnt {
            // Ensured above that cnt <= addrs.len()
            let addr = addrs.pop_front().unwrap();

            unsafe {
                *libbpf_sys::_xsk_ring_prod__fill_addr(self.inner.as_mut(), idx) = addr;
            }
            idx += 1;
        }

        if cnt > 0 {
            unsafe { libbpf_sys::_xsk_ring_prod__submit(self.inner.as_mut(), cnt) };
        }

        cnt
    }

    pub fn produce_and_wakeup(
        &mut self,
        addrs: &mut VecDeque<u64>,
        nb: u64,
        socket_fd: &Fd,
        poll_timeout: &Milliseconds,
    ) -> io::Result<u64> {
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
    pub fn consume(&mut self, addrs: &mut VecDeque<u64>, nb: u64) -> u64 {
        let mut idx: u32 = 0;

        // Assuming 64-bit architecture so usize -> u64 should be fine
        let nb = cmp::min(nb, addrs.len().try_into().unwrap());

        let cnt = unsafe { libbpf_sys::_xsk_ring_cons__peek(self.inner.as_mut(), nb, &mut idx) };

        for _ in 0..cnt {
            let addr = unsafe { *libbpf_sys::_xsk_ring_cons__comp_addr(self.inner.as_mut(), idx) };

            addrs.push_back(addr);

            idx += 1;
        }

        if cnt > 0 {
            unsafe { libbpf_sys::_xsk_ring_cons__release(self.inner.as_mut(), cnt) };
        }

        cnt
    }
}
