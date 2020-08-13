use libc::{MAP_ANONYMOUS, MAP_FAILED, MAP_HUGETLB, MAP_PRIVATE, PROT_READ, PROT_WRITE};
use log::error;
use std::{convert::TryInto, io, ptr, slice};

#[derive(Debug)]
pub enum MmapAccessError {
    OffsetOutOfBounds,
    MemRangeOutOfBounds,
}

pub struct MmapArea {
    len: u64,
    mem_ptr: *mut libc::c_void,
    len_usize: usize,
}

unsafe impl Send for MmapArea {}

impl MmapArea {
    pub fn new(len: u64, use_huge_pages: bool) -> io::Result<Self> {
        let addr = ptr::null_mut();
        let prot = PROT_READ | PROT_WRITE;
        let file = -1;
        let offset = 0;

        let mut flags = MAP_ANONYMOUS | MAP_PRIVATE;

        if use_huge_pages {
            flags |= MAP_HUGETLB;
        }

        // Assuming 64-bit architecure to u64 -> usize should work
        let len_usize: usize = len.try_into().unwrap();

        let mem_ptr =
            unsafe { libc::mmap(addr, len_usize, prot, flags, file, offset as libc::off_t) };

        if mem_ptr == MAP_FAILED {
            Err(io::Error::last_os_error())
        } else {
            Ok(MmapArea {
                len,
                mem_ptr,
                len_usize,
            })
        }
    }

    pub fn as_mut_ptr(&mut self) -> *mut libc::c_void {
        self.mem_ptr
    }

    pub fn as_ptr(&self) -> *const libc::c_void {
        self.mem_ptr
    }

    pub fn mem_range(&self, offset: usize, len: usize) -> Result<&[u8], MmapAccessError> {
        self.check_bounds(offset, len)?;

        unsafe {
            let ptr = self.mem_ptr.offset(offset.try_into().unwrap());

            Ok(slice::from_raw_parts(ptr as *const u8, len))
        }
    }

    pub fn mem_range_mut(
        &mut self,
        offset: usize,
        len: usize,
    ) -> Result<&mut [u8], MmapAccessError> {
        self.check_bounds(offset, len)?;

        unsafe {
            let ptr = self.mem_ptr.offset(offset.try_into().unwrap());

            Ok(slice::from_raw_parts_mut(ptr as *mut u8, len))
        }
    }

    fn check_bounds(&self, offset: usize, len: usize) -> Result<(), MmapAccessError> {
        if offset >= self.len_usize {
            return Err(MmapAccessError::OffsetOutOfBounds);
        } else if offset + len > self.len_usize {
            return Err(MmapAccessError::MemRangeOutOfBounds);
        }
        Ok(())
    }

    pub fn len(&self) -> u64 {
        self.len
    }
}

impl Drop for MmapArea {
    fn drop(&mut self) {
        let err = unsafe { libc::munmap(self.mem_ptr, self.len_usize) };

        if err != 0 {
            error!("munmap failed: {}", err);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn confirm_pointer_offset_is_a_single_byte() {
        assert_eq!(std::mem::size_of::<libc::c_void>(), 1);
    }

    #[test]
    fn confirm_clone_not_possible() {
        let mut mmap = MmapArea::new(128, false).unwrap();
    }
}
