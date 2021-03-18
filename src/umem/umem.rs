use libbpf_sys::{xsk_ring_cons, xsk_ring_prod, xsk_umem, xsk_umem_config, XDP_PACKET_HEADROOM};
use std::sync::{Arc, Mutex};
use std::{convert::TryInto, error::Error, fmt, io, marker::PhantomData, mem::MaybeUninit, ptr};

use crate::socket::{self, Fd};

use super::{config::Config, mmap::MmapArea};

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

struct XskUmem(*mut xsk_umem);
unsafe impl Send for XskUmem {}
impl Drop for XskUmem {
    fn drop(&mut self) {
        let err = unsafe { libbpf_sys::xsk_umem__delete(self.0) };

        if err != 0 {
            log::error!("xsk_umem__delete() failed: {}", err);
        }
    }
}

/// A region of virtual contiguous memory divided into equal-sized
/// frames.  It provides the underlying working memory for an AF_XDP
/// socket.
pub struct Umem<'a> {
    config: Config,
    frame_size: usize,
    umem_len: usize,
    mtu: usize,
    inner: Arc<Mutex<XskUmem>>,
    mmap_area: MmapArea,
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
    /// Once we've successfully requested a region of memory, create
    /// the UMEM with it by splitting the memory region into frames
    /// and creating the [FillQueue](struct.FillQueue.html) and
    /// [CompQueue](struct.CompQueue.html).
    pub fn create_umem(
        mut self,
    ) -> io::Result<(Umem<'a>, FillQueue<'a>, CompQueue<'a>, Vec<FrameDesc<'a>>)> {
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
            let e = errno::errno();
            return Err(io::Error::from_raw_os_error(e.0));
        }

        // Upcasting u32 -> size_of<usize> = size_of<u64> is ok, the
        // latter equality being guaranteed by the crate's top level
        // conditional compilation flags (see lib.rs)
        let frame_size = self.config.frame_size() as usize;
        let frame_count = self.config.frame_count() as usize;

        let frame_headroom = self.config.frame_headroom() as usize;
        let xdp_packet_headroom = XDP_PACKET_HEADROOM as usize;
        let mtu = frame_size - (xdp_packet_headroom + frame_headroom);

        let mut frame_descs: Vec<FrameDesc> = Vec::with_capacity(frame_count);

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

            frame_descs.push(frame_desc);
        }

        let umem = Umem {
            config: self.config,
            frame_size,
            umem_len: frame_count * frame_size,
            mtu,
            inner: unsafe { Arc::new(Mutex::new(XskUmem(umem_ptr))) },
            mmap_area: self.mmap_area,
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

    /// Config used for building the UMEM.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// The maximum transmission unit, this determines
    /// the largest packet that may be sent.
    ///
    /// Equal to `frame_size - (XDP_PACKET_HEADROOM + frame_headroom)`.
    #[inline]
    pub fn mtu(&self) -> usize {
        self.mtu
    }

    pub(crate) fn as_mut_ptr(&mut self) -> *mut xsk_umem {
        self.inner.lock().expect("failed to lock").0
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

    /// Return a reference to the UMEM region starting at `addr` of
    /// length `len`.
    ///
    /// This does not check that the region accessed makes sense and
    /// may cause undefined behaviour if used improperly. An example
    /// of potential misuse is referencing a region that extends
    /// beyond the end of the UMEM.
    ///
    /// Apart from the memory considerations, this function is also
    /// `unsafe` as there is no guarantee the kernel isn't also
    /// reading from or writing to the same region.
    #[inline]
    pub unsafe fn read_from_umem(&self, addr: &usize, len: &usize) -> &[u8] {
        self.mmap_area.mem_range(*addr, *len)
    }

    /// Checked version of `umem_region_ref`. Ensures that the
    /// referenced region is contained within a single frame of the
    /// UMEM.
    #[inline]
    pub unsafe fn read_from_umem_checked(
        &self,
        addr: &usize,
        len: &usize,
    ) -> Result<&[u8], AccessError> {
        self.is_access_valid(&addr, &len)?;

        Ok(self.mmap_area.mem_range(*addr, *len))
    }

    /// Copy `data` to the region starting at `frame_desc.addr`, and
    /// set `frame_desc.len` when done.
    ///
    /// This does no checking that the region written to makes sense
    /// and may cause undefined behaviour if used improperly. An
    /// example of potential misuse is writing beyond the end of the
    /// UMEM, or if `data` is large then potentially writing across
    /// frame boundaries.
    ///
    /// Apart from the considerations around writing to memory, this
    /// function is also `unsafe` as there is no guarantee the kernel
    /// isn't also reading from or writing to the same region.
    #[inline]
    pub unsafe fn write_to_umem(&mut self, frame_desc: &mut FrameDesc, data: &[u8]) {
        let data_len = data.len();

        if data_len > 0 {
            let umem_region = self.mmap_area.mem_range_mut(&frame_desc.addr(), &data_len);

            umem_region[..data_len].copy_from_slice(data);
        }

        frame_desc.set_len(data_len);
    }

    /// Checked version of `write_to_umem_frame`. Ensures that a
    /// successful write is completely contained within a single frame
    /// of the UMEM.
    #[inline]
    pub unsafe fn write_to_umem_checked(
        &mut self,
        frame_desc: &mut FrameDesc,
        data: &[u8],
    ) -> Result<(), WriteError> {
        let data_len = data.len();

        if data_len > 0 {
            self.is_data_valid(data).map_err(|e| WriteError::Data(e))?;

            self.is_access_valid(&frame_desc.addr(), &data_len)
                .map_err(|e| WriteError::Access(e))?;

            let umem_region = self.mmap_area.mem_range_mut(&frame_desc.addr(), &data_len);

            umem_region[..data_len].copy_from_slice(data);
        }

        frame_desc.set_len(data_len);

        Ok(())
    }

    /// Return a reference to the UMEM region starting at `addr` of
    /// length `len`.
    ///
    /// This does not check that the region accessed makes sense and
    /// may cause undefined behaviour if used improperly. An example
    /// of potential misuse is referencing a region that extends
    /// beyond the end of the UMEM.
    ///
    /// Apart from the memory considerations, this function is also
    /// `unsafe` as there is no guarantee the kernel isn't also
    /// reading from or writing to the same region.
    ///
    /// If data is written to a frame, the length on the corresponding
    /// [FrameDesc](struct.FrameDesc.html) for `addr` must be updated
    /// before submitting to the [TxQueue](struct.TxQueue.html). This
    /// ensures the correct number of bytes are sent. Use
    /// `write_to_umem` or `write_to_umem_checked` to avoid the
    /// overhead of updating the frame descriptor.
    #[inline]
    pub unsafe fn umem_region_mut(&mut self, addr: &usize, len: &usize) -> &mut [u8] {
        self.mmap_area.mem_range_mut(&addr, &len)
    }

    /// Checked version of `umem_region_mut`. Ensures the requested
    /// region lies within a single frame.
    #[inline]
    pub unsafe fn umem_region_mut_checked(
        &mut self,
        addr: &usize,
        len: &usize,
    ) -> Result<&mut [u8], AccessError> {
        self.is_access_valid(addr, len)?;

        Ok(self.mmap_area.mem_range_mut(&addr, &len))
    }

    pub fn split(
        self,
        frame_desc: Vec<FrameDesc>,
        tx_size: usize,
    ) -> (Umem, Umem, Vec<FrameDesc>, Vec<FrameDesc>) {
        let frames_tx = frame_desc[..tx_size].into();
        let frames_rx = frame_desc[tx_size..].into();

        let offset = (self.config.frame_size() as usize) * tx_size;
        let (mut mmap_tx, mut mmap_rx) = self.mmap_area.split(offset);

        eprintln!("offset = {}", offset);
        eprintln!("mmap tx ptr = {:?}", mmap_tx.as_mut_ptr());
        eprintln!("mmap rx ptr = {:?}", mmap_rx.as_mut_ptr());

        let umem_tx = Umem {
            config: self.config.clone(),
            frame_size: self.frame_size,
            umem_len: tx_size,
            inner: self.inner.clone(),
            mtu: self.mtu,
            mmap_area: mmap_tx,
            _marker: PhantomData,
        };

        let umem_rx = Umem {
            config: self.config.clone(),
            frame_size: self.frame_size,
            umem_len: self.umem_len - tx_size,
            inner: self.inner,
            mtu: self.mtu,
            mmap_area: mmap_rx,
            _marker: PhantomData,
        };

        (umem_tx, umem_rx, frames_tx, frames_rx)
    }
}

impl FillQueue<'_> {
    /// Let the kernel know that the frames in `descs` may be used to
    /// receive data.
    ///
    /// This function is marked `unsafe` as it is possible to cause a
    /// data race by simultaneously submitting the same frame
    /// descriptor to the fill ring and the Tx ring, for example.
    /// Once the frames have been submitted they should not be used
    /// again until consumed again via the
    /// [RxQueue](struct.RxQueue.html).
    ///
    /// Note that if the length of `descs` is greater than the number
    /// of available spaces on the underlying ring buffer then no
    /// frames at all will be handed over to the kernel.
    ///
    /// This returns the number of frames submitted to the kernel. Due
    /// to the constraint mentioned in the above paragraph, this
    /// should always be the length of `descs` or `0`.
    #[inline]
    pub unsafe fn produce(&mut self, descs: &[FrameDesc]) -> usize {
        // usize <-> u64 'as' conversions are ok as the crate's top
        // level conditional compilation flags (see lib.rs) guarantee
        // that size_of<usize> = size_of<u64>
        let nb = descs.len() as u64;

        if nb == 0 {
            return 0;
        }

        let mut idx: u32 = 0;

        let cnt = libbpf_sys::_xsk_ring_prod__reserve(self.inner.as_mut(), nb, &mut idx);
        eprintln!("xsk_ring_prod__reserve cnt = {}", cnt);

        if cnt > 0 {
            for desc in descs.iter().take(cnt.try_into().unwrap()) {
                *libbpf_sys::_xsk_ring_prod__fill_addr(self.inner.as_mut(), idx) = desc.addr as u64;

                idx += 1;
            }

            libbpf_sys::_xsk_ring_prod__submit(self.inner.as_mut(), cnt);
        }

        cnt.try_into().unwrap()
    }

    /// Same as `produce` but wake up the kernel (if required) to let
    /// it know there are frames available that may be used to receive
    /// data.
    ///
    /// For more details see the
    /// [docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html#xdp-use-need-wakeup-bind-flag).
    ///
    /// This function is marked `unsafe` for the same reasons that
    /// `produce` is `unsafe`.
    #[inline]
    pub unsafe fn produce_and_wakeup(
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
    #[inline]
    pub fn consume(&mut self, descs: &mut [FrameDesc]) -> usize {
        // usize <-> u64 'as' conversions are ok as the crate's top
        // level conditional compilation flags (see lib.rs) guarantee
        // that size_of<usize> = size_of<u64>
        let nb = descs.len() as u64;

        if nb == 0 {
            return 0;
        }

        let mut idx: u32 = 0;

        let cnt = unsafe { libbpf_sys::_xsk_ring_cons__peek(self.inner.as_mut(), nb, &mut idx) };

        if cnt > 0 {
            for desc in descs.iter_mut().take(cnt.try_into().unwrap()) {
                let addr: u64 =
                    unsafe { *libbpf_sys::_xsk_ring_cons__comp_addr(self.inner.as_mut(), idx) };

                desc.set_addr(addr as usize);
                desc.set_len(0);
                desc.set_options(0);

                idx += 1;
            }

            unsafe { libbpf_sys::_xsk_ring_cons__release(self.inner.as_mut(), cnt) };
        }

        cnt.try_into().unwrap()
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

    fn umem<'a>() -> (Umem<'a>, FillQueue<'a>, CompQueue<'a>, Vec<FrameDesc<'a>>) {
        let config = umem_config();

        Umem::builder(config)
            .create_mmap()
            .expect("Failed to create mmap region")
            .create_umem()
            .expect("Failed to create UMEM")
    }

    #[test]
    fn umem_create_split() {
        let config = umem_config();

        let (umem, fq, cq, frame_descs) = Umem::builder(config)
            .create_mmap()
            .expect("Failed to create mmap region")
            .create_umem()
            .expect("Failed to create UMEM");

        let (umem_tx, umem_rx, frame_descs_tx, frame_descs_rx) = umem.split(frame_descs, 8);

        assert!(false)
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
