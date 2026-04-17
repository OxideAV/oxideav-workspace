//! Shared helpers for oxideav-gif integration tests.

use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::{Arc, Mutex};

/// A `WriteSeek` sink that stashes bytes in an `Arc<Mutex<Vec<u8>>>` so
/// callers can retrieve the bytes after the muxer has dropped its
/// reference. Write + Seek + Read trait-bundle is satisfied so the sink
/// can slot into `Box<dyn WriteSeek>`.
pub struct SharedSink {
    pub inner: Arc<Mutex<Vec<u8>>>,
    pub pos: u64,
}

impl SharedSink {
    pub fn new() -> (Self, Arc<Mutex<Vec<u8>>>) {
        let inner = Arc::new(Mutex::new(Vec::<u8>::new()));
        (
            Self {
                inner: Arc::clone(&inner),
                pos: 0,
            },
            inner,
        )
    }
}

impl Write for SharedSink {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        let mut guard = self.inner.lock().unwrap();
        let start = self.pos as usize;
        if start + data.len() > guard.len() {
            guard.resize(start + data.len(), 0);
        }
        guard[start..start + data.len()].copy_from_slice(data);
        self.pos += data.len() as u64;
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Read for SharedSink {
    fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
        Err(std::io::Error::other("SharedSink: read unsupported"))
    }
}

impl Seek for SharedSink {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let guard = self.inner.lock().unwrap();
        let len = guard.len() as u64;
        let new_pos = match pos {
            SeekFrom::Start(n) => n,
            SeekFrom::End(n) => (len as i64 + n).max(0) as u64,
            SeekFrom::Current(n) => (self.pos as i64 + n).max(0) as u64,
        };
        self.pos = new_pos;
        Ok(self.pos)
    }
}
