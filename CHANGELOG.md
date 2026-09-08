# Changelog

## Unreleased

## Changed

- The `Config` and `ConfigBuilder` structs can now be created and populated in a `const` context. This implies their dependent types (`QueueSize` and `FrameSize`) are also `const` compatible.

## [0.11.0] - 2026-08-28

## Fixed

- `Umem::headroom`, `Umem::headroom_mut`, `Umem::frame` and `Umem::frame_mut`. These now give the frame headroom at the
  start of the frame, which is where the kernel reserves it. Previously they gave the bytes immediately before the
  packet, which belong to the XDP program.
- `Umem::data_mut` and `Umem::frame_mut`. These now allow writes up to the end of the frame rather than the mtu from the
  packet's start, which are the same thing unless an XDP program has moved the packet. A packet moved forward previously
  gave a buffer running past the end of the frame.
- `UmemConfigBuilder::build`. A large frame headroom no longer overflows the check that the two headrooms leave room for
  packet data. It previously panicked in a debug build and wrapped in a release one, where the config was then accepted.

## Changed

- `UmemConfigBuilder::build` now rejects a frame size that is not a power of two, and a frame headroom leaving no room
  for packet data. The kernel refuses both when the UMEM is created, so these configs failed before.

## [0.10.0] - 2026-08-21

## Added

- `TxQueue::nb_free_exact`, `FillQueue::nb_free_exact`, `RxQueue::nb_avail_exact` and `CompQueue::nb_avail_exact`. These
  give the number of free slots on a producer ring and the number of entries waiting on a consumer ring, and are exact
  because they refresh libxdp's cached view of the ring rather than trusting it.
- `TxQueue::nb_free`, `FillQueue::nb_free`, `RxQueue::nb_avail` and `CompQueue::nb_avail`. These give the same counts
  without forcing a refresh, so they may fall short of what the ring really holds.
- `use_cc_build`, a feature forwarding to the `libxdp-sys` feature of the same name. It builds the vendored libxdp with
  the `cc` crate rather than by running its Makefile, so the build writes only to `OUT_DIR` and is configured up front
  instead of by whatever `configure` finds on the host

## Fixed

- the docs.rs build, which now documents the crate with `use_cc_build` enabled. docs.rs mounts the crate source
  directory read only, where the Makefile build's `configure` step cannot run

## [0.9.0] - 2026-08-15

## Added

- new constants for enabling multi-buffer

## Fixed

- keep the rx, tx, fill and comp rings alive and in place for as long as libxdp needs them. libxdp retains the ring
  pointers it is passed and dereferences them again during teardown to determine which memory to unmap, so moving or
  freeing a ring beforehand caused
  `munmap` to be called with a garbage address. The socket's memory was then never released and subsequent binds to the
  same device and queue id failed with `EBUSY`. The fill and comp pair is saved on the UMEM and on the context shared by
  every socket bound to the same device and queue id, so it has to outlive sockets it was never handed to
- a `FillQueue` or `CompQueue` now keeps its socket alive rather than just the UMEM. One that outlived every other
  handle to its socket was left reading and writing through an unmapped ring
- deleting a socket now takes the same lock creating one does, so sockets on a shared `Umem` can be created and dropped
  from more than one thread. libxdp's per-UMEM refcounts and context list have no synchronisation of their own, and a
  lost update to either unmaps a context's rings while a socket is still on it, or leaves them mapped for good
- `SocketCreateError` and `UmemCreateError` no longer answer `source()`
  with an `io::Error` reading `Success (os error 0)` on the paths that have no OS error behind them. They report `None`
  there instead
- use `wrapping_add` for queue `idxs`, to align with underlying C implementation

## Changed

- dropping a `TxQueue` and `RxQueue` no longer deletes the socket if the `FillQueue` or `CompQueue` returned alongside
  them are still alive. All four now have to go before the device is released
- dropping the last of a socket's queues can block, its deletion being serialised against creating or deleting a socket
  on the same `Umem`

## [0.8.0] - 2025-09-17

## Changed

- bumped `libxdp-sys` to version `0.2.2`, which corresponds to `libxdp` version `1.5.5`
- use rust edition `2024`

## [0.7.0] - 2025-04-11

## Fixed

- add missing lifetime to `umem::frame::Data::contents`
- in the `dev1_to_dev2` example, use the sender completion queue size to calculate sender frame count

## Changed

- bump dependencies

## [0.6.1] - 2024-05-19

## Changed

- updated example in readme

## [0.6.0] - 2024-05-19

## Changed

- use `libxdp-sys` instead of `libbpf-sys`

## [0.5.0] - 2022-10-18

## Changed

- bump `libbpf-sys` version

## [0.4.1] - 2022-03-10

## Added

- provide `FrameDesc` with a `Default` impl to make generating empty descs for rx simpler

## Fixed

- negate error codes when calling `io::Error::from_raw_os_error`
- some `libc` calls just return `-1` on error, not an informative error code so in these cases call
  `io::Error::last_os_error()`
  instead of `io::Error::from_raw_os_error(err)`, where `err` is always equal to `-1`...

## [0.4.0] - 2022-02-09

## Added

- add `contents_mut` to `{Data, Headroom}Mut`, along with other convenience traits (`{As, Borrow, Deref}{Mut}`)

## Changed

- update `{Data, Headroom}Mut::cursor` docs to clarify when `{Data,
  Headroom}Mut::contents_mut` might be more appropriate
- more colour to safety section of `Umem::frame` and `Umem::frame_mut`
  indicating why using the frame desc of another UMEM might be problematic

## [0.3.0] - 2022-01-17

## Added

- support shared UMEM
- support retrieving XDP statistics
- new frame level structs to allow more granular UMEM access along with clearer separation between headroom and packet
  data. Includes a cursor for convenient writing
- config builders and add extra types to enforce restrictions on certain values / sizes (e.g queue sizes)

## Changed

- bump libs, e.g. `libbpf-sys` to 0.6.0-1

## Removed

- got rid of lifetimes by packaging the various queues with an `Arc`'d UMEM or socket where needed to ensure they don't
  outlive what they depend on. Shouldn't cause any slowdown in the single threaded case since the `Arc`s aren't
  dereferenced in the fast path

## [0.2.4] - 2021-07-10

## Changes

- expose the socket file descriptor on the `Fd` struct to make it possible to register the socket manually
- bump libbpf-sys to version 0.4

## [0.2.3] - 2021-06-09

## Changed

- added CI, fixed docs

## [0.2.2] - 2020-05-25

## Changed

- bumped lib versions, libbpf-sys specifically

## [0.2.1] - 2020-01-29

### Changed

- bumped libbpf-sys version to 0.3
- fixed docs, wasn't showing some stuff since the structs/enums weren't exposed

## [0.2.0] - 2021-01-17

Breaking change

### Changed

- Changed the APIs for the UMEM and socket to be `unsafe` where required. It's possible in a number of locations to get
  into a race with the kernel for a bit of shared memory, so tried to make those areas clearer.
- Can now set the `addr` on `FrameDesc` manually, previously had to go through the library.
- Cleared up examples and hopefully made them a bit more illustrative.

### Added

- A `bench` sub-project, work on which is ongoing.
