use std::{
    io,
    num::NonZeroU32,
    ptr::NonNull,
    slice,
    sync::{Arc, Mutex},
};

pub use inner::Mmap;

use crate::FrameDesc;

use super::{
    frame::{Data, DataMut, Headroom, HeadroomMut},
    FrameLayout,
};

#[derive(Clone, Debug)]
pub struct UmemRegion {
    layout: FrameLayout,
    addr: NonNull<libc::c_void>,
    _mmap: Arc<Mutex<Mmap>>,
}

unsafe impl Send for UmemRegion {}

// SAFETY: this impl is only safe in the context of this library. The
// only mutators of the mmap'd region are the frames, which write to
// disjoint sections (assuming the unsafe requirements are upheld).
unsafe impl Sync for UmemRegion {}

impl UmemRegion {
    pub fn new(
        frame_count: NonZeroU32,
        frame_layout: FrameLayout,
        use_huge_pages: bool,
    ) -> io::Result<Self> {
        let len = (frame_count.get() as usize) * frame_layout.frame_size();

        let mmap = Mmap::new(len, use_huge_pages)?;

        Ok(Self {
            layout: frame_layout,
            addr: mmap.addr(),
            _mmap: Arc::new(Mutex::new(mmap)),
        })
    }

    /// Get a pointer to the start of the memory mapped region.
    #[inline]
    pub fn as_ptr(&self) -> *mut libc::c_void {
        self.addr.as_ptr()
    }

    /// Retrieve a pointer to the headroom segment of this frame.
    ///
    /// # Safety
    ///
    /// `addr` must point to the start of a packet data segment of a
    /// frame within this UMEM.
    #[inline]
    unsafe fn headroom_ptr(&self, desc: &FrameDesc) -> *mut u8 {
        let addr = desc.addr - self.layout.frame_headroom;
        unsafe { self.as_ptr().add(addr) as *mut u8 }
    }

    /// Retrieve a pointer to the packet data segment of this frame.
    ///
    /// # Safety
    ///
    /// `addr` must point to the start of a packet data segment of a
    /// frame within this UMEM.
    #[inline]
    unsafe fn data_ptr(&self, desc: &FrameDesc) -> *mut u8 {
        unsafe { self.as_ptr().add(desc.addr) as *mut u8 }
    }

    /// The frame's headroom and packet data segments.
    ///
    /// # Safety
    ///
    /// `desc` must describe a frame that belongs to this
    /// [`Umem`]. Furthermore, the underlying frame described by
    /// `desc` must not be mutably accessed anywhere else at the same
    /// time, either in userspace or by the kernel.
    #[inline]
    pub unsafe fn frame(&self, desc: &FrameDesc) -> (Headroom, Data) {
        // SAFETY: unsafe contract in constructor and `set_desc`
        // ensures that this frame's current `addr` points to the
        // start of a packet data segment in its underlying UMEM -
        // therefore the dereferenced slices are whole and valid
        // segments.
        //
        // The unsafe contract of this function also guarantees there
        // are no other mutable references to these slices at the same
        // time.
        unsafe { (self.headroom(desc), self.data(desc)) }
    }

    /// The frame's headroom segment.
    ///
    /// # Safety
    ///
    /// See [`get`](Frame::segments).
    #[inline]
    pub unsafe fn headroom(&self, desc: &FrameDesc) -> Headroom {
        // SAFETY: see `segments`.
        let headroom_ptr = unsafe { self.headroom_ptr(desc) };

        Headroom::new(unsafe { slice::from_raw_parts(headroom_ptr, desc.lengths.headroom) })
    }

    /// The frame's packet data segment
    ///
    /// # Safety
    ///
    /// See [`get`](Frame::segments).
    #[inline]
    pub unsafe fn data(&self, desc: &FrameDesc) -> Data {
        // SAFETY: see `segments`.
        let data_ptr = unsafe { self.data_ptr(desc) };

        Data::new(unsafe { slice::from_raw_parts(data_ptr, desc.lengths.data) })
    }

