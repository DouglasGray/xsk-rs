use libbpf_sys::{xsk_ring_cons, xsk_ring_prod, xsk_umem, xsk_umem_config, XDP_PACKET_HEADROOM};
use std::{convert::TryInto, error::Error, fmt, io, marker::PhantomData, mem::MaybeUninit, ptr};

use crate::socket::{self, Fd};

use super::{config::Config, mmap::MmapArea};
use crate::umem::mmap::MmapShape;
use crate::umem::Frame;
use std::convert::TryFrom;

use std::sync::Arc;

/// Describes a UMEM frame's address and size of its current contents.
///
/// The `addr` field is an offset in bytes from the start of the UMEM
/// and corresponds to some point within a frame. The `len` field
/// describes the length (in bytes) of any data stored in that frame,
/// starting from `addr`.
///
/// If sending data, the `len` field will need to be set by the user
/// before transmitting via the [TxQueue](struct.TxQueue.html).
/// Otherwise when reading received frames using the
/// [RxQueue](struct.RxQueue.html), the `len` field will be set by the
/// kernel and dictates the number of bytes the user should read from
/// the UMEM.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameDesc<'umem> {
    addr: usize,
    len: usize,
    options: u32,
    _marker: PhantomData<&'umem ()>,
}

impl FrameDesc<'_> {
    pub fn new() -> Self {
        Self {
            addr: 0,
            len: 0,
            options: 0,
            _marker: PhantomData,
        }
    }

    #[inline]
    pub fn addr(&self) -> usize {
        self.addr
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub fn options(&self) -> u32 {
        self.options
    }

    /// Set the frame descriptor's address. This determines where in
    /// the UMEM it references.
    ///
    /// Manual setting shouldn't generally be required is likely best
    /// avoided since the setting of addresses is handled by the
    /// library, however it may be needed if writing straight to a
    /// region in UMEM via
    /// [umem_region_mut](struct.Umem.html#method.umem_region_mut) or
    /// [umem_region_mut_checked](struct.Umem.html#method.umem_region_mut_checked)
    #[inline]
    pub fn set_addr(&mut self, addr: usize) {
        self.addr = addr
    }

    /// Set the frame descriptor's length. This should equal the
    /// length of the data stored at `addr`.
    ///
    /// Once data has been written to the UMEM region starting at
    /// `addr`, this `FrameDesc`'s length must be updated before
    /// handing it over to the kernel to be transmitted to ensure the
    /// correct number of bytes are sent.
    ///
    /// Manual setting shouldn't generally be required and if copying
    /// packets to UMEM it's better to use
    /// [write_to_umem](struct.Umem.html#method.write_to_umem) or
    /// [write_to_umem_checked](struct.Umem.html#method.write_to_umem_checked)
    /// which will handle setting the frame descriptor length, however
    /// it may be needed if writing to a UMEM region manually (see
    /// `set_addr`) or if, for example, the data you want to send is
    /// already at `addr` and you just need to set the length before
    /// transmission.
    #[inline]
    pub fn set_len(&mut self, len: usize) {
        self.len = len
    }

    /// Set the frame descriptor options.
    #[inline]
    pub fn set_options(&mut self, options: u32) {
        self.options = options
    }
}

/// Initial step for building a UMEM. This creates the underlying
/// `mmap` area.
pub struct UmemBuilder {
    config: Config,
}

/// Use the `mmap`'d region to create the UMEM.
pub struct UmemBuilderWithMmap {
    config: Config,
    mmap_area: MmapArea,
}

/// A region of virtual contiguous memory divided into equal-sized
/// frames.  It provides the underlying working memory for an AF_XDP
/// socket.
pub struct Umem<'a> {
    config: Config,
    frame_size: usize,
    umem_len: usize,
    mtu: usize,
    inner: Box<xsk_umem>,
    mmap_area: Arc<MmapArea>,
    _marker: PhantomData<&'a ()>,
}

/// Used to transfer ownership of UMEM frames from kernel-space to
/// user-space.
///
/// Frames received in this queue are those that have been sent via
/// the [TxQueue](struct.TxQueue.html).
///
/// For more information see the
/// [docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html#umem-completion-ring)
pub struct CompQueue<'umem> {
    inner: Box<xsk_ring_cons>,
    umem: Arc<Umem<'umem>>,
    _marker: PhantomData<&'umem ()>,
}

