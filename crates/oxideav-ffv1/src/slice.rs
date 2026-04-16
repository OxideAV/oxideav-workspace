//! Slice-level encode/decode for FFV1 version 3.
//!
//! A slice is the independently-decodable unit. Our simple profile emits a
//! single slice covering the whole frame. Each slice contains:
//! 1. A range-coded slice header (position, size, quant_table_set index,
//!    picture_structure, sar).
//! 2. Per-plane pixel data, median-predicted + context-modelled, coded via
//!    the range coder.
//! 3. A byte-aligned footer with the 24-bit slice size. Optional CRC32
//!    parity (when `ec != 0`) isn't emitted by our encoder and isn't
//!    validated by our decoder.

use oxideav_core::{Error, Result};

use crate::predictor::predict;
use crate::range_coder::{RangeDecoder, RangeEncoder};
use crate::state::{compute_context, context_count, default_quant_tables, PlaneState, QuantTables};

/// Geometry for a single plane within a slice.
#[derive(Clone, Copy, Debug)]
pub struct PlaneGeom {
    pub width: u32,
    pub height: u32,
}

/// Encode a single plane's worth of 8-bit samples to range-coded residuals.
/// `samples` is a row-major buffer of exactly `width * height` bytes (no
/// stride); the caller should arrange that. `state` holds the per-context
/// range-coder states, one entry per context index.
pub fn encode_plane(
    enc: &mut RangeEncoder,
    samples: &[u8],
    width: u32,
    height: u32,
    tables: &QuantTables,
    state: &mut PlaneState,
) {
    let w = width as usize;
    let h = height as usize;
    assert_eq!(samples.len(), w * h, "encode_plane: dimensions mismatch");

    for y in 0..h {
        for x in 0..w {
            let s = samples[y * w + x] as i32;
            let (big_l, l, t, tl, big_t, tr) = neighbours(samples, w, h, x, y);
            let mut ctx = compute_context(tables, big_l, l, t, tl, big_t, tr);
            let sign_flip = ctx < 0;
            if sign_flip {
                ctx = -ctx;
            }
            let pred = predict(l, t, tl);
            // Wrap residual into signed 8-bit domain (n LSBs of difference,
            // interpreted as signed).
            let diff = s - pred;
            let wrapped: i32 = (diff << 24) >> 24; // sign-extend low 8 bits
            let residual = if sign_flip { -wrapped } else { wrapped };
            let state_row = &mut state.states[ctx as usize];
            enc.put_symbol(state_row, residual, true);
        }
    }
}

/// Decode a single plane's worth of 8-bit samples. Mirrors `encode_plane`.
pub fn decode_plane(
    dec: &mut RangeDecoder<'_>,
    samples: &mut [u8],
    width: u32,
    height: u32,
    tables: &QuantTables,
    state: &mut PlaneState,
) -> Result<()> {
    let w = width as usize;
    let h = height as usize;
    if samples.len() != w * h {
        return Err(Error::invalid("decode_plane: buffer length mismatch"));
    }
    for y in 0..h {
        for x in 0..w {
            let (big_l, l, t, tl, big_t, tr) = neighbours(samples, w, h, x, y);
            let mut ctx = compute_context(tables, big_l, l, t, tl, big_t, tr);
            let sign_flip = ctx < 0;
            if sign_flip {
                ctx = -ctx;
            }
            let pred = predict(l, t, tl);
            let state_row = &mut state.states[ctx as usize];
            let mut residual = dec.get_symbol(state_row, true);
            if sign_flip {
                residual = -residual;
            }
            // Reconstruct: pred + residual, wrapped into 8-bit. Since the
            // encoder already wrapped the diff to signed 8-bit, adding back
            // may need to be masked.
            let recon = ((pred + residual) as u32) & 0xFF;
            samples[y * w + x] = recon as u8;
        }
    }
    Ok(())
}

