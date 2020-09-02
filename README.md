# xsk-rs

A Rust interface for Linux AF_XDP sockets using libbpf. 

For more information please see the [networking docs](https://www.kernel.org/doc/html/latest/networking/af_xdp.html)
or a more [detailed overview](http://vger.kernel.org/lpc_net2018_talks/lpc18_paper_af_xdp_perf-v2.pdf).

Inspired by Jesse DuMond's [OCaml implementation](https://github.com/suttonshire/ocaml-xsk).

### Running tests / examples

It may be that root permissions are required to run the tests or examples. If that's the case try:

```
# tests
sudo env PATH=$PATH cargo test

# examples
sudo env PATH=$PATH cargo run --example hello_xdp
```

### Compatibility

Tested on a 64-bit machine running Linux kernel version 5.7.1.