/// Used to transfer ownership of UMEM frames from user-space to
/// kernel-space.
///
/// These frames will be used to receive packets, and will eventually
/// be returned via the [RxQueue](struct.RxQueue.html).
///
/// For more information see the
/// [docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html#umem-fill-ring)
pub struct FillQueue<'umem> {
    inner: Box<xsk_ring_prod>,
    _umem: Arc<Umem<'umem>>,
    _marker: PhantomData<&'umem ()>,
}

impl UmemBuilder {
    /// Allocate a memory region for the UMEM.
    ///
    /// Before we can create the UMEM we first need to allocate a
    /// chunk of memory, which will eventually be split up into
    /// frames. We do this with a call to `mmap`, requesting a read +
    /// write protected anonymous memory region.
    pub fn create_mmap(self) -> io::Result<UmemBuilderWithMmap> {
        let mmap_area = MmapArea::new(
            MmapShape::new(
                self.config.frame_count() as usize,
                self.config.frame_size() as usize,
            ),
            self.config.use_huge_pages(),
        )?;

        Ok(UmemBuilderWithMmap {
            config: self.config,
            mmap_area,
        })
    }
}

impl<'a> UmemBuilderWithMmap {
    /// Using the allocated memory region, create the UMEM.
    ///
    /// Once we've successfully requested a region of memory, create
    /// the UMEM with it by splitting the memory region into frames
    /// and creating the [FillQueue](struct.FillQueue.html) and
    /// [CompQueue](struct.CompQueue.html).
    pub fn create_umem(
        mut self,
    ) -> io::Result<(Arc<Umem<'a>>, FillQueue<'a>, CompQueue<'a>, Vec<Frame>)> {
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
                self.mmap_area.len() as u64,
                fq_ptr.as_mut_ptr(),
                cq_ptr.as_mut_ptr(),
                &umem_create_config,
            )
        };

        if err != 0 {
            return Err(io::Error::from_raw_os_error(err));
        }

        let mmap_area = Arc::new(self.mmap_area);

        // Upcasting u32 -> size_of<usize> = size_of<u64> is ok, the
        // latter equality being guaranteed by the crate's top level
        // conditional compilation flags (see lib.rs)
        let frame_size = self.config.frame_size() as usize;
        let frame_count = self.config.frame_count() as usize;

        let frame_headroom = self.config.frame_headroom() as usize;
        let xdp_packet_headroom = XDP_PACKET_HEADROOM as usize;
        let mtu = frame_size - (xdp_packet_headroom + frame_headroom);

        let mut frames: Vec<Frame> = Vec::with_capacity(frame_count);

        for i in 0..frame_count {
            let addr = i * frame_size;
            let len = 0;
            let options = 0;

            let frame_desc = FrameDesc {
                addr,
                len,
                options,
                _marker: PhantomData,
            };

            // Safety: We know that the frame_desc points to a unique part of the mmap area as we
            // loop over all the sections in the area in this loop.
            let frame = unsafe { Frame::new(Arc::clone(&mmap_area), frame_desc) };

            frames.push(frame);
        }

        let umem = Arc::new(Umem {
            config: self.config,
            frame_size,
            umem_len: frame_count * frame_size,
            mtu,
            inner: unsafe { Box::from_raw(umem_ptr) },
            mmap_area,
            _marker: PhantomData,
        });

        let fill_queue = FillQueue {
            inner: unsafe { Box::new(fq_ptr.assume_init()) },
            _umem: Arc::clone(&umem),
            _marker: PhantomData,
        };

        let comp_queue = CompQueue {
            inner: unsafe { Box::new(cq_ptr.assume_init()) },
            umem: Arc::clone(&umem),
            _marker: PhantomData,
        };

        Ok((umem, fill_queue, comp_queue, frames))
    }
}