/// Fetch the six-tap neighbourhood (L, l, t, tl, T, tr) for sample (x,y). Per
/// FFV1 §3.8 these are sampled from positions:
///
/// ```text
///        TL  T  TR
///     L   l  X
///     (big L is two left of X)
/// ```
///
/// Missing neighbours (off-image) are replaced with the closest edge value;
/// corner cases follow FFmpeg's behaviour.
#[inline]
fn neighbours(
    samples: &[u8],
    w: usize,
    h: usize,
    x: usize,
    y: usize,
) -> (i32, i32, i32, i32, i32, i32) {
    let _ = h;
    // l = sample to the left of X; L = sample two to the left.
    let l = if x >= 1 {
        samples[y * w + (x - 1)] as i32
    } else {
        0
    };
    let big_l = if x >= 2 {
        samples[y * w + (x - 2)] as i32
    } else {
        l
    };
    // Top row neighbours.
    let (t, tl, tr, big_t) = if y >= 1 {
        let top_row = (y - 1) * w;
        let t = samples[top_row + x] as i32;
        let tl = if x >= 1 {
            samples[top_row + (x - 1)] as i32
        } else {
            t
        };
        let tr = if x + 1 < w {
            samples[top_row + (x + 1)] as i32
        } else {
            t
        };
        let big_t = if y >= 2 {
            samples[(y - 2) * w + x] as i32
        } else {
            t
        };
        (t, tl, tr, big_t)
    } else {
        // No row above — FFmpeg uses 0 for T/TL/TR and copies left for
        // big_T. Use l as a reasonable proxy and 0 for the top-row fetches.
        (l, l, l, 0)
    };
    (big_l, l, t, tl, big_t, tr)
}

// -------------------------------------------------------------------------
// Slice header
// -------------------------------------------------------------------------

/// Range-coded slice header fields for our single-slice profile.
pub struct SliceHeader {
    pub slice_x: u32,
    pub slice_y: u32,
    pub slice_w_minus1: u32,
    pub slice_h_minus1: u32,
    /// Per-plane quant_table_set index. For our simple profile this is [0;3].
    pub qt_idx: [u32; 3],
    pub picture_structure: u32,
    pub sar_num: u32,
    pub sar_den: u32,
}

impl SliceHeader {
    pub fn default_full_frame(num_planes: usize, num_h: u32, num_v: u32) -> Self {
        let _ = num_planes;
        Self {
            slice_x: 0,
            slice_y: 0,
            slice_w_minus1: num_h - 1,
            slice_h_minus1: num_v - 1,
            qt_idx: [0; 3],
            picture_structure: 0,
            sar_num: 0,
            sar_den: 0,
        }
    }

    pub fn encode(&self, enc: &mut RangeEncoder, num_planes: usize) {
        let mut st = [128u8; 32];
        enc.put_symbol_u(&mut st, self.slice_x);
        enc.put_symbol_u(&mut st, self.slice_y);
        enc.put_symbol_u(&mut st, self.slice_w_minus1);
        enc.put_symbol_u(&mut st, self.slice_h_minus1);
        for i in 0..num_planes {
            enc.put_symbol_u(&mut st, self.qt_idx[i]);
        }
        enc.put_symbol_u(&mut st, self.picture_structure);
        enc.put_symbol_u(&mut st, self.sar_num);
        enc.put_symbol_u(&mut st, self.sar_den);
    }

    pub fn parse(dec: &mut RangeDecoder<'_>, num_planes: usize) -> Result<Self> {
        let mut st = [128u8; 32];
        let slice_x = dec.get_symbol_u(&mut st);
        let slice_y = dec.get_symbol_u(&mut st);
        let slice_w_minus1 = dec.get_symbol_u(&mut st);
        let slice_h_minus1 = dec.get_symbol_u(&mut st);
        let mut qt_idx = [0u32; 3];
        for i in 0..num_planes.min(3) {
            qt_idx[i] = dec.get_symbol_u(&mut st);
        }
        let picture_structure = dec.get_symbol_u(&mut st);
        let sar_num = dec.get_symbol_u(&mut st);
        let sar_den = dec.get_symbol_u(&mut st);
        Ok(Self {
            slice_x,
            slice_y,
            slice_w_minus1,
            slice_h_minus1,
            qt_idx,
            picture_structure,
            sar_num,
            sar_den,
        })
    }
}

