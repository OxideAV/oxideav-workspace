//! MP3 "bit reservoir" — a rolling buffer of main-data bytes.
//!
//! Each MP3 frame's main-data does not have to fit inside the same frame's
//! on-disk payload. A frame's `main_data_begin` field (in the side info) is
//! the number of bytes BEFORE the end of the previous frame's side-info
//! block that the current frame's main-data starts at. The main-data runs
//! for whatever part2_3_length sums demand, and may overrun into the
//! *next* frame's storage slot.
//!
//! The reservoir keeps the last ~N bytes of main-data so we can satisfy
//! look-backs. ISO/IEC 11172-3 caps the look-back at 511 bytes for MPEG-1
//! and 255 bytes for MPEG-2 — we use 512 bytes as a generous upper bound.

const MAX_RESERVOIR: usize = 4096;
const MAX_LOOKBACK: usize = 511;

/// Rolling buffer of the main-data that previous frames have contributed
/// (and the current frame in progress).
pub struct Reservoir {
    buf: Vec<u8>,
}

impl Default for Reservoir {
    fn default() -> Self {
        Self::new()
    }
}

impl Reservoir {
    pub fn new() -> Self {
        Self {
            buf: Vec::with_capacity(MAX_RESERVOIR),
        }
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn clear(&mut self) {
        self.buf.clear();
    }

    /// Append `main_data` bytes (everything after the side-info of a frame).
    /// Trims the head so the buffer never grows unboundedly.
    pub fn append(&mut self, main_data: &[u8]) {
        self.buf.extend_from_slice(main_data);
        if self.buf.len() > MAX_RESERVOIR {
            let drop = self.buf.len() - MAX_RESERVOIR;
            self.buf.drain(..drop);
        }
    }

    /// After `main_data_begin` tells us how far back (in bytes) the current
    /// frame's main-data starts, produce a contiguous slice representing
    /// this frame's main data as it flows into the reservoir.
    ///
    /// Returns `None` if insufficient lookback is available (first frames
    /// of a stream often have large main_data_begin that we can't honour —
    /// in practice MP3 encoders warm up with several silent/empty frames).
    pub fn view_from_lookback(&self, main_data_begin: u16) -> Option<&[u8]> {
        let mdb = main_data_begin as usize;
        if mdb > self.buf.len() {
            return None;
        }
        if mdb > MAX_LOOKBACK {
            return None;
        }
        let start = self.buf.len() - mdb;
        Some(&self.buf[start..])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_and_view() {
        let mut r = Reservoir::new();
        r.append(&[1, 2, 3, 4, 5]);
        // lookback 3 -> expect [3, 4, 5]
        assert_eq!(r.view_from_lookback(3).unwrap(), &[3, 4, 5]);
        // lookback 5 -> whole buffer
        assert_eq!(r.view_from_lookback(5).unwrap(), &[1, 2, 3, 4, 5]);
        // lookback > len -> None
        assert!(r.view_from_lookback(6).is_none());
    }

    #[test]
    fn trims_to_max_size() {
        let mut r = Reservoir::new();
        let blob = vec![0u8; MAX_RESERVOIR + 100];
        r.append(&blob);
        assert!(r.len() <= MAX_RESERVOIR);
    }

    #[test]
    fn first_frame_lookback_zero_ok() {
        let r = Reservoir::new();
        assert_eq!(r.view_from_lookback(0).unwrap(), &[] as &[u8]);
    }
}