impl Umem<'_> {
    pub fn builder(config: Config) -> UmemBuilder {
        UmemBuilder { config }
    }

    /// Config used for building the UMEM.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Return the mmap area used by this umem
    pub fn mmap_area(&self) -> &Arc<MmapArea> {
        &self.mmap_area
    }

    /// The maximum transmission unit, this determines
    /// the largest packet that may be sent.
    ///
    /// Equal to `frame_size - (XDP_PACKET_HEADROOM + frame_headroom)`.
    #[inline]
    pub fn mtu(&self) -> usize {
        self.mtu
    }

    pub(crate) fn as_ptr(&self) -> *const xsk_umem {
        self.inner.as_ref()
    }

    pub(crate) fn as_mut_ptr(&mut self) -> *mut xsk_umem {
        self.inner.as_mut()
    }

    /// Check if trying to access the UMEM region bound by [`addr`,
    /// `addr` + `len`] makes sense.
    #[inline]
    pub fn is_access_valid(&self, addr: &usize, len: &usize) -> Result<(), AccessError> {
        if *len == 0 {
            return Err(AccessError::NullRegion);
        }

        // Addr is offset in bytes and starts at 0, so need to subtract 1
        let region_end = *addr + (*len - 1);

        // >= instead of > since address starts at 0, for example if
        // frame_count = 2 and frame_size = 2048 then umem_len =
        // 4096. However the last valid addressable byte will be at
        // 4095.
        if region_end >= self.umem_len {
            return Err(AccessError::RegionOutOfBounds {
                addr: *addr,
                len: *len,
                umem_len: self.umem_len,
            });
        }

        if (*addr / self.frame_size) != (region_end / self.frame_size) {
            return Err(AccessError::CrossesFrameBoundary {
                addr: *addr,
                len: *len,
            });
        }

        Ok(())
    }

    /// Check if `data` is ok to write to the UMEM for transmission.
    #[inline]
    pub fn is_data_valid(&self, data: &[u8]) -> Result<(), DataError> {
        // Check if data is transmissable
        if data.len() > self.mtu {
            return Err(DataError::SizeExceedsMtu {
                data_len: data.len(),
                mtu: self.mtu,
            });
        }

        Ok(())
    }
}

impl Drop for Umem<'_> {
    fn drop(&mut self) {
        let err = unsafe { libbpf_sys::xsk_umem__delete(self.inner.as_mut()) };
        if err != 0 {
            log::error!("xsk_umem__delete() failed: {}", err);
        }
    }
}

