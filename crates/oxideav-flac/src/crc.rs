//! CRC-8 and CRC-16 tables used by FLAC frames.
//!
//! - **CRC-8** (poly 0x07, init 0): covers the frame header, ending right
//!   before the 8-bit CRC field.
//! - **CRC-16** (poly 0x8005, init 0, non-reflected): covers the entire
//!   frame including the header, ending before the 16-bit CRC at the end.

const CRC8_TABLE: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u8;
        let mut j = 0;
        while j < 8 {
            c = if c & 0x80 != 0 { (c << 1) ^ 0x07 } else { c << 1 };
            j += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
};

pub fn crc8(bytes: &[u8]) -> u8 {
    let mut c = 0u8;
    for &b in bytes {
        c = CRC8_TABLE[(c ^ b) as usize];
    }
    c
}

const CRC16_TABLE: [u16; 256] = {
    let mut t = [0u16; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = (i as u16) << 8;
        let mut j = 0;
        while j < 8 {
            c = if c & 0x8000 != 0 { (c << 1) ^ 0x8005 } else { c << 1 };
            j += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
};

pub fn crc16(bytes: &[u8]) -> u16 {
    let mut c = 0u16;
    for &b in bytes {
        c = (c << 8) ^ CRC16_TABLE[((c >> 8) as u8 ^ b) as usize];
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc8_zero_for_empty() {
        assert_eq!(crc8(&[]), 0);
        assert_eq!(crc16(&[]), 0);
    }

    #[test]
    fn deterministic() {
        assert_eq!(crc8(b"hello"), crc8(b"hello"));
        assert_ne!(crc8(b"hello"), crc8(b"hellp"));
        assert_eq!(crc16(b"world"), crc16(b"world"));
        assert_ne!(crc16(b"world"), crc16(b"worle"));
    }
}
