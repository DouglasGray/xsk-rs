pub mod framed;

use libc::{MAP_ANONYMOUS, MAP_FAILED, MAP_HUGETLB, MAP_PRIVATE, PROT_READ, PROT_WRITE};
use log::error;
use std::{
    io,
    ptr::{self, NonNull},
};

/// An anonymous memory mapped region.
#[derive(Clone)]
pub struct Mmap {
    addr: NonNull<libc::c_void>,
    len: usize,
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
            Ok(Mmap {
                len,
                addr: NonNull::new(addr)
                    .expect("ptr non-null since we confirmed `mmap()` succeeded"),
            })
        }
    }

    #[inline]
    pub fn as_mut(&mut self) -> &mut libc::c_void {
        unsafe { self.addr.as_mut() }
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
