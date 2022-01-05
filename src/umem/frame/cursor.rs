//! A wrapper for convenient writing to a [`Umem`](crate::umem::Umem) frame.

use std::io::{self, IoSlice, Write};

use crate::util;

/// Wraps a buffer and a value denoting its current write position and
/// provides a convenient [`Write`] implementation.
///
/// Practically it allows us to write to a [`Umem`](crate::umem::Umem) frame
/// and update its descriptor's length at the same time, avoiding some
/// potentially error prone logic.
#[derive(Debug)]
pub struct Cursor<'a> {
    pos: &'a mut usize,
    buf: &'a mut [u8],
}

impl<'a> Cursor<'a> {
    #[inline]
    pub(super) fn new(pos: &'a mut usize, buf: &'a mut [u8]) -> Self {
        Self { pos, buf }
    }

    /// The cursor's current write position in the buffer.
    #[inline]
    pub fn pos(&self) -> usize {
        *self.pos
    }

    /// Sets the cursor's write position.
    #[inline]
    pub fn set_pos(&mut self, pos: usize) {
        *self.pos = util::min_usize(pos, self.buf.len());
    }

    /// The length of the underlying buffer.
    #[inline]
    pub fn buf_len(&mut self) -> usize {
        self.buf.len()
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
        let pos = util::min_usize(*self.pos, self.buf.len());
        let amt = (&mut self.buf[pos..]).write(buf)?;

        *self.pos += amt;

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
        assert_eq!(&buf[..pos], b"hello");

        {
            let mut cursor = Cursor::new(&mut pos, &mut buf[..]);

            cursor.write_all(b", world").unwrap();
        }

        assert_eq!(pos, 12);
        assert_eq!(&buf[..pos], b"hello, world");
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
        assert_eq!(&buf[..pos], b"hello");

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
