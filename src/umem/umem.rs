use libbpf_sys::{xsk_ring_cons, xsk_ring_prod, xsk_umem, xsk_umem_config};
use std::{convert::TryInto, io, marker::PhantomData, mem::MaybeUninit, ptr};
use thiserror::Error;

use crate::socket::{self, Fd};

use super::{config::Config, mmap::MmapArea};

#[derive(Debug, Clone, PartialEq)]
pub struct FrameDesc {
    addr: u64,
    len: u32,
    options: u32,
}

impl FrameDesc {
    pub fn addr(&self) -> u64 {
        self.addr
    }

    pub fn len(&self) -> u32 {
        self.len
    }

    pub fn options(&self) -> u32 {
        self.options
    }

    pub(crate) fn set_addr(&mut self, addr: u64) {
        self.addr = addr
    }

    pub fn set_len(&mut self, len: u32) {
        self.len = len
    }

    pub(crate) fn set_options(&mut self, options: u32) {
        self.options = options
    }
}

pub struct UmemBuilder {
    config: Config,
}

pub struct UmemBuilderWithMmap {
    config: Config,
    mmap_area: MmapArea,
}

pub struct Umem<'a> {
    inner: Box<xsk_umem>,
    mmap_area: MmapArea,
    frame_descs: Vec<FrameDesc>,
    frame_count: u32,
    frame_size: u32,
    max_addr: u64,
    frame_size_u64: u64,
    frame_size_usize: usize,
    _marker: PhantomData<&'a ()>,
}

#[derive(Error, Debug)]
pub enum UmemAccessError {
    #[error("Frame address {req_addr:? } is out of bounds, max address is {max_addr:?}")]
    AddrOutOfBounds { req_addr: u64, max_addr: u64 },
    #[error("Frame address {req_addr:?} must be a multiple of the frame size ({frame_size:?})")]
    AddrNotAligned { req_addr: u64, frame_size: u32 },
    #[error("Data ({data_len:?} bytes) cannot be larger than frame size ({frame_size:?} bytes)")]
    DataLenOutOfBounds { data_len: usize, frame_size: u32 },
}

pub struct CompQueue<'umem> {
    inner: Box<xsk_ring_cons>,
    _marker: PhantomData<&'umem ()>,
}

unsafe impl Send for CompQueue<'_> {}

pub struct FillQueue<'umem> {
    inner: Box<xsk_ring_prod>,
    _marker: PhantomData<&'umem ()>,
}

unsafe impl Send for FillQueue<'_> {}

impl UmemBuilder {
    pub fn create_mmap(self) -> io::Result<UmemBuilderWithMmap> {
        let mmap_area = MmapArea::new(self.config.umem_len(), self.config.use_huge_pages())?;

        Ok(UmemBuilderWithMmap {
            config: self.config,
            mmap_area,
        })
    }
}

impl<'a> UmemBuilderWithMmap {
    pub fn create_umem(mut self) -> io::Result<(Umem<'a>, FillQueue<'a>, CompQueue<'a>)> {
        let umem_create_config = xsk_umem_config {
            fill_size: self.config.fill_queue_size(),
            comp_size: self.config.comp_queue_size(),
            frame_size: self.config.frame_size(),
            frame_headroom: self.config.frame_headroom(),
            flags: self.config.umem_flags().bits(),
        };

        let mut umem_ptr: *mut xsk_umem = ptr::null_mut();
        let mut fq_ptr: MaybeUninit<xsk_ring_prod> = MaybeUninit::uninit();
        let mut cq_ptr: MaybeUninit<xsk_ring_cons> = MaybeUninit::uninit();

        let err = unsafe {
            libbpf_sys::xsk_umem__create(
                &mut umem_ptr,
                self.mmap_area.as_mut_ptr(),
                self.mmap_area.len(),
                fq_ptr.as_mut_ptr(),
                cq_ptr.as_mut_ptr(),
                &umem_create_config,
            )
        };

        if err != 0 {
            return Err(io::Error::from_raw_os_error(err));
        }

        // Ok: usize <= u64
        let mut frame_descs: Vec<FrameDesc> =
            Vec::with_capacity(self.config.frame_count().try_into().unwrap());

        // Ok: upcasting u32 -> u64
        let frame_size_u64: u64 = self.config.frame_size().try_into().unwrap();
        let frame_count_u64: u64 = self.config.frame_count().try_into().unwrap();

        for i in 0..frame_count_u64 {
            let addr = i * frame_size_u64;
            let len = 0;
            let options = 0;

            let frame_desc = FrameDesc { addr, len, options };

            frame_descs.push(frame_desc);
        }

        let umem = Umem {
            inner: unsafe { Box::from_raw(umem_ptr) },
            mmap_area: self.mmap_area,
            frame_descs,
            frame_count: self.config.frame_count(),
            frame_size: self.config.frame_size(),
            max_addr: (frame_count_u64 - 1) * frame_size_u64,
            frame_size_u64,
            frame_size_usize: self.config.frame_size().try_into().unwrap(),
            _marker: PhantomData,
        };

        let fill_queue = FillQueue {
            inner: unsafe { Box::new(fq_ptr.assume_init()) },
            _marker: PhantomData,
        };

        let comp_queue = CompQueue {
            inner: unsafe { Box::new(cq_ptr.assume_init()) },
            _marker: PhantomData,
        };

        Ok((umem, fill_queue, comp_queue))
    }
}

