use libbpf_sys::{xsk_ring_cons, xsk_ring_prod, xsk_umem, xsk_umem_config};
use std::{convert::TryInto, error::Error, fmt, io, marker::PhantomData, mem::MaybeUninit, ptr};

use crate::socket::{self, Fd};

use super::{config::Config, mmap::MmapArea};

/// Describes a UMEM frame's location and current contents.
///
/// The `addr` field identifies the particular UMEM frame and
/// the `len` field describes the length (in bytes) of any data
/// stored at that frame. The address is the offset in bytes from
/// the start of the UMEM, and as each addresss references the
/// start of a frame it is therefore a multiple of the frame size.
///
/// If sending data, the `len` field will need to be set by the user
/// before transmitting via the [TxQueue](struct.TxQueue.html).
/// Otherwise when reading received frames using the [RxQueue](struct.RxQueue.html),
/// the `len` field will be set by the kernel and dictates the number
/// of bytes the user should read from the UMEM.
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

    /// Required when sending data using [TxQueue](struct.TxQueue.html).
    ///
    /// Once data has been written to a UMEM frame by the user at a given
    /// address, they must update the respective `FrameDesc`'s length before
    /// handing it over to the kernel to be transmitted to ensure the
    /// correct number of bytes are sent.
    pub fn set_len(&mut self, len: u32) {
        self.len = len
    }

    pub(crate) fn set_options(&mut self, options: u32) {
        self.options = options
    }
}

/// Initial step for building a UMEM. This creates the underlying `mmap` area.
pub struct UmemBuilder {
    config: Config,
}

/// Final step for building a UMEM, this makes the required calls to `libbpf`.
pub struct UmemBuilderWithMmap {
    config: Config,
    mmap_area: MmapArea,
}

/// A region of virtual contiguous memory divided into equal-sized frames.
/// It provides the underlying working memory for an AF_XDP socket.
pub struct Umem<'a> {
    inner: Box<xsk_umem>,
    mmap_area: MmapArea,
    frame_count: u32,
    frame_size: u32,
    max_addr: u64,
    frame_size_u64: u64,
    frame_size_usize: usize,
    _marker: PhantomData<&'a ()>,
}

#[derive(Debug)]
pub enum UmemAccessError {
    AddrOutOfBounds { req_addr: u64, max_addr: u64 },
    AddrNotAligned { req_addr: u64, frame_size: u32 },
    DataLenOutOfBounds { data_len: usize, frame_size: u32 },
}

impl fmt::Display for UmemAccessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use UmemAccessError::*;
        match self {
            AddrOutOfBounds { req_addr, max_addr } => write!(
                f,
                "frame address {} is out of bounds, max address is {}",
                req_addr, max_addr
            ),
            AddrNotAligned {
                req_addr,
                frame_size,
            } => write!(
                f,
                "frame address {} must be a multiple of the frame size ({})",
                req_addr, frame_size
            ),
            DataLenOutOfBounds {
                data_len,
                frame_size,
            } => write!(
                f,
                "data length ({} bytes) cannot be larger than the frame size ({} bytes)",
                data_len, frame_size
            ),
        }
    }
}

impl Error for UmemAccessError {}

/// Used to transfer ownership of UMEM frames from kernel-space to user-space.
///
/// Frames received in this queue are those that have been sent via the
/// [TxQueue](struct.TxQueue.html).
///
/// For more information see the [docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html#umem-completion-ring)
pub struct CompQueue<'umem> {
    inner: Box<xsk_ring_cons>,
    _marker: PhantomData<&'umem ()>,
}

unsafe impl Send for CompQueue<'_> {}

/// Used to transfer ownership of UMEM frames from user-space to kernel-space.
///
/// These frames will be used to receive packets, and so will be returned
/// via the [RxQueue](struct.RxQueue.html).
///
/// For more information see the [docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html#umem-fill-ring)
pub struct FillQueue<'umem> {
    inner: Box<xsk_ring_prod>,
    _marker: PhantomData<&'umem ()>,
}

unsafe impl Send for FillQueue<'_> {}

impl UmemBuilder {
    /// Allocate a memory region for the UMEM.
    ///
    /// Before we can create the UMEM we first need to allocate a chunk of memory,
    /// which will eventually be split up into frames. We do this with a call to `mmap`,
    /// requesting a read + write protected anonymous memory region.
    pub fn create_mmap(self) -> io::Result<UmemBuilderWithMmap> {
        let mmap_area = MmapArea::new(self.config.umem_len(), self.config.use_huge_pages())?;

        Ok(UmemBuilderWithMmap {
            config: self.config,
            mmap_area,
        })
    }
}

