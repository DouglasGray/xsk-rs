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
sudo ./target/release/examples/hello_xdp
sudo ./target/release/examples/dev2_to_dev1 -- [FLAGS] [OPTIONS]
```

### Compatibility

Tested on a 64-bit machine running Linux kernel version 5.7.1.
