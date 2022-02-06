# Changelog

## [0.4.0] - 2022-02-06

## Added
- add `contents_mut` to `{Data, Headroom}Mut`, along with other
  convenience traits (`{As, Borrow, Deref}{Mut}`)

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
