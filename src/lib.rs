//! # xsk-rs
//!
//! A rust interface for AF_XDP sockets using libbpf.
//!
//! For more information please see the [networking
//! docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html)
//! or a more [detailed
//! overview](http://vger.kernel.org/lpc_net2018_talks/lpc18_paper_af_xdp_perf-v2.pdf).
//!
//! An overview of XDP [setup
//! dependencies](https://github.com/xdp-project/xdp-tutorial/blob/main/setup_dependencies.org)
//! may also come in handy.
//!
//! Some simple examples may be found in the [GitHub
//! repo](https://github.com/DouglasGray/xsk-rs/tree/master/examples),
//! including an example of use in a multithreaded context and another
//! creating a socket with a shared [`Umem`].
//!
//! ### Safety
//!
//! There is a fair amount of unsafe involved when using this library, and
//! so the potential for disaster, however if you keep in mind the
//! following then there should hopefully be few avenues for catastrophe:
//! - When a frame / address has been submitted to the [`FillQueue`]
//!   or [`TxQueue`], do not use it again until you have consumed it
//!   from either the [`CompQueue`] or [`RxQueue`].
//! - Do not use one [`Umem`]'s frame descriptors to access frames of
//!   another, different [`Umem`]. For example, via [`Umem::frame`].
//!
//! ### Usage
//!
//! The below example sends a packet from one interface to another.
//!
//! ```no_run
//! use std::{convert::TryInto, io::Write, str};
//! use xsk_rs::{
//!     config::{SocketConfig, UmemConfig},
//!     socket::Socket,
//!     umem::Umem,
//! };
//!
//! // Create a UMEM for dev1 with 32 frames, whose sizes are
//! // specified via the `UmemConfig` instance.
//! let (dev1_umem, mut dev1_descs) =
//!     Umem::new(UmemConfig::default(), 32.try_into().unwrap(), false)
//!         .expect("failed to create UMEM");
//!
//! // Bind an AF_XDP socket to the interface named `xsk_dev1`, on
//! // queue 0.
//! let (mut dev1_tx_q, _dev1_rx_q, _dev1_fq_and_cq) = Socket::new(
//!     SocketConfig::default(),
//!     &dev1_umem,
//!     &"xsk_dev1".parse().unwrap(),
//!     0,
//! )
//! .expect("failed to create dev1 socket");
//!
//! // Create a UMEM for dev2. Another option is to use the same UMEM
//! // as dev1 - to do that we'd just pass `dev1_umem` to the
//! // `Socket::new` call. In this case the UMEM would be shared, and
//! // so `dev1_descs` could be used in either context, but each
//! // socket would have its own completion queue and fill queue.
//! let (dev2_umem, mut dev2_descs) =
//!     Umem::new(UmemConfig::default(), 32.try_into().unwrap(), false)
//!         .expect("failed to create UMEM");
//!
//! // Bind an AF_XDP socket to the interface named `xsk_dev2`, on
//! // queue 0.
//! let (_dev2_tx_q, mut dev2_rx_q, dev2_fq_and_cq) = Socket::new(
//!     SocketConfig::default(),
//!     &dev2_umem,
//!     &"xsk_dev2".parse().unwrap(),
//!     0,
//! )
//! .expect("failed to create dev2 socket");
//!
//! let (mut dev2_fq, _dev2_cq) = dev2_fq_and_cq.expect("missing dev2 fill queue and comp queue");
//!
//! // 1. Add frames to dev2's fill queue so we are ready to receive
//! // some packets.
//! unsafe {
//!     dev2_fq.produce(&dev2_descs);
//! }
//!
//! // 2. Write to dev1's UMEM.
//! let pkt = "Hello, world!".as_bytes();
//!
//! unsafe {
//!     dev1_umem
//!         .data_mut(&mut dev1_descs[0])
//!         .cursor()
//!         .write_all(pkt)
//!         .expect("failed writing packet to frame")
//! }
//!
//! // 3. Submit the frame to the kernel for transmission.
//! println!("sending: {:?}", str::from_utf8(&pkt).unwrap());
//!
//! unsafe {
//!     dev1_tx_q.produce_and_wakeup(&dev1_descs[..1]).unwrap();
//! }
//!
//! // 4. Read on dev2.
//! let pkts_recvd = unsafe { dev2_rx_q.poll_and_consume(&mut dev2_descs, 100).unwrap() };
//!
//! // 5. Confirm that one of the packets we received matches what we expect.
//! for recv_desc in dev2_descs.iter().take(pkts_recvd) {
//!     let data = unsafe { dev2_umem.data(recv_desc) };
//!
//!     if data.contents() == &pkt[..] {
//!         println!("received: {:?}", str::from_utf8(data.contents()).unwrap());
//!         return;
//!     }
//! }
//!
//! panic!("no matching packets received")
//! ```
#![deny(missing_docs)]
#![deny(missing_debug_implementations)]
#![deny(unsafe_op_in_unsafe_fn)]
#![allow(clippy::doc_lazy_continuation)]

use cfg_if::cfg_if;

cfg_if! {
    if #[cfg(all(target_pointer_width = "64", target_family = "unix"))] {
        pub mod umem;
        pub use umem::{frame::FrameDesc, CompQueue, FillQueue, Umem};

        pub mod socket;
        pub use socket::{RxQueue, Socket, TxQueue};

        pub mod config;

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
