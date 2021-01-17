use libc::{MAP_ANONYMOUS, MAP_FAILED, MAP_HUGETLB, MAP_PRIVATE, PROT_READ, PROT_WRITE};
use log::error;
use std::{convert::TryInto, io, ptr, slice};

pub struct MmapArea {
    len: usize,
    mem_ptr: *mut libc::c_void,
}

unsafe impl Send for MmapArea {}

impl MmapArea {
    pub fn new(len: usize, use_huge_pages: bool) -> io::Result<Self> {
        let addr = ptr::null_mut();
        let prot = PROT_READ | PROT_WRITE;
        let file = -1;
        let offset = 0;

        let mut flags = MAP_ANONYMOUS | MAP_PRIVATE;

        if use_huge_pages {
            flags |= MAP_HUGETLB;
        }

        let mem_ptr = unsafe { libc::mmap(addr, len, prot, flags, file, offset as libc::off_t) };

        if mem_ptr == MAP_FAILED {
            Err(io::Error::last_os_error())
        } else {
            Ok(MmapArea { len, mem_ptr })
        }
    }

    pub(in crate::umem) fn as_mut_ptr(&mut self) -> *mut libc::c_void {
        self.mem_ptr
    }

    /// Return a reference to memory at `offset` of length `len`. Does
    /// not perform a bounds check.
    ///
    /// Marked `unsafe` as there is no guarantee that the kernel isn't
    /// currently writing to or reading from the region (since it's
    /// backing the UMEM).
    #[inline]
    pub unsafe fn mem_range(&self, offset: usize, len: usize) -> &[u8] {
        let ptr = self.mem_ptr.offset(offset.try_into().unwrap());

        slice::from_raw_parts(ptr as *const u8, len)
    }

    /// Return a mutable reference to memory at `offset` of length
    /// `len`. Does not perform a bounds check.
    ///
    /// Marked `unsafe` as there is no guarantee that the kernel isn't
    /// currently writing to or reading from the region (since it's
    /// backing the UMEM).
    #[inline]
    pub unsafe fn mem_range_mut(&mut self, offset: &usize, len: &usize) -> &mut [u8] {
        let ptr = self.mem_ptr.offset((*offset).try_into().unwrap());

        slice::from_raw_parts_mut(ptr as *mut u8, *len)
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }
}

impl Drop for MmapArea {
    fn drop(&mut self) {
        let err = unsafe { libc::munmap(self.mem_ptr, self.len) };

        if err != 0 {
            error!("munmap() failed: {}", err);
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
