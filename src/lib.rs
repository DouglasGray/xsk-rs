//! # xsk-rs
//!
//! A rust interface for AF_XDP sockets using libbpf.
//!
//! For more information please see the [networking
//! docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html)
//! or a more [detailed
//! overview](http://vger.kernel.org/lpc_net2018_talks/lpc18_paper_af_xdp_perf-v2.pdf).
//!
//! Some simple examples may be found in the `examples` directory in
//! the GitHub repo.

use cfg_if::cfg_if;

cfg_if! {
    if #[cfg(all(target_pointer_width = "64", target_family = "unix"))] {
        #[warn(unsafe_op_in_unsafe_fn)]
        pub mod umem;
        #[warn(unsafe_op_in_unsafe_fn)]
        pub mod socket;
        pub mod config;
        pub mod prelude;

        mod ring;
        mod util;

        #[cfg(test)]
        mod tests {
            use std::mem;

            #[test]
            fn ensure_usize_and_u64_are_same_size() {
                assert_eq!(mem::size_of::<usize>(), mem::size_of::<u64>());
            }
        }
    }
}
