# xsk-rs

A Rust interface for Linux AF_XDP sockets using libbpf. 

[API documentation](https://docs.rs/xsk-rs).

For more information please see the [networking docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html)
or a more [detailed overview](http://vger.kernel.org/lpc_net2018_talks/lpc18_paper_af_xdp_perf-v2.pdf).

Inspired by Jesse DuMond's [OCaml implementation](https://github.com/suttonshire/ocaml-xsk).

### Examples

A couple can be found in the `examples` directory. A simple example of
moving bytes between two sockets via a veth pair can be found in
`hello_xdp.rs`, while a slightly more complex example of sending and
receiving eth frames (also via a veth pair) is in `dev2_to_dev1.rs`,
which includes a single-threaded and multi-threaded
implementation. Note that neither example will be indicative of actual
performance, since binding the sockets to the veth pair means that
packets will pass through the kernel network stack.

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

Tested on a 64-bit machine running Linux kernel version 5.14.0.

### Usage

The below example sends a packet from one interface to another. It
uses a shared UMEM for brevity.

```rust
use std::{convert::TryInto, io::Write, str};
use xsk_rs::{
    config::{SocketConfig, UmemConfig},
    socket::Socket,
    umem::Umem,
};

fn main() {
    let (umem, mut frames) = Umem::new(UmemConfig::default(), 32.try_into().unwrap(), false)
        .expect("failed to create UMEM");

    let (dev1_frames, dev2_frames) = frames.split_at_mut(16);

    let (mut dev1_tx_q, _dev1_rx_q, _dev1_fq_and_cq) = Socket::new(
        SocketConfig::default(),
        &umem,
        &"xsk_dev1".parse().unwrap(),
        0,
    )
    .expect("failed to create dev1 socket");

    let (_dev2_tx_q, mut dev2_rx_q, dev2_fq_and_cq) = Socket::new(
        SocketConfig::default(),
        &umem,
        &"xsk_dev2".parse().unwrap(),
        0,
    )
    .expect("failed to create dev2 socket");

    let (mut dev2_fq, _dev2_cq) = dev2_fq_and_cq.expect("missing dev2 fill queue and comp queue");

    // 1. Add frames to dev2's fill queue
    unsafe {
        dev2_fq.produce(&dev2_frames);
    }

    // 2. Write data to dev1's UMEM
    let pkt = "Hello, world!".as_bytes();

    unsafe {
        dev1_frames[0]
            .data_mut()
            .cursor()
            .write_all(pkt)
            .expect("failed writing packet to frame")
    }

    // 3. Hand over dev1's frame to the kernel for transmission
	println!("sending: {:?}", str::from_utf8(&pkt).unwrap());
	
    unsafe {
        dev1_tx_q.produce_and_wakeup(&dev1_frames[..1]).unwrap();
    }

    // 4. Read from dev2
    let pkts_recvd = unsafe { dev2_rx_q.poll_and_consume(dev2_frames, 100).unwrap() };

    // 5. Confirm that one of the packets we received matches what we expect
    for recv_frame in dev2_frames.iter().take(pkts_recvd) {
        let data = unsafe { recv_frame.data() };

        println!("received: {:?}", str::from_utf8(data.contents()).unwrap());

        if data.contents() == &pkt[..] {
            return;
        }
    }

    panic!("no matching packets received")
}
```
