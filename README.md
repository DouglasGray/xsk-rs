# xsk-rs

A Rust interface for Linux AF_XDP sockets using libxdp. 

[API documentation](https://docs.rs/xsk-rs).

For more information please see the [networking docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html)
or a more [detailed overview](http://vger.kernel.org/lpc_net2018_talks/lpc18_paper_af_xdp_perf-v2.pdf).

Initially inspired by Jesse DuMond's [OCaml implementation](https://github.com/suttonshire/ocaml-xsk).

### Examples

A few may be found in the `examples` directory. A simple example of
moving bytes between two sockets via a veth pair can be found in
`examples/hello_xdp.rs`, while a slightly more complex example of
sending and receiving eth frames (also via a veth pair) is in
`examples/dev2_to_dev1.rs`, which includes a single-threaded and
multi-threaded implementation. Note that neither example will be
indicative of actual performance, since binding the sockets to the
veth pair means that packets will pass through the kernel network
stack.

An example with shared UMEM is in `examples/shared_umem.rs`.

### Running tests / examples

Root permissions may be required to run the tests or examples, since 
they require a veth pair to be set up. However to avoid running cargo 
under `root` it's best to first build the tests/examples and run the 
binaries directly.

```
# tests
cargo build --tests
sudo run_all_tests.sh

# examples
cargo build --examples --release
sudo target/release/examples/hello_xdp
sudo target/release/examples/dev1_to_dev2 -- [FLAGS] [OPTIONS]
```

### Compatibility

Tested on a 64-bit machine running Linux kernel version 6.5.0.

### Safety

There is a fair amount of unsafe involved when using this library, and
so the potential for disaster, however if you keep in mind the
following then there should hopefully be few avenues for catastrophe:
- When a frame / address has been submitted to the fill queue or tx
  ring, do not use it again until you have consumed it from either the
  completion queue or rx ring.
- Do not use one UMEM's frame descriptors to access frames of another,
  different UMEM.

### Usage

The below example sends a packet from one interface to another.

```rust
use std::{convert::TryInto, io::Write};
use xsk_rs::{
    config::{SocketConfig, UmemConfig},
    socket::Socket,
    umem::Umem,
};

const ETHERNET_PACKET: [u8; 42] = [
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xf6, 0xe0, 0xf6, 0xc9, 0x60, 0x0a, 0x08, 0x06, 0x00, 0x01,
    0x08, 0x00, 0x06, 0x04, 0x00, 0x01, 0xf6, 0xe0, 0xf6, 0xc9, 0x60, 0x0a, 0xc0, 0xa8, 0x45, 0x01,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xc0, 0xa8, 0x45, 0xfe,
];

fn main() {
    // Create a UMEM for dev1 with 32 frames, whose sizes are
    // specified via the `UmemConfig` instance.
    let (dev1_umem, mut dev1_descs) =
        Umem::new(UmemConfig::default(), 32.try_into().unwrap(), false)
            .expect("failed to create UMEM");

    // Bind an AF_XDP socket to the interface named `xsk_dev1`, on
    // queue 0.
    let (mut dev1_tx_q, _dev1_rx_q, _dev1_fq_and_cq) = unsafe {
        Socket::new(
            SocketConfig::default(),
            &dev1_umem,
            &"xsk_dev1".parse().unwrap(),
            0,
        )
    }
    .expect("failed to create dev1 socket");

    // Create a UMEM for dev2. Another option is to use the same UMEM
    // as dev1 - to do that we'd just pass `dev1_umem` to the
    // `Socket::new` call. In this case the UMEM would be shared, and
    // so `dev1_descs` could be used in either context, but each
    // socket would have its own completion queue and fill queue.
    let (dev2_umem, mut dev2_descs) =
        Umem::new(UmemConfig::default(), 32.try_into().unwrap(), false)
            .expect("failed to create UMEM");

    // Bind an AF_XDP socket to the interface named `xsk_dev2`, on
    // queue 0.
    let (_dev2_tx_q, mut dev2_rx_q, dev2_fq_and_cq) = unsafe {
        Socket::new(
            SocketConfig::default(),
            &dev2_umem,
            &"xsk_dev2".parse().unwrap(),
            0,
        )
    }
    .expect("failed to create dev2 socket");

    let (mut dev2_fq, _dev2_cq) = dev2_fq_and_cq.expect("missing dev2 fill queue and comp queue");

    // 1. Add frames to dev2's fill queue so we are ready to receive
    // some packets.
    unsafe {
        dev2_fq.produce(&dev2_descs);
    }

    // 2. Write to dev1's UMEM.
    let pkt = "Hello, world!".as_bytes();

    unsafe {
        dev1_umem
            .data_mut(&mut dev1_descs[0])
            .cursor()
            .write_all(pkt)
            .expect("failed writing packet to frame")
    }

    // 3. Submit the frame to the kernel for transmission.
    println!("sending packet");

    unsafe {
        dev1_tx_q.produce_and_wakeup(&dev1_descs[..1]).unwrap();
    }

    // 4. Read on dev2.
    let pkts_recvd = unsafe { dev2_rx_q.poll_and_consume(&mut dev2_descs, 100).unwrap() };

    // 5. Confirm that one of the packets we received matches what we expect.
    for recv_desc in dev2_descs.iter().take(pkts_recvd) {
        let data = unsafe { dev2_umem.data(recv_desc) };

        if data.contents() == &ETHERNET_PACKET {
            println!("received packet!");
            return;
        }
    }

    panic!("no matching packets received")
}
```
