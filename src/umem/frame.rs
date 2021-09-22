use super::mmap::MmapArea;
use crate::FrameDesc;
use std::convert::TryInto;
use std::fmt::{Debug, Formatter};
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::ops::{Deref, DerefMut};
use std::ptr::NonNull;
use std::slice;
use std::sync::Arc;

// ToDo: Handle headroom

/// An owned frame in a [super::Umem] [super::mmap::MmapArea]
///
/// ... Todo: just notes
///
/// * Frame owns a unique part of umem
/// * Uninitialized bytes after the initialized parts can not be read (except with unsafe).
///     * -> To make it imposible to read data of older packets.
///
/// The interface for this struct is influenced by
/// [`heapless::Vec`](https://docs.rs/heapless/0.7.5/heapless/struct.Vec.html) as it also manages
/// a variable length data slice in a fixed length allocation.
pub struct Frame {
    /// This reference makes sure that the underlying memory is still available
    _mmap_area: Arc<MmapArea>,
    base_ptr: NonNull<u8>,
    capacity: usize,
    desc: FrameDesc<'static>,
}

impl Frame {
    /// Create a new Frame which lives in `mmap_area` and is described by `frame_desc`
    ///
    /// # Panic
    /// If the length given in the descriptor is larger than the size of a frame in the `MmapArea`
    /// this function panics. Or if the frame_desc points outside of the `MmapArea`.
    ///
    /// # Safety
    /// `frame_desc` must point to a non-aliased region in `mmap_area`.
    pub unsafe fn new(mmap_area: Arc<MmapArea>, desc: FrameDesc<'static>) -> Self {
        assert!(desc.len() <= mmap_area.size.frame_size());
        let capacity = mmap_area.size.frame_size();

        let mmap_base = mmap_area.mem_ptr as *mut u8;
        let base_ptr = mmap_base.offset(
            desc.addr()
                .try_into()
                .expect("Offset does not fill up half of the address space"),
        );

        // Check base_ptr points into mmap area and is not null
        let valid_range = (mmap_base as usize)
            ..=(mmap_base.offset(
                (mmap_area.size.total_bytes() - mmap_area.size.frame_size())
                    .try_into()
                    .expect("mmap does not fill half of the address space"),
            ) as usize);
        assert!(valid_range.contains(&(base_ptr as usize)));
        let base_ptr = NonNull::new(base_ptr).expect("Mmap_area is non null");

        Frame {
            _mmap_area: mmap_area,
            base_ptr,
            capacity,
            desc,
        }
    }

    /// Set the size of the frame to 0
    pub fn clear(&mut self) {
        // Safety: 0 is always shorter as the initialized length
        unsafe { self.set_len(0) };
    }

    /// Get a view of the initialized part of the Frame
    pub fn as_slice(&self) -> &[u8] {
        &self.full_cap_slice()[..self.len()]
    }

    /// Get a mutable view of the initialized parts of the Frame
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        let len = self.len();
        &mut self.full_cap_slice_mut()[..len]
    }

    /// Get the number of initialized bytes
    pub fn len(&self) -> usize {
        self.desc.len()
    }

    /// Set the number of initialized bytes
    ///
    /// # Safety
    /// * ToDo: may not expose unilitialized data
    pub unsafe fn set_len(&mut self, new_len: usize) {
        self.desc.set_len(new_len)
    }

    /// Add a byte to the end of the frame
    pub fn push(&mut self, val: u8) -> Result<(), ()> {
        let len = self.len();
        if len == self.capacity {
            return Err(());
        }

        self.full_cap_slice_mut()[len] = val;

        // Safety: The new value was just initialized
        unsafe { self.set_len(len + 1) };

        Ok(())
    }

    /// Set the new length of the frame
    ///
    /// If the new size is larger then the old, the new bytes will be filled with zeroes.
    pub fn resize(&mut self, new_size: usize) {
        if new_size > self.len() {
            let len = self.len();
            self.full_cap_slice_mut()[len..new_size].fill(0);
        }

        // Safety: We just filled up the new space with zeroes so it is initialized
        unsafe { self.set_len(new_size) };
    }

    /// Get the frame descriptor for this frame
    pub fn frame_desc(&self) -> &FrameDesc {
        &self.desc
    }

    /// Get a view to the full space available to the Frame including uninitialized bytes
    fn full_cap_slice(&self) -> &[u8] {
        unsafe { slice::from_raw_parts(self.base_ptr.as_ptr(), self.capacity) }
    }

    /// Get a mutable view to the full space available to the Frame including uninitialized bytes
    fn full_cap_slice_mut(&mut self) -> &mut [u8] {
        unsafe { slice::from_raw_parts_mut(self.base_ptr.as_ptr(), self.capacity) }
    }
}

// Safety: We have unique ownership over the base_ptr. All other parts of the struct are auto Send
unsafe impl Send for Frame {}

// Safety: We do not use interior mutability, all mutations are performed through a &mut Frame
unsafe impl Sync for Frame {}

// Traits that cause Frame to behave similar to a &[u8] or a Vec<u8>
impl Deref for Frame {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl DerefMut for Frame {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.as_mut_slice()
    }
}

impl AsRef<[u8]> for Frame {
    fn as_ref(&self) -> &[u8] {
        self
    }
}

impl AsMut<[u8]> for Frame {
    fn as_mut(&mut self) -> &mut [u8] {
        self
    }
}

impl Write for Frame {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let len = self.len();
        let mut target = &mut self.full_cap_slice_mut()[len..];

        let written = target.write(buf)?;
        // Safety: `target.write` initialized the new memory region
        unsafe { self.set_len(len + written) };

        assert!(self.len() <= self.capacity);

        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Extend<u8> for Frame {
    fn extend<T: IntoIterator<Item = u8>>(&mut self, iter: T) {
        for val in iter {
            match self.push(val) {
                Ok(_) => {}
                Err(_) => break,
            }
        }
    }
}

impl PartialEq for Frame {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl Eq for Frame {}

impl PartialEq<&[u8]> for Frame {
    fn eq(&self, other: &&[u8]) -> bool {
        self.as_slice() == *other
    }
}

impl Hash for Frame {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_slice().hash(state)
    }
}

impl Debug for Frame {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Frame").field(&self.as_slice()).finish()
    }
}
