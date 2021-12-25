use std::{io, sync::Arc};

pub use inner::*;

#[cfg(not(test))]
mod inner {
    use super::*;

    use libc::{
        MAP_ANONYMOUS, MAP_FAILED, MAP_HUGETLB, MAP_POPULATE, MAP_SHARED, PROT_READ, PROT_WRITE,
    };
    use log::error;
    use std::ptr::{self, NonNull};

    struct MmapInner {
        addr: NonNull<libc::c_void>,
        len: usize,
    }

    impl MmapInner {
        /// # Safety
        ///
        /// Only one instance of this struct may exist since it unmaps
        /// the allocated memory as part of its [`Drop`] impl. If
        /// there are copies or clones of `addr` then care must be
        /// taken to ensure they aren't used once this struct goes out
        /// of scope, and that they don't unmap the memory themselves.
        unsafe fn new(addr: NonNull<libc::c_void>, len: usize) -> Self {
            Self { addr, len }
        }
    }

    impl Drop for MmapInner {
        fn drop(&mut self) {
            let err = unsafe { libc::munmap(self.addr.as_ptr(), self.len) };

            if err != 0 {
                error!("`munmap()` failed with error code {}", err);
            }
        }
    }

    /// An anonymous memory mapped region.
    #[derive(Clone)]
    pub struct Mmap {
        addr: NonNull<libc::c_void>, // Store a copy to avoid double deref.
        _inner: Arc<MmapInner>,
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
                    -1,               // file
                    0 as libc::off_t, // offset
                )
            };

            if addr == MAP_FAILED {
                Err(io::Error::last_os_error())
            } else {
                let addr =
                    NonNull::new(addr).expect("ptr non-null since we confirmed `mmap()` succeeded");

                // SAFETY: this is the only `MmapInner` instance for
                // this pointer. `Mmap` also owns a copy of the raw
                // pointer for quicker access, but it lives alongside
                // an `Arc<MmapInner>` so will never outlive this
                // struct. `Frame` is the only other place this raw
                // pointer is used. However each of these own a copy
                // of `Mmap` and do not expose the raw pointer
                // publicly, so any pointers used there will not
                // outlive this struct either.
                let inner = unsafe { MmapInner::new(addr, len) };

                Ok(Mmap {
                    addr,
                    _inner: Arc::new(inner),
                })
            }
        }

        /// Get a pointer to the start of the memory mapped region.
        #[inline]
        pub fn as_mut_ptr(&self) -> *mut libc::c_void {
            self.addr.as_ptr()
        }

        /// Get a pointer to some address within the `mmap`d area,
        /// calculated as an offset from the start of the region.
        ///
        /// # Safety
        ///
        /// The resulting offset pointer must be within the `mmap`d region.
        #[inline]
        pub unsafe fn offset(&self, offset: usize) -> *mut libc::c_void {
            unsafe { self.as_mut_ptr().add(offset as usize) }
        }
    }
}

#[cfg(test)]
mod inner {
    use super::*;

    /// A mocked [`Mmap`] that uses a [`Vec`] internally.
    #[derive(Clone)]
    pub struct Mmap {
        inner: Arc<Vec<u8>>,
    }

    impl Mmap {
        pub fn new(len: usize, _use_huge_pages: bool) -> io::Result<Self> {
            Ok(Self {
                inner: Arc::new(vec![0; len]),
            })
        }

        #[inline]
        pub fn as_mut_ptr(&self) -> *mut libc::c_void {
            self.inner.as_ptr() as *mut libc::c_void
        }

        #[inline]
        pub unsafe fn offset(&self, offset: usize) -> *mut u8 {
            unsafe { self.inner.as_ptr().add(offset) as *mut u8 }
        }

        pub fn len(&self) -> usize {
            self.inner.len()
        }
    }
}

unsafe impl Send for Mmap {}

// SAFETY: this impl is only safe in the context of this library. The
// only mutators of the mmap'd region are the frames, which write to
// disjoint sections (assuming the unsafe requirements are upheld).
unsafe impl Sync for Mmap {}

#[cfg(test)]
mod tests {
    #[test]
    fn confirm_pointer_offset_is_a_single_byte() {
        assert_eq!(std::mem::size_of::<libc::c_void>(), 1);
    }
}
