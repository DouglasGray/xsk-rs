use libc::{MAP_ANONYMOUS, MAP_FAILED, MAP_HUGETLB, MAP_PRIVATE, PROT_READ, PROT_WRITE};
use std::{io, ptr};

pub struct MmapArea {
    pub len: usize,
    mem_ptr: *mut libc::c_void,
}

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

    pub fn as_mut_ptr(&mut self) -> *mut libc::c_void {
        self.mem_ptr
    }
}

impl Drop for MmapArea {
    fn drop(&mut self) {
        let err = unsafe { libc::munmap(self.mem_ptr, self.len) };

        if err != 0 {
            eprintln!("munmap failed: {}", err);
        }
    }
}