impl Umem<'_> {
    pub fn builder(config: Config) -> UmemBuilder {
        UmemBuilder { config }
    }

    pub fn empty_frame_descs(&self) -> &[FrameDesc] {
        &self.frame_descs[..]
    }

    pub fn frame_count(&self) -> u32 {
        self.frame_count
    }

    pub fn frame_size(&self) -> u32 {
        self.frame_size
    }

    pub(crate) fn as_mut_ptr(&mut self) -> *mut xsk_umem {
        self.inner.as_mut()
    }

    fn check_frame_addr_valid(&self, addr: &u64) -> Result<(), UmemAccessError> {
        // Check frame address is within bounds
        if *addr > self.max_addr {
            return Err(UmemAccessError::AddrOutOfBounds {
                max_addr: self.max_addr,
                req_addr: *addr,
            });
        }

        // Check frame address is aligned
        if *addr % self.frame_size_u64 != 0 {
            return Err(UmemAccessError::AddrNotAligned {
                req_addr: *addr,
                frame_size: self.frame_size,
            });
        }

        Ok(())
    }

    fn check_data_valid(&self, data: &[u8]) -> Result<(), UmemAccessError> {
        // Check that data fits within a frame
        if data.len() > self.frame_size.try_into().unwrap() {
            return Err(UmemAccessError::DataLenOutOfBounds {
                data_len: data.len(),
                frame_size: self.frame_size,
            });
        }

        Ok(())
    }

    pub fn frame_ref(&self, addr: &u64) -> Result<&[u8], UmemAccessError> {
        self.check_frame_addr_valid(&addr)?;

        let offset: usize = (*addr).try_into().unwrap();

        Ok(self
            .mmap_area
            .mem_range(offset, self.frame_size_usize)
            .unwrap())
    }

    pub fn frame_ref_mut(&mut self, addr: &u64) -> Result<&mut [u8], UmemAccessError> {
        self.check_frame_addr_valid(&addr)?;

        let offset: usize = (*addr).try_into().unwrap();

        Ok(self
            .mmap_area
            .mem_range_mut(offset, self.frame_size_usize)
            .unwrap())
    }

    pub fn copy_data_to_frame(
        &mut self,
        addr: &u64,
        data: &[u8],
    ) -> Result<usize, UmemAccessError> {
        if data.len() == 0 {
            return Ok(0);
        }

        self.check_data_valid(data)?;

        let frame_ref = self.frame_ref_mut(addr)?;

        frame_ref[..data.len()].copy_from_slice(data);

        Ok(data.len())
    }
}

impl Drop for Umem<'_> {
    fn drop(&mut self) {
        let err = unsafe { libbpf_sys::xsk_umem__delete(self.inner.as_mut()) };

        if err != 0 {
            log::error!("xsk_umem__delete failed: {}", err);
        }
    }
}

impl FillQueue<'_> {
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
                    *libbpf_sys::_xsk_ring_prod__fill_addr(self.inner.as_mut(), idx) = desc.addr;
                }
                idx += 1;
            }

            unsafe { libbpf_sys::_xsk_ring_prod__submit(self.inner.as_mut(), cnt) };
        }

        cnt.try_into().unwrap()
    }

    pub fn produce_and_wakeup(
        &mut self,
        descs: &[FrameDesc],
        socket_fd: &mut Fd,
        poll_timeout: i32,
    ) -> io::Result<usize> {
        let cnt = self.produce(descs);

        if cnt > 0 && self.needs_wakeup() {
            self.wakeup(socket_fd, poll_timeout)?;
        }

        Ok(cnt)
    }

    pub fn wakeup(&self, fd: &mut Fd, poll_timeout: i32) -> io::Result<()> {
        socket::poll_read(fd, poll_timeout)?;
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
}