impl FillQueue<'_> {
    /// Let the kernel know that the frames in `descs` may be used to
    /// receive data.
    ///
    /// Note that if the length of `descs` is greater than the number
    /// of available spaces on the underlying ring buffer then no
    /// frames at all will be handed over to the kernel.
    ///
    /// This returns the number of frames submitted to the kernel. Due
    /// to the constraint mentioned in the above paragraph, this
    /// should always be the length of `descs` or `0`.
    ///
    /// # Safety
    ///
    /// This function is `unsafe` as it is possible to cause a data
    /// race by simultaneously submitting the same frame descriptor to
    /// the fill ring and the Tx ring, for example.  Once the frames
    /// have been submitted they should not be used again until
    /// consumed again via the [RxQueue](struct.RxQueue.html).
    ///
    /// # Panic
    /// Panics if a `Frame` that is not belonging to this `Umem` is passed.

    // ToDo: This shares a lot of code with socket::TxQueue::produce add a function to deduplicate
    #[inline]
    #[must_use = "produce() returns the frames that have not been sent, as there was not enough room. These should be tried again later!"]
    pub fn produce(&mut self, mut frames: Vec<Frame>) -> Vec<Frame> {
        if frames.is_empty() {
            return frames;
        }

        let num_frames = frames
            .len()
            .try_into()
            .expect("number of frames fits size_t");

        let mut start_idx: u32 = 0;
        let cnt = unsafe {
            libbpf_sys::_xsk_ring_prod__reserve(self.inner.as_mut(), num_frames, &mut start_idx)
        };
        let start_idx = start_idx; // Remove mut
        let cnt: usize = cnt.try_into().expect("size_t fits into usize");

        assert!(
            u64::try_from(cnt).is_ok(),
            "Number of packets should fit into a u64"
        );
        assert!(
            cnt as u64 <= num_frames,
            "Kernel should at maximum return the number of frames we asked for"
        );

        let umem_mmap_area = self._umem.mmap_area();
        if cnt > 0 {
            // ToDo: Check if drain is correct here
            for (idx, frame) in frames.drain(..cnt).enumerate() {
                assert!(
                    Arc::ptr_eq(frame.mmap_area(), umem_mmap_area),
                    "a Umem can only take `Frame`s pointing into its `MmapArea`"
                );

                // Safety: ToDo
                let idx: u32 = idx.try_into().expect("number of frames fits u32");
                unsafe {
                    let addr =
                        libbpf_sys::_xsk_ring_prod__fill_addr(self.inner.as_mut(), start_idx + idx);
                    *addr = frame.frame_desc().addr as u64;
                }

                // ToDo: Do something with frame instead of dropping to avoid atomic decrease in ARC on every send
            }

            // Safety: ToDo
            unsafe {
                libbpf_sys::_xsk_ring_prod__submit(self.inner.as_mut(), cnt as u64);
            }
        }

        frames
    }

    /// Same as `produce` but wake up the kernel (if required) to let
    /// it know there are frames available that may be used to receive
    /// data.
    ///
    /// For more details see the
    /// [docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html#xdp-use-need-wakeup-bind-flag).
    ///
    /// # Safety
    ///
    /// This function is `unsafe` for the same reasons that `produce`
    /// is `unsafe`.
    #[inline]
    #[must_use = "produce_and_wakeup() returns the frames that have not been sent, as there was not enough room. These should be tried again later!"]
    pub unsafe fn produce_and_wakeup(
        &mut self,
        frames: Vec<Frame>,
        socket_fd: &mut Fd,
        poll_timeout: i32,
    ) -> io::Result<Vec<Frame>> {
        let old_len = frames.len();
        let remaining = self.produce(frames);

        if remaining.len() != old_len && self.needs_wakeup() {
            self.wakeup(socket_fd, poll_timeout)?;
        }

        Ok(remaining)
    }

    /// Wake up the kernel to let it know it can continue using the
    /// fill ring to process received data.
    ///
    /// See `produce_and_wakeup` for link to docs with further
    /// explanation.
    #[inline]
    pub fn wakeup(&self, fd: &mut Fd, poll_timeout: i32) -> io::Result<()> {
        socket::poll_read(fd, poll_timeout)?;
        Ok(())
    }

    /// Check if the libbpf `NEED_WAKEUP` flag is set on the fill
    /// ring.  If so then this means a call to `wakeup` will be
    /// required to continue processing received data with the fill
    /// ring.
    ///
    /// See `produce_and_wakeup` for link to docs with further
    /// explanation.
    #[inline]
    pub fn needs_wakeup(&self) -> bool {
        unsafe { libbpf_sys::_xsk_ring_prod__needs_wakeup(self.inner.as_ref()) != 0 }
    }
}

unsafe impl Send for FillQueue<'_> {}

impl CompQueue<'_> {
    /// Update `descs` with frames whose contents have been sent
    /// (after submission via the [TxQueue](struct.TxQueue.html) and
    /// may now be used again.
    ///
    /// The number of entries updated will be less than or equal to
    /// the length of `descs`.  Entries will be updated sequentially
    /// from the start of `descs` until the end.  Returns the number
    /// of elements of `descs` which have been updated.
    ///
    /// Free frames should be added back on to either the
    /// [FillQueue](struct.FillQueue.html) for data receipt or the
    /// [TxQueue](struct.TxQueue.html) for data transmission.

    // ToDo: This shares a lot of code with the Rx ring
    #[inline]
    pub fn consume(&mut self) -> Vec<Frame> {
        let mut start_idx: u32 = 0;

        // u64::MAX -> Try to get all frames
        let cnt = unsafe {
            libbpf_sys::_xsk_ring_cons__peek(self.inner.as_mut(), u64::MAX, &mut start_idx)
        };
        let start_idx = start_idx; // Remove mut
        let cnt: u32 = cnt.try_into().expect("size_t fits into usize");

        let mut frames = Vec::with_capacity(cnt.try_into().expect("u32 fits into a usize"));

        if cnt > 0 {
            let mmap_area = self.umem.mmap_area();

            for idx in (0..cnt).map(|idx| idx + start_idx) {
                let mut desc = FrameDesc::new();
                let addr: u64 =
                    unsafe { *libbpf_sys::_xsk_ring_cons__comp_addr(self.inner.as_mut(), idx) };

                desc.set_addr(addr as usize);
                desc.set_len(0);
                desc.set_options(0);

                // ToDo: Is this detailed enough?
                // Safety: The kernel can only give us frames back that we previously gave to it via
                // the tx queue. Thus the desc we receive mut be unique.
                let frame = unsafe { Frame::new(Arc::clone(mmap_area), desc) };
                frames.push(frame);
            }

            unsafe { libbpf_sys::_xsk_ring_cons__release(self.inner.as_mut(), cnt.into()) };
        }

        frames
    }
}

