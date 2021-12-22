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

/// A handrolled `min` calc for usizes that appears to be ~20% faster
/// than using [`cmp::min`](std::cmp::min) - though the difference is
/// still only ~50-60 picoseconds when tested on a CPU with max clock
/// speed of 4.9 GHz (see bench sub-crate for code). Decided it would
/// be worth it since the need for `min` appears a fair bit in normal
/// control flow.
#[inline]
pub fn min_usize(fst: usize, snd: usize) -> usize {
    if fst < snd {
        fst
    } else {
        snd
    }
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
