# xsk-rs

A Rust interface for Linux AF_XDP sockets using libbpf. 

[API documentation](https://docs.rs/xsk-rs).

For more information please see the [networking docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html)
or a more [detailed overview](http://vger.kernel.org/lpc_net2018_talks/lpc18_paper_af_xdp_perf-v2.pdf).

Inspired by Jesse DuMond's [OCaml implementation](https://github.com/suttonshire/ocaml-xsk).

### Examples

A couple can be found in the `examples` directory. A simple example of moving bytes between a veth pair can be found 
in `hello_xdp.rs`, while a slightly more complex example of sending and receiving eth frames is in 
`dev2_to_dev1.rs`, which includes a single-threaded and multi-threaded implementation.

### Running tests / examples

It may be that root permissions are required to run the tests or examples, since they require a veth pair to be set up. 
If that's the case try:

```
# tests
sudo env PATH=$PATH cargo test

# examples
sudo env PATH=$PATH cargo run --example hello_xdp
sudo env PATH=$PATH cargo run --example dev2_to_dev1 -- [FLAGS] [OPTIONS]
```

### Compatibility

Tested on a 64-bit machine running Linux kernel version 5.7.1.