// -------------------------------------------------------------------------
// Whole-slice encode for our single-slice profile
// -------------------------------------------------------------------------

/// A lightweight view of one slice's per-plane pixel data. Each plane is a
/// contiguous row-major buffer of exactly `width * height` bytes.
pub struct SlicePlanes<'a> {
    pub y: &'a [u8],
    pub u: Option<&'a [u8]>,
    pub v: Option<&'a [u8]>,
    pub y_geom: PlaneGeom,
    pub c_geom: PlaneGeom,
}

/// Encode a single FFV1 slice covering the whole frame: header + planes +
/// 3-byte size footer. Returns the slice bytes, suitable to be packed into a
/// packet.
pub fn encode_slice(planes: &SlicePlanes<'_>) -> Vec<u8> {
    let tables = default_quant_tables();
    let ctx_count = context_count(&tables);

    // One range-coder pass: header + data.
    let mut enc = RangeEncoder::new();
    let num_planes = if planes.u.is_some() && planes.v.is_some() {
        3
    } else {
        1
    };
    let hdr = SliceHeader::default_full_frame(num_planes, 1, 1);
    hdr.encode(&mut enc, num_planes);

    let mut plane_state = PlaneState::new(ctx_count);
    encode_plane(
        &mut enc,
        planes.y,
        planes.y_geom.width,
        planes.y_geom.height,
        &tables,
        &mut plane_state,
    );
    if let (Some(u), Some(v)) = (planes.u, planes.v) {
        plane_state.reset();
        encode_plane(
            &mut enc,
            u,
            planes.c_geom.width,
            planes.c_geom.height,
            &tables,
            &mut plane_state,
        );
        plane_state.reset();
        encode_plane(
            &mut enc,
            v,
            planes.c_geom.width,
            planes.c_geom.height,
            &tables,
            &mut plane_state,
        );
    }
    let mut bytes = enc.finish();

    // Slice-size footer: 3-byte big-endian length of `bytes` (excluding the
    // 3 footer bytes themselves, per FFmpeg convention).
    let len = bytes.len() as u32;
    bytes.push(((len >> 16) & 0xFF) as u8);
    bytes.push(((len >> 8) & 0xFF) as u8);
    bytes.push((len & 0xFF) as u8);
    bytes
}

/// Layout of a decoded slice.
pub struct DecodedSlice {
    pub y: Vec<u8>,
    pub u: Option<Vec<u8>>,
    pub v: Option<Vec<u8>>,
    pub y_geom: PlaneGeom,
    pub c_geom: PlaneGeom,
}

