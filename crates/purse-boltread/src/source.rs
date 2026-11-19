// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! Positional reads over a file or an in-memory buffer.

use std::io;
use std::sync::Arc;

/// Random-access bytes the reader reads pages from.
///
/// The reader takes [`Source::size`] once, when it opens, and never reads
/// past it. Implemented for [`std::fs::File`] (Unix and Windows) and for
/// in-memory buffers, so tests and fuzzers can feed bytes directly.
pub trait Source: Send + Sync {
    /// The length of the data in bytes.
    fn size(&self) -> io::Result<u64>;

    /// Fills `buf` with the bytes at `offset`, or fails (with
    /// [`io::ErrorKind::UnexpectedEof`] when the range runs past the end).
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()>;
}

fn read_slice(data: &[u8], buf: &mut [u8], offset: u64) -> io::Result<()> {
    let src = usize::try_from(offset)
        .ok()
        .and_then(|start| data.get(start..start.checked_add(buf.len())?))
        .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "read past end of buffer"))?;
    buf.copy_from_slice(src);
    Ok(())
}

impl Source for Vec<u8> {
    fn size(&self) -> io::Result<u64> {
        Ok(self.len() as u64)
    }
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        read_slice(self, buf, offset)
    }
}

impl Source for Box<[u8]> {
    fn size(&self) -> io::Result<u64> {
        Ok(self.len() as u64)
    }
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        read_slice(self, buf, offset)
    }
}

impl Source for Arc<[u8]> {
    fn size(&self) -> io::Result<u64> {
        Ok(self.len() as u64)
    }
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        read_slice(self, buf, offset)
    }
}

impl Source for &'static [u8] {
    fn size(&self) -> io::Result<u64> {
        Ok(self.len() as u64)
    }
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        read_slice(self, buf, offset)
    }
}

#[cfg(unix)]
impl Source for std::fs::File {
    fn size(&self) -> io::Result<u64> {
        Ok(self.metadata()?.len())
    }
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        std::os::unix::fs::FileExt::read_exact_at(self, buf, offset)
    }
}

#[cfg(windows)]
impl Source for std::fs::File {
    fn size(&self) -> io::Result<u64> {
        Ok(self.metadata()?.len())
    }
    fn read_exact_at(&self, mut buf: &mut [u8], mut offset: u64) -> io::Result<()> {
        use std::os::windows::fs::FileExt;
        while !buf.is_empty() {
            match self.seek_read(buf, offset) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "read past end of file",
                    ));
                }
                Ok(n) => {
                    let rest = std::mem::take(&mut buf);
                    buf = rest.get_mut(n..).unwrap_or_default();
                    offset = offset.saturating_add(n as u64);
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}