impl<'a> UmemBuilderWithMmap {
    /// Using the allocated memory region, create the UMEM.
    ///
    /// Once we've successfully requested a region of memory, create the UMEM with it by
    /// splitting the memory region into frames and creating the [FillQueue](struct.FillQueue.html)
    /// and [CompQueue](struct.CompQueue.html).
    pub fn create_umem(
        mut self,
    ) -> io::Result<(Umem<'a>, FillQueue<'a>, CompQueue<'a>, Vec<FrameDesc>)> {
        let umem_create_config = xsk_umem_config {
            fill_size: self.config.fill_queue_size(),
            comp_size: self.config.comp_queue_size(),
            frame_size: self.config.frame_size(),
            frame_headroom: self.config.frame_headroom(),
            flags: 0,
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

        Ok((umem, fill_queue, comp_queue, frame_descs))
    }
}

impl Umem<'_> {
    pub fn builder(config: Config) -> UmemBuilder {
        UmemBuilder { config }
    }

    /// Number of frames in the UMEM
    pub fn frame_count(&self) -> u32 {
        self.frame_count
    }

    /// The length of each frame in bytes
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

    /// Return a reference to the frame at a given address.
    ///
    /// `addr` references the first byte of a frame and must therefore be
    /// a multiple of the frame size.
    pub fn frame_ref(&self, addr: &u64) -> Result<&[u8], UmemAccessError> {
        self.check_frame_addr_valid(&addr)?;

        let offset: usize = (*addr).try_into().unwrap();

        Ok(self
            .mmap_area
            .mem_range(offset, self.frame_size_usize)
            .unwrap())
    }

    /// Return a mutable reference to the frame at a given address. Can be used
    /// if writing to the UMEM, for example before transmitting a packet.
    ///
    /// `addr` references the first byte of a frame and must therefore be
    /// a multiple of the frame size.
    ///
    /// Remember that if you write data to a frame, you must update the length on
    /// the corresponding [FrameDesc](struct.FrameDesc.html) before submitting to
    /// the [TxQueue](struct.TxQueue.html).
    pub fn frame_ref_mut(&mut self, addr: &u64) -> Result<&mut [u8], UmemAccessError> {
        self.check_frame_addr_valid(&addr)?;

        let offset: usize = (*addr).try_into().unwrap();

        Ok(self
            .mmap_area
            .mem_range_mut(offset, self.frame_size_usize)
            .unwrap())
    }

    /// Copy `data` into the frame at `addr`. Consider using
    /// `copy_data_into_frame` instead as less book-keeping required.
    ///
    /// `addr` references the first byte of a frame and must therefore be
    /// a multiple of the frame size. The length of `data` must be less
    /// than or equal to the frame size.
    ///
    /// Returns the number of bytes copied. Remember that once data has
    /// been written to a frame, you must update the length on
    /// the corresponding [FrameDesc](struct.FrameDesc.html) before submitting
    /// to the [TxQueue](struct.TxQueue.html).
    pub fn copy_data_to_frame_at_addr(
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

    /// Copy `data` into the frame described by `frame_desc`.
    ///
    /// Similar to `copy_data_to_frame_at_addr` but it sets the length
    /// on `frame_desc` if copying the data was successful, thereby
    /// avoiding having to remember to set it yourself. The length of
    /// `data` must be less than or equal to the frame size.
    pub fn copy_data_to_frame(
        &mut self,
        frame_desc: &mut FrameDesc,
        data: &[u8],
    ) -> Result<(), UmemAccessError> {
        if data.len() == 0 {
            frame_desc.set_len(0);
            return Ok(());
        }

        self.check_data_valid(data)?;

        let frame_ref = self.frame_ref_mut(&frame_desc.addr())?;

        frame_ref[..data.len()].copy_from_slice(data);

        frame_desc.set_len(data.len().try_into().unwrap());

        Ok(())
    }
}

impl Drop for Umem<'_> {
    fn drop(&mut self) {
        log::debug!("calling xsk_umem__delete");
        let err = unsafe { libbpf_sys::xsk_umem__delete(self.inner.as_mut()) };

        if err != 0 {
            log::error!("xsk_umem__delete failed: {}", err);
        }
    }
}

impl FillQueue<'_> {
    /// Let the kernel know that the frames in `descs` may be used to receive data.
    ///
    /// Note that if the length of `descs` is greater than the number of available spaces on the
    /// underlying ring buffer then no frames at all will be handed over to the kernel.
    ///
    /// This function returns the number of frames submitted to the kernel. Due to the
    /// constraint mentioned in the above paragraph, this should always be the length of
    /// `descs` or `0`.
    ///
    /// Once the frames have been submitted they should not be used again until consumed again
    /// via the [RxQueue](struct.RxQueue.html)
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
        unsafe { libbpf_sys::_xsk_ring_prod__needs_wakeup(self.inner.as_ref()) != 0 }
    }
}

