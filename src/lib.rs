pub mod poll;
pub mod socket;
pub mod umem;

pub(crate) fn get_errno() -> i32 {
    unsafe { *libc::__errno_location() }
}
