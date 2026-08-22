//! A wrapper for convenient writing to a [`Umem`](crate::umem::Umem) frame.

use std::io::{self, IoSlice, Write};

use crate::util;

/// Wraps a buffer and a value denoting its current write position and
/// provides a convenient [`Write`] implementation.
///
/// Practically it allows us to write to a [`Umem`](crate::umem::Umem) frame
/// and update its descriptor's length at the same time, avoiding some
/// potentially error prone logic.
///
/// The position is a `u32`, not a `usize`: it is the length of the
/// segment being written - see [`SegmentLengths`] - and a segment can
/// never outgrow the frame holding it, which [`FrameSize`] bounds to
/// a `u32`. That is also the width the kernel reads a packet data
/// length back at, so keeping it here avoids a conversion on every
/// write.
///
/// [`SegmentLengths`]: super::SegmentLengths
/// [`FrameSize`]: crate::config::FrameSize
#[derive(Debug)]
pub struct Cursor<'a> {
    pos: &'a mut u32,
    buf: &'a mut [u8],
}

impl<'a> Cursor<'a> {
    #[inline]
    pub(super) fn new(pos: &'a mut u32, buf: &'a mut [u8]) -> Self {
        Self { pos, buf }
    }

    /// The cursor's current write position in the buffer.
    #[inline]
    pub fn pos(&self) -> u32 {
        *self.pos
    }

    /// Sets the cursor's write position.
    ///
    /// Clamped to the length of the underlying buffer.
    #[inline]
    pub fn set_pos(&mut self, pos: u32) {
        *self.pos = util::min(pos, self.buf_len());
    }

    /// The length of the underlying buffer.
    #[inline]
    pub fn buf_len(&mut self) -> u32 {
        // A buffer is a frame segment, so its length is bounded by
        // the frame size and always fits.
        self.buf.len() as u32
    }

    /// Fills the buffer with zeroes and sets the cursor's write
    /// position to the start of the buffer.
    #[inline]
    pub fn zero_out(&mut self) {
        self.buf.fill(0);
        self.set_pos(0);
    }
}

// Taken almost verbatim from
// [`std::io::Cursor`](https://doc.rust-lang.org/src/std/io/cursor.rs.html#437)
impl Write for Cursor<'_> {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // Kept in `usize` rather than going through `buf_len`, since
        // this indexes the buffer and `Write` counts in `usize` too.
        let pos = util::min(*self.pos as usize, self.buf.len());
        let amt = (&mut self.buf[pos..]).write(buf)?;

        // `amt` is at most the remaining buffer length, so this stays
        // within a frame and cannot overflow.
        *self.pos += amt as u32;

        Ok(amt)
    }

    #[inline]
    fn write_vectored(&mut self, bufs: &[IoSlice<'_>]) -> io::Result<usize> {
        let mut nwritten = 0;
        for buf in bufs {
            let n = self.write(buf)?;
            nwritten += n;
            if n < buf.len() {
                break;
            }
        }
        Ok(nwritten)
    }

    #[inline]
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_works() {
        let mut pos = 0;
        let mut buf = [0; 32];

        {
            let mut cursor = Cursor::new(&mut pos, &mut buf[..]);

            cursor.write_all(b"hello").unwrap();
        }

        assert_eq!(pos, 5);
        assert_eq!(&buf[..pos as usize], b"hello");

        {
            let mut cursor = Cursor::new(&mut pos, &mut buf[..]);

            cursor.write_all(b", world").unwrap();
        }

        assert_eq!(pos, 12);
        assert_eq!(&buf[..pos as usize], b"hello, world");
    }

    #[test]
    fn zero_out_works() {
        let mut pos = 0;
        let mut buf = [0; 32];

        {
            let mut cursor = Cursor::new(&mut pos, &mut buf[..]);

            cursor.write_all(b"hello").unwrap();
        }

        assert_eq!(pos, 5);
        assert_eq!(&buf[..pos as usize], b"hello");

        {
            let mut cursor = Cursor::new(&mut pos, &mut buf[..]);

            cursor.zero_out();
        }

        assert_eq!(pos, 0);
        assert_eq!(&buf, &[0; 32]);
    }

    #[test]
    fn set_pos_cannot_exceed_buf_len() {
        let mut pos = 0;
        let mut buf = [0; 32];

        let mut cursor = Cursor::new(&mut pos, &mut buf[..]);

        cursor.set_pos(1);
        assert_eq!(cursor.pos(), 1);

        cursor.set_pos(32);
        assert_eq!(cursor.pos(), 32);

        cursor.set_pos(33);
        assert_eq!(cursor.pos(), 32);
    }
}