impl CompQueue<'_> {
    /// Update `descs` with frames whose contents have been sent (after submission via
    /// the [TxQueue](struct.TxQueue.html) and may now be used again.
    ///
    /// The number of entries updated will be less than or equal to the length of `descs`.
    /// Entries will be updated sequentially from the start of `descs` until the end.
    /// Returns the number of elements of `descs` which have been updated.
    ///
    /// Free frames should be added back on to either the [FillQueue](struct.FillQueue.html)
    /// for data receipt or the [TxQueue](struct.TxQueue.html) for data transmission.
    pub fn consume(&mut self, descs: &mut [FrameDesc]) -> usize {
        let nb: u64 = descs.len().try_into().unwrap();

        if nb == 0 {
            return 0;
        }

        let mut idx: u32 = 0;

        let cnt = unsafe { libbpf_sys::_xsk_ring_cons__peek(self.inner.as_mut(), nb, &mut idx) };

        if cnt > 0 {
            for desc in descs.iter_mut().take(cnt.try_into().unwrap()) {
                let addr =
                    unsafe { *libbpf_sys::_xsk_ring_cons__comp_addr(self.inner.as_mut(), idx) };

                desc.set_addr(addr);
                desc.set_len(0);
                desc.set_options(0);

                idx += 1;
            }

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
    use crate::umem::Config;

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
        )
        .unwrap()
    }

    fn umem<'a>() -> (Umem<'a>, FillQueue<'a>, CompQueue<'a>, Vec<FrameDesc>) {
        let config = umem_config();

        Umem::builder(config)
            .create_mmap()
            .expect("Failed to create mmap region")
            .create_umem()
            .expect("Failed to create UMEM")
    }

    #[test]
    fn umem_create_succeeds_when_frame_count_is_one() {
        let config = Config::new(
            NonZeroU32::new(1).unwrap(),
            NonZeroU32::new(4096).unwrap(),
            4,
            4,
            0,
            false,
        )
        .unwrap();

        Umem::builder(config)
            .create_mmap()
            .expect("Failed to create mmap region")
            .create_umem()
            .expect("Failed to create UMEM");
    }

    #[test]
    fn umem_create_succeeds_when_fill_size_is_one() {
        let config = Config::new(
            NonZeroU32::new(16).unwrap(),
            NonZeroU32::new(4096).unwrap(),
            1,
            4,
            0,
            false,
        )
        .unwrap();

        Umem::builder(config)
            .create_mmap()
            .expect("Failed to create mmap region")
            .create_umem()
            .expect("Failed to create UMEM");
    }

    #[test]
    fn umem_create_succeeds_when_comp_size_is_one() {
        let config = Config::new(
            NonZeroU32::new(16).unwrap(),
            NonZeroU32::new(4096).unwrap(),
            4,
            1,
            0,
            false,
        )
        .unwrap();

        Umem::builder(config)
            .create_mmap()
            .expect("Failed to create mmap region")
            .create_umem()
            .expect("Failed to create UMEM");
    }

    #[test]
    #[should_panic]
    fn umem_create_fails_when_frame_size_is_lt_2048() {
        let config = Config::new(
            NonZeroU32::new(1).unwrap(),
            NonZeroU32::new(2047).unwrap(),
            4,
            4,
            0,
            false,
        )
        .unwrap();

        Umem::builder(config)
            .create_mmap()
            .expect("Failed to create mmap region")
            .create_umem()
            .expect("Failed to create UMEM");
    }

    #[test]
    fn frame_addr_checks_ok() {
        let (umem, _fq, _cq, _frame_descs) = umem();

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
        let (umem, _fq, _cq, _frame_descs) = umem();

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
    fn write_to_umem_frame_addr_then_read_small_byte_array() {
        let (mut umem, _fq, _cq, _frame_descs) = umem();

        let addr = 0;
        let data = [b'H', b'e', b'l', b'l', b'o'];

        umem.copy_data_to_frame_at_addr(&addr, &data[..]).unwrap();

        let frame_ref = umem.frame_ref(&addr).unwrap();

        assert_eq!(data, frame_ref[..data.len()]);
    }

    #[test]
    fn write_no_data_to_umem_frame() {
        let (mut umem, _fq, _cq, mut frame_descs) = umem();

        let data = [];

        umem.copy_data_to_frame(&mut frame_descs[0], &data[..])
            .unwrap();

        assert_eq!(frame_descs[0].len(), 0);
    }

    #[test]
    fn write_to_umem_frame_then_read_small_byte_array() {
        let (mut umem, _fq, _cq, mut frame_descs) = umem();

        let data = [b'H', b'e', b'l', b'l', b'o'];

        umem.copy_data_to_frame(&mut frame_descs[0], &data[..])
            .unwrap();

        assert_eq!(frame_descs[0].len(), 5);

        let frame_ref = umem.frame_ref(&frame_descs[0].addr()).unwrap();

        assert_eq!(data, frame_ref[..data.len()]);
    }

    #[test]
    fn write_max_bytes_to_neighbouring_umem_frames() {
        let (mut umem, _fq, _cq, _frame_descs) = umem();

        // Create random data and write to adjacent frames
        let fst_addr = 0;
        let snd_addr = FRAME_SIZE as u64;

        let fst_data = generate_random_bytes(FRAME_SIZE);
        let snd_data = generate_random_bytes(FRAME_SIZE);

        umem.copy_data_to_frame_at_addr(&fst_addr, &fst_data)
            .unwrap();
        umem.copy_data_to_frame_at_addr(&snd_addr, &snd_data)
            .unwrap();

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