unsafe impl Send for CompQueue<'_> {}

/// UMEM access errors
#[derive(Debug)]
pub enum AccessError {
    /// Attempted to access a region with zero length.
    NullRegion,
    /// Attempted to access a region outside of the UMEM.
    RegionOutOfBounds {
        addr: usize,
        len: usize,
        umem_len: usize,
    },
    /// Attempted to access a region that intersects with two or more
    /// frames.
    CrossesFrameBoundary { addr: usize, len: usize },
}

impl fmt::Display for AccessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use AccessError::*;
        match self {
            NullRegion => write!(f, "region has zero length"),
            RegionOutOfBounds {
                addr,
                len,
                umem_len,
            } => write!(
                f,
                "UMEM region [{}, {}] is out of bounds (UMEM length is {})",
                addr,
                addr + (len - 1),
                umem_len
            ),
            CrossesFrameBoundary { addr, len } => write!(
                f,
                "UMEM region [{}, {}] intersects with more then one frame",
                addr,
                addr + (len - 1),
            ),
        }
    }
}

impl Error for AccessError {}

/// Data related errors
#[derive(Debug)]
pub enum DataError {
    /// Size of data written to UMEM for tx exceeds the MTU.
    SizeExceedsMtu { data_len: usize, mtu: usize },
}

impl fmt::Display for DataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DataError::SizeExceedsMtu { data_len, mtu } => write!(
                f,
                "data length ({} bytes) cannot be greater than the MTU ({} bytes)",
                data_len, mtu
            ),
        }
    }
}

impl Error for DataError {}

/// Errors that may occur when writing data to the UMEM.
#[derive(Debug)]
pub enum WriteError {
    Access(AccessError),
    Data(DataError),
}

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use WriteError::*;
        match self {
            Access(access_err) => write!(f, "{}", access_err),
            Data(data_err) => write!(f, "{}", data_err),
        }
    }
}

impl Error for WriteError {}