/// Decode one slice with the given plane geometry. The caller provides the
/// expected geometry (derived from the configuration record + frame size).
pub fn decode_slice(
    bytes: &[u8],
    y_geom: PlaneGeom,
    c_geom: Option<PlaneGeom>,
) -> Result<DecodedSlice> {
    // Strip the 3-byte size footer.
    if bytes.len() < 3 {
        return Err(Error::invalid("FFV1 slice too short"));
    }
    let body = &bytes[..bytes.len() - 3];

    let tables = default_quant_tables();
    let ctx_count = context_count(&tables);

    let mut dec = RangeDecoder::new(body);
    let num_planes = if c_geom.is_some() { 3 } else { 1 };
    let _hdr = SliceHeader::parse(&mut dec, num_planes)?;

    let mut plane_state = PlaneState::new(ctx_count);
    let mut y_buf = vec![0u8; (y_geom.width * y_geom.height) as usize];
    decode_plane(
        &mut dec,
        &mut y_buf,
        y_geom.width,
        y_geom.height,
        &tables,
        &mut plane_state,
    )?;

    let (u_buf, v_buf) = if let Some(cg) = c_geom {
        let n = (cg.width * cg.height) as usize;
        let mut u = vec![0u8; n];
        let mut v = vec![0u8; n];
        plane_state.reset();
        decode_plane(
            &mut dec,
            &mut u,
            cg.width,
            cg.height,
            &tables,
            &mut plane_state,
        )?;
        plane_state.reset();
        decode_plane(
            &mut dec,
            &mut v,
            cg.width,
            cg.height,
            &tables,
            &mut plane_state,
        )?;
        (Some(u), Some(v))
    } else {
        (None, None)
    };

    Ok(DecodedSlice {
        y: y_buf,
        u: u_buf,
        v: v_buf,
        y_geom,
        c_geom: c_geom.unwrap_or(y_geom),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_plane_roundtrip_flat() {
        // Flat 16x16 gray plane; every sample equals 128.
        let src = vec![128u8; 16 * 16];
        let planes = SlicePlanes {
            y: &src,
            u: None,
            v: None,
            y_geom: PlaneGeom {
                width: 16,
                height: 16,
            },
            c_geom: PlaneGeom {
                width: 0,
                height: 0,
            },
        };
        let bytes = encode_slice(&planes);
        let decoded = decode_slice(&bytes, planes.y_geom, None).expect("decode");
        assert_eq!(decoded.y, src);
    }

    #[test]
    fn single_plane_roundtrip_gradient() {
        let w = 8u32;
        let h = 8u32;
        let mut src = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            for x in 0..w {
                src.push(((x * 32 + y * 4) & 0xFF) as u8);
            }
        }
        let planes = SlicePlanes {
            y: &src,
            u: None,
            v: None,
            y_geom: PlaneGeom {
                width: w,
                height: h,
            },
            c_geom: PlaneGeom {
                width: 0,
                height: 0,
            },
        };
        let bytes = encode_slice(&planes);
        let decoded = decode_slice(&bytes, planes.y_geom, None).expect("decode");
        assert_eq!(decoded.y, src);
    }

    #[test]
    fn three_plane_roundtrip_420() {
        let w = 16u32;
        let h = 16u32;
        let cw = w / 2;
        let ch = h / 2;
        let y_src: Vec<u8> = (0..w * h).map(|i| ((i * 7) & 0xFF) as u8).collect();
        let u_src: Vec<u8> = (0..cw * ch).map(|i| ((i * 13 + 64) & 0xFF) as u8).collect();
        let v_src: Vec<u8> = (0..cw * ch)
            .map(|i| ((i * 19 + 128) & 0xFF) as u8)
            .collect();
        let planes = SlicePlanes {
            y: &y_src,
            u: Some(&u_src),
            v: Some(&v_src),
            y_geom: PlaneGeom {
                width: w,
                height: h,
            },
            c_geom: PlaneGeom {
                width: cw,
                height: ch,
            },
        };
        let bytes = encode_slice(&planes);
        let decoded = decode_slice(&bytes, planes.y_geom, Some(planes.c_geom)).expect("decode");
        assert_eq!(decoded.y, y_src);
        assert_eq!(decoded.u, Some(u_src));
        assert_eq!(decoded.v, Some(v_src));
    }

    #[test]
    fn three_plane_roundtrip_444_random() {
        let w = 32u32;
        let h = 24u32;
        let mut rng = 0xdead_beefu32;
        let mut rand_byte = || {
            rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
            (rng >> 16) as u8
        };
        let y_src: Vec<u8> = (0..w * h).map(|_| rand_byte()).collect();
        let u_src: Vec<u8> = (0..w * h).map(|_| rand_byte()).collect();
        let v_src: Vec<u8> = (0..w * h).map(|_| rand_byte()).collect();
        let planes = SlicePlanes {
            y: &y_src,
            u: Some(&u_src),
            v: Some(&v_src),
            y_geom: PlaneGeom {
                width: w,
                height: h,
            },
            c_geom: PlaneGeom {
                width: w,
                height: h,
            },
        };
        let bytes = encode_slice(&planes);
        let decoded = decode_slice(&bytes, planes.y_geom, Some(planes.c_geom)).expect("decode");
        assert_eq!(decoded.y, y_src);
        assert_eq!(decoded.u, Some(u_src));
        assert_eq!(decoded.v, Some(v_src));
    }
}
