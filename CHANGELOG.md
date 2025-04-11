# Changelog

## Unreleased

## [0.7.0] - 2025-04-11

## Fixed
- add missing lifetime to `umem::frame::Data::contents`
- in the `dev1_to_dev2` example, use the sender completion queue size
  to calculate sender frame count

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
- provide `FrameDesc` with a `Default` impl to make generating empty
  descs for rx simpler
  
## Fixed
- negate error codes when calling `io::Error::from_raw_os_error`
- some `libc` calls just return `-1` on error, not an informative
  error code so in these cases call `io::Error::last_os_error()`
  instead of `io::Error::from_raw_os_error(err)`, where `err` is
  always equal to `-1`...

## [0.4.0] - 2022-02-09

## Added
- add `contents_mut` to `{Data, Headroom}Mut`, along with other
  convenience traits (`{As, Borrow, Deref}{Mut}`)

## Changed
- update `{Data, Headroom}Mut::cursor` docs to clarify when `{Data,
  Headroom}Mut::contents_mut` might be more appropriate
- more colour to safety section of `Umem::frame` and `Umem::frame_mut`
  indicating why using the frame desc of another UMEM might be
  problematic

## [0.3.0] - 2022-01-17

## Added
- support shared UMEM
- support retrieving XDP statistics
- new frame level structs to allow more granular UMEM access along
  with clearer separation between headroom and packet data. Includes a
  cursor for convenient writing
- config builders and add extra types to enforce restrictions on
  certain values / sizes (e.g queue sizes)

## Changed
- bump libs, e.g. `libbpf-sys` to 0.6.0-1

## Removed
- got rid of lifetimes by packaging the various queues with an `Arc`'d
  UMEM or socket where needed to ensure they don't outlive what they
  depend on. Shouldn't cause any slowdown in the single threaded case
  since the `Arc`s aren't dereferenced in the fast path

## [0.2.4] - 2021-07-10

## Changes
- expose the socket file descriptor on the `Fd` struct to make it
  possible to register the socket manually
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
- fixed docs, wasn't showing some stuff since the structs/enums
  weren't exposed

## [0.2.0] - 2021-01-17
Breaking change

### Changed
- Changed the APIs for the UMEM and socket to be `unsafe` where
  required. It's possible in a number of locations to get into a race
  with the kernel for a bit of shared memory, so tried to make those
  areas clearer.
- Can now set the `addr` on `FrameDesc` manually, previously had to go
  through the library.
- Cleared up examples and hopefully made them a bit more illustrative.

### Added
- A `bench` sub-project, work on which is ongoing.
