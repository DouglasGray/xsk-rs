//! # xsk-rs
//!
//! A rust interface for AF_XDP sockets using libbpf.
//!
//! For more information please see the [networking docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html)
//! or a more [detailed overview](http://vger.kernel.org/lpc_net2018_talks/lpc18_paper_af_xdp_perf-v2.pdf).
//!
//! Some simple examples may be found in the `examples` directory in the GitHub repo.

#[cfg(all(target_pointer_width = "64", target_family = "unix"))]
pub mod socket;

#[cfg(all(target_pointer_width = "64", target_family = "unix"))]
pub mod umem;

#[cfg(all(target_pointer_width = "64", target_family = "unix"))]
mod util;

#[cfg(all(target_pointer_width = "64", target_family = "unix"))]
pub use socket::{
    BindFlags, Config as SocketConfig, LibbpfFlags, RxQueue, Socket, TxQueue, XdpFlags,
};

#[cfg(all(target_pointer_width = "64", target_family = "unix"))]
pub use umem::{
    AddrError, CompQueue, Config as UmemConfig, DataError, FillQueue, FrameDesc, Umem, WriteError,
};

#[cfg(test)]
mod tests {
    use std::mem;

    #[test]
    fn ensure_usize_and_u64_are_same_size() {
        assert_eq!(mem::size_of::<usize>(), mem::size_of::<u64>());
    }
}
