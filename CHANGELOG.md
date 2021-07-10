# Changelog

## [Unreleased]

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
