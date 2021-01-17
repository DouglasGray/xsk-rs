# Changelog

## [Unreleased]

## [0.2.0] - 2021-01-17
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
