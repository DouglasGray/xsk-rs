#[inline]
pub fn get_errno() -> i32 {
    unsafe { *libc::__errno_location() }
}

#[inline]
pub fn is_pow_of_two(val: u32) -> bool {
    if val == 0 {
        return false;
    }
    (val & (val - 1)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_powers_of_two() {
        assert_eq!(is_pow_of_two(0), false);
        assert_eq!(is_pow_of_two(1), true);
        assert_eq!(is_pow_of_two(2), true);
        assert_eq!(is_pow_of_two(13), false);
    }
}
