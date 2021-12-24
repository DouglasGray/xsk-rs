use std::{io, sync::Arc};

pub use inner::*;

#[cfg(not(test))]
mod inner {
    use super::*;

    use libc::{MAP_ANONYMOUS, MAP_FAILED, MAP_HUGETLB, MAP_PRIVATE, PROT_READ, PROT_WRITE};
    use log::error;
    use std::ptr::{self, NonNull};

    struct MmapInner {
        addr: NonNull<libc::c_void>,
        len: usize,
    }

    impl MmapInner {
        fn new(addr: NonNull<libc::c_void>, len: usize) -> Self {
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
        inner: Arc<MmapInner>,
    }

    impl Mmap {
        pub fn new(len: usize, use_huge_pages: bool) -> io::Result<Self> {
            let prot = PROT_READ | PROT_WRITE;
            let file = -1;
            let offset = 0;

            let mut flags = MAP_ANONYMOUS | MAP_PRIVATE;

            if use_huge_pages {
                flags |= MAP_HUGETLB;
            }

            let addr = unsafe {
                libc::mmap(
                    ptr::null_mut(),
                    len,
                    prot,
                    flags,
                    file,
                    offset as libc::off_t,
                )
            };

            if addr == MAP_FAILED {
                Err(io::Error::last_os_error())
            } else {
                let inner = MmapInner::new(
                    NonNull::new(addr).expect("ptr non-null since we confirmed `mmap()` succeeded"),
                    len,
                );

                Ok(Mmap {
                    inner: Arc::new(inner),
                })
            }
        }

        #[inline]
        pub fn as_mut_ptr(&self) -> *mut libc::c_void {
            self.inner.addr.as_ptr()
        }

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

// Safety: this impl is only safe in the context of this library. The
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