    /// Mutable references to the frame's headroom and packet data
    /// segments.
    ///
    /// # Safety
    ///
    /// The underlying [`Umem`](super::Umem) region this frame
    /// accesses must not be mutably or immutably accessed anywhere
    /// else at the same time, either in userspace or by the kernel.
    #[inline]
    pub unsafe fn frame_mut<'a>(
        &'a self,
        desc: &'a mut FrameDesc,
    ) -> (HeadroomMut<'a>, DataMut<'a>) {
        // SAFETY: unsafe contract in constructor and `set_desc`
        // ensures that this frame's current `addr` points to the
        // start of a packet data segment in its underlying UMEM -
        // therefore the dereferenced slices are whole and valid
        // segments.
        //
        // The unsafe contract of this function also guarantees there
        // are no other mutable or immutable references to these
        // slices at the same time.
        let headroom_ptr = unsafe { self.headroom_ptr(desc) };
        let data_ptr = unsafe { self.data_ptr(desc) };

        let headroom =
            unsafe { slice::from_raw_parts_mut(headroom_ptr, self.layout.frame_headroom) };

        let data = unsafe { slice::from_raw_parts_mut(data_ptr, self.layout.mtu) };

        (
            HeadroomMut::new(&mut desc.lengths.headroom, headroom),
            DataMut::new(&mut desc.lengths.data, data),
        )
    }

    /// A mutable reference to the frame's headroom segment.
    ///
    /// # Safety
    ///
    /// See [`get_mut`](Frame::segments_mut).
    #[inline]
    pub unsafe fn headroom_mut<'a>(&'a self, desc: &'a mut FrameDesc) -> HeadroomMut<'a> {
        // SAFETY: see `segments_mut`.
        let headroom_ptr = unsafe { self.headroom_ptr(desc) };

        let headroom =
            unsafe { slice::from_raw_parts_mut(headroom_ptr, self.layout.frame_headroom) };

        HeadroomMut::new(&mut desc.lengths.headroom, headroom)
    }

    /// A mutable reference to the frame's packet data segment.
    ///
    /// # Safety
    ///
    /// See [`get_mut`](Frame::segments_mut).
    #[inline]
    pub unsafe fn data_mut<'a>(&'a self, desc: &'a mut FrameDesc) -> DataMut<'a> {
        // SAFETY: see `segments_mut`.
        let data_ptr = unsafe { self.data_ptr(desc) };

        let data = unsafe { slice::from_raw_parts_mut(data_ptr, self.layout.mtu) };

        DataMut::new(&mut desc.lengths.data, data)
    }
}

#[cfg(not(test))]
mod inner {
    use super::*;

    use libc::{
        MAP_ANONYMOUS, MAP_FAILED, MAP_HUGETLB, MAP_POPULATE, MAP_SHARED, PROT_READ, PROT_WRITE,
    };
    use log::error;
    use std::ptr;

    /// An anonymous memory mapped region.
    #[derive(Debug)]
    pub struct Mmap {
        addr: NonNull<libc::c_void>, // Store a copy to avoid double deref.
        len: usize,
    }

    impl Mmap {
        pub fn new(len: usize, use_huge_pages: bool) -> io::Result<Self> {
            // MAP_ANONYMOUS: mapping not backed by a file.
            // MAP_SHARED: shares this mapping, so changes are visible
            // to other processes mapping the same file.
            // MAP_POPULATE: pre-populate page tables, reduces
            // blocking on page faults later.
            let mut flags = MAP_ANONYMOUS | MAP_SHARED | MAP_POPULATE;

            if use_huge_pages {
                flags |= MAP_HUGETLB;
            }

            let addr = unsafe {
                libc::mmap(
                    ptr::null_mut(),
                    len,
                    PROT_READ | PROT_WRITE, // prot
                    flags,
                    -1, // file
                    0,  // offset
                )
            };

            if addr == MAP_FAILED {
                Err(io::Error::last_os_error())
            } else {
                let addr =
                    NonNull::new(addr).expect("ptr non-null since we confirmed `mmap()` succeeded");

                Ok(Mmap { addr, len })
            }
        }

        #[inline]
        pub fn addr(&self) -> NonNull<libc::c_void> {
            self.addr
        }
    }

    impl Drop for Mmap {
        fn drop(&mut self) {
            let err = unsafe { libc::munmap(self.addr.as_ptr(), self.len) };

            if err != 0 {
                error!("`munmap()` failed with error code {}", err);
            }
        }
    }
}

#[cfg(test)]
mod inner {
    use std::mem::ManuallyDrop;

    use super::*;

    #[derive(Debug)]
    struct VecParts<T> {
        ptr: NonNull<T>,
        len: usize,
        capacity: usize,
    }

    impl<T> VecParts<T> {
        fn new(v: Vec<T>) -> Self {
            let mut v = ManuallyDrop::new(v);

            Self {
                ptr: NonNull::new(v.as_mut_ptr()).unwrap(),
                len: v.len(),
                capacity: v.capacity(),
            }
        }
    }

    impl<T> Drop for VecParts<T> {
        fn drop(&mut self) {
            unsafe { Vec::from_raw_parts(self.ptr.as_ptr(), self.len, self.capacity) };
        }
    }

    /// A mocked [`Mmap`] that uses the heap for memory.
    #[derive(Debug)]
    pub struct Mmap {
        mem: VecParts<u8>,
    }

    impl Mmap {
        pub fn new(len: usize, _use_huge_pages: bool) -> io::Result<Self> {
            Ok(Self {
                mem: VecParts::new(vec![0; len]),
            })
        }

        #[inline]
        pub fn addr(&self) -> NonNull<libc::c_void> {
            todo!()
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn confirm_pointer_offset_is_a_single_byte() {
        assert_eq!(std::mem::size_of::<libc::c_void>(), 1);
    }
}