/*
ToDo: Move these tests over to MmapArea + Frame

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

    fn umem<'a>() -> (Umem<'a>, FillQueue<'a>, CompQueue<'a>, Vec<Frame>) {
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
    fn mtu_is_correct() {
        let config = Config::new(
            NonZeroU32::new(1).unwrap(),
            NonZeroU32::new(2048).unwrap(),
            4,
            4,
            512,
            false,
        )
        .unwrap();

        let (umem, _fq, _cq, _frame_descs) = Umem::builder(config)
            .create_mmap()
            .expect("Failed to create mmap region")
            .create_umem()
            .expect("Failed to create UMEM");

        assert_eq!(umem.mtu(), (2048 - XDP_PACKET_HEADROOM - 512) as usize);
    }

    #[test]
    fn umem_access_checks_ok() {
        let (umem, _fq, _cq, _frame_descs) = umem();

        let max_len = FRAME_SIZE as usize;

        assert!(umem.is_access_valid(&0, &1).is_ok());

        assert!(umem.is_access_valid(&0, &max_len).is_ok());

        assert!(matches!(
            umem.is_access_valid(&0, &(max_len + 1)),
            Err(AccessError::CrossesFrameBoundary { .. })
        ));

        let last_frame_addr = ((FRAME_COUNT - 1) * FRAME_SIZE) as usize;

        assert!(umem.is_access_valid(&last_frame_addr, &max_len).is_ok());

        assert!(matches!(
            umem.is_access_valid(&last_frame_addr, &(max_len + 1)),
            Err(AccessError::RegionOutOfBounds { .. })
        ));

        let umem_end = (FRAME_COUNT * FRAME_SIZE) as usize;

        assert!(matches!(
            umem.is_access_valid(&umem_end, &0),
            Err(AccessError::NullRegion)
        ));

        assert!(matches!(
            umem.is_access_valid(&umem_end, &1),
            Err(AccessError::RegionOutOfBounds { .. })
        ));
    }

    #[test]
    fn data_checks_ok() {
        let (umem, _fq, _cq, _frame_descs) = umem();

        // Empty data ok
        let empty_data: Vec<u8> = Vec::new();

        assert!(umem.is_data_valid(&empty_data).is_ok());

        let mtu = FRAME_SIZE - XDP_PACKET_HEADROOM;

        // Data within mtu ok
        let data = generate_random_bytes(mtu - 1);

        assert!(umem.is_data_valid(&data).is_ok());

        // Data exactly frame size is ok
        let data = generate_random_bytes(mtu);

        assert!(umem.is_data_valid(&data).is_ok());

        // Data greater than frame size fails
        let data = generate_random_bytes(mtu + 1);

        assert!(matches!(
            umem.is_data_valid(&data),
            Err(DataError::SizeExceedsMtu { .. })
        ));
    }

    #[test]
    fn write_no_data_to_umem() {
        let (mut umem, _fq, _cq, mut frame_descs) = umem();

        let data = [];

        unsafe {
            umem.write_to_umem_checked(&mut frame_descs[0], &data[..])
                .unwrap();
        }

        assert_eq!(frame_descs[0].len(), 0);
    }

    #[test]
    fn write_to_umem_frame_then_read_small_byte_array() {
        let (mut umem, _fq, _cq, mut frame_descs) = umem();

        let data = [b'H', b'e', b'l', b'l', b'o'];

        unsafe {
            umem.write_to_umem_checked(&mut frame_descs[0], &data[..])
                .unwrap();
        }

        assert_eq!(frame_descs[0].len(), 5);

        let umem_region = unsafe {
            umem.read_from_umem_checked(&frame_descs[0].addr, &frame_descs[0].len)
                .unwrap()
        };

        assert_eq!(data, umem_region[..data.len()]);
    }

    #[test]
    fn write_max_bytes_to_neighbouring_umem_frames() {
        let (mut umem, _fq, _cq, mut frame_descs) = umem();

        let data_len = FRAME_SIZE;

        // Create random data and write to adjacent frames
        let fst_data = generate_random_bytes(data_len);
        let snd_data = generate_random_bytes(data_len);

        unsafe {
            let umem_region = umem
                .umem_region_mut_checked(&frame_descs[0].addr(), &(data_len as usize))
                .unwrap();

            umem_region.copy_from_slice(&fst_data[..]);
            frame_descs[0].set_len(data_len as usize);

            let umem_region = umem
                .umem_region_mut_checked(&frame_descs[1].addr(), &(data_len as usize))
                .unwrap();

            umem_region.copy_from_slice(&snd_data[..]);
            frame_descs[1].set_len(data_len as usize);
        }

        let fst_frame_ref =
            unsafe { umem.read_from_umem(&frame_descs[0].addr(), &frame_descs[0].len()) };

        let snd_frame_ref =
            unsafe { umem.read_from_umem(&frame_descs[1].addr(), &frame_descs[1].len()) };

        // Check that they are indeed the samelet fst_frame_ref = umem.frame_ref_at_addr(&fst_addr).unwrap();
        assert_eq!(fst_data[..], fst_frame_ref[..fst_data.len()]);
        assert_eq!(snd_data[..], snd_frame_ref[..snd_data.len()]);

        // Ensure there are no gaps and the frames lie snugly
        let mem_len = (FRAME_SIZE * 2) as usize;

        let mem_range = unsafe { umem.mmap_area.mem_range(0, mem_len) };

        let mut data_vec = Vec::with_capacity(mem_len);

        data_vec.extend_from_slice(&fst_data);
        data_vec.extend_from_slice(&snd_data);

        assert_eq!(&data_vec[..], mem_range);
    }
}
*/
