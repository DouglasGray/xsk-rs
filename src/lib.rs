//! # xsk-rs
//!
//! A rust interface for AF_XDP sockets using libbpf.
//!
//! For more information please see the [networking docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html)
//! or a more [detailed overview](http://vger.kernel.org/lpc_net2018_talks/lpc18_paper_af_xdp_perf-v2.pdf).

pub mod socket;
pub mod umem;

mod util;

pub use socket::{
    BindFlags, Config as SocketConfig, LibbpfFlags, RxQueue, Socket, TxQueue, XdpFlags,
};
pub use umem::{CompQueue, Config as UmemConfig, FillQueue, FrameDesc, Umem};