impl CompQueue<'_> {
    pub fn consume(&mut self, descs: &mut [FrameDesc]) -> usize {
        let nb: u64 = descs.len().try_into().unwrap();

        if nb == 0 {
            return 0;
        }

        let mut idx: u32 = 0;

        let cnt = unsafe { libbpf_sys::_xsk_ring_cons__peek(self.inner.as_mut(), nb, &mut idx) };

        for desc in descs.iter_mut().take(cnt.try_into().unwrap()) {
            let addr = unsafe { *libbpf_sys::_xsk_ring_cons__comp_addr(self.inner.as_mut(), idx) };

            desc.set_addr(addr);
            desc.set_len(0);
            desc.set_options(0);

            idx += 1;
        }

        if cnt > 0 {
            unsafe { libbpf_sys::_xsk_ring_cons__release(self.inner.as_mut(), cnt) };
        }

        cnt.try_into().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use rand;
    use std::num::NonZeroU32;

    use super::*;
    use crate::umem::{Config, UmemFlags};

    const FRAME_COUNT: u32 = 8;
    const FRAME_SIZE: u32 = 2048;

    fn generate_random_bytes(len: u32) -> Vec<u8> {
        (0..len).map(|_| rand::random::<u8>()).collect()
    }

    fn umem_config() -> Config {
        Config::new(
            NonZeroU32::new(FRAME_COUNT).unwrap(),
            NonZeroU32::new(FRAME_SIZE).unwrap(),
            4,
            4,
            0,
            false,
            UmemFlags::empty(),
        )
        .unwrap()
    }

    fn umem<'a>() -> (Umem<'a>, FillQueue<'a>, CompQueue<'a>) {
        let config = umem_config();

        Umem::builder(config)
            .create_mmap()
            .expect("Failed to create mmap region")
            .create_umem()
            .expect("Failed to create UMEM")
    }

    #[test]
    fn frame_addr_checks_ok() {
        let (umem, _fq, _cq) = umem();

        // First frame / addr 0 is ok
        assert!(umem.check_frame_addr_valid(&0).is_ok());

        // Max possible address ok
        let max_addr = (FRAME_COUNT as u64 - 1) * (FRAME_SIZE as u64);

        assert!(umem.check_frame_addr_valid(&max_addr).is_ok());

        // Another frame ok
        let frame_addr = 2 * FRAME_SIZE as u64;
        assert!(umem.check_frame_addr_valid(&(frame_addr)).is_ok());

        // Max address + 1 fails
        assert!(umem.check_frame_addr_valid(&(max_addr + 1)).is_err());

        // Next valid address after maximum also fails
        let max_addr_next = (FRAME_COUNT as u64) * (FRAME_SIZE as u64);

        assert!(matches!(
            umem.check_frame_addr_valid(&max_addr_next),
            Err(UmemAccessError::AddrOutOfBounds { .. })
        ));

        // Misaligned address fails
        assert!(matches!(
            umem.check_frame_addr_valid(&1),
            Err(UmemAccessError::AddrNotAligned { .. })
        ));

        // Misaligned address fails
        assert!(matches!(
            umem.check_frame_addr_valid(&(frame_addr + 13)),
            Err(UmemAccessError::AddrNotAligned { .. })
        ));
    }

    #[test]
    fn data_checks_ok() {
        let (umem, _fq, _cq) = umem();

        // Empty data ok
        let empty_data: Vec<u8> = Vec::new();

        assert!(umem.check_data_valid(&empty_data).is_ok());

        // Data within frame size ok
        let data = generate_random_bytes(FRAME_SIZE - 1);

        assert!(umem.check_data_valid(&data).is_ok());

        // Data exactly frame size is ok
        let data = generate_random_bytes(FRAME_SIZE);

        assert!(umem.check_data_valid(&data).is_ok());

        // Data greater than frame size fails
        let data = generate_random_bytes(FRAME_SIZE + 1);

        assert!(matches!(
            umem.check_data_valid(&data),
            Err(UmemAccessError::DataLenOutOfBounds { .. })
        ));
    }

    #[test]
    fn write_to_umem_then_read_small_byte_array() {
        let (mut umem, _fq, _cq) = umem();

        let addr = 0;
        let data = [b'H', b'e', b'l', b'l', b'o'];

        umem.copy_data_to_frame(&addr, &data[..]).unwrap();

        let frame_ref = umem.frame_ref(&addr).unwrap();

        assert_eq!(data, frame_ref[..data.len()]);
    }

    #[test]
    fn write_max_bytes_to_neighbouring_umem_frames() {
        let (mut umem, _fq, _cq) = umem();

        // Create random data and write to adjacent frames
        let fst_addr = 0;
        let snd_addr = FRAME_SIZE as u64;

        let fst_data = generate_random_bytes(FRAME_SIZE);
        let snd_data = generate_random_bytes(FRAME_SIZE);

        umem.copy_data_to_frame(&fst_addr, &fst_data).unwrap();
        umem.copy_data_to_frame(&snd_addr, &snd_data).unwrap();

        let fst_frame_ref = umem.frame_ref(&fst_addr).unwrap();
        let snd_frame_ref = umem.frame_ref(&snd_addr).unwrap();

        // Check that they are indeed the same
        assert_eq!(fst_data[..], fst_frame_ref[..fst_data.len()]);
        assert_eq!(snd_data[..], snd_frame_ref[..snd_data.len()]);

        // Ensure there are no gaps and the frames lie snugly
        let mem_len = (FRAME_SIZE * 2) as usize;

        let mem_range = umem.mmap_area.mem_range(0, mem_len).unwrap();

        let mut data_vec = Vec::with_capacity(mem_len);

        data_vec.extend_from_slice(&fst_data);
        data_vec.extend_from_slice(&snd_data);

        assert_eq!(&data_vec[..], mem_range);
    }
}
