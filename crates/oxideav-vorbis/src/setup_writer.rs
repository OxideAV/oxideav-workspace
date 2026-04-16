//! Serialise a parsed Vorbis `Setup` back into a setup-packet byte stream.
//!
//! This is the inverse of [`crate::setup::parse_setup`]. We use it to build
//! stereo setups from the libvorbis mono reference: parse the mono setup
//! into a `Setup`, clone the codebooks/floors/residues, change the mapping
//! to 2 channels with no coupling, then re-emit. The round-trip is
//! self-consistent (our parser reads what we wrote).
//!
//! Not intended to reproduce libvorbis's exact byte layout for arbitrary
//! inputs — only to produce packets our decoder accepts.

use oxideav_core::{Error, Result};

use crate::bitwriter::BitWriter;
use crate::codebook::{Codebook, VqLookup};
use crate::setup::{Floor, Floor1, Mapping, Residue, Setup};

/// Serialise `setup` (with `audio_channels` in mind for mapping mux bits)
/// into a Vorbis setup packet. The bytes start with 0x05 "vorbis" and end
/// with a framing bit, exactly as required by the spec.
pub fn write_setup(setup: &Setup, audio_channels: u8) -> Result<Vec<u8>> {
    let mut w = BitWriter::with_capacity(4096);
    for &b in &[0x05u32, 0x76, 0x6f, 0x72, 0x62, 0x69, 0x73] {
        w.write_u32(b, 8);
    }

    // Codebooks.
    if setup.codebooks.is_empty() {
        return Err(Error::invalid("Vorbis setup_writer: no codebooks"));
    }
    w.write_u32(setup.codebooks.len() as u32 - 1, 8);
    for cb in &setup.codebooks {
        write_codebook(&mut w, cb)?;
    }

    // Time-domain placeholder: one entry of 16-bit zero.
    w.write_u32(0, 6); // count - 1 = 0
    w.write_u32(0, 16);

    // Floors.
    if setup.floors.is_empty() {
        return Err(Error::invalid("Vorbis setup_writer: no floors"));
    }
    w.write_u32(setup.floors.len() as u32 - 1, 6);
    for f in &setup.floors {
        match f {
            Floor::Type1(f1) => {
                w.write_u32(1, 16); // floor type 1
                write_floor1(&mut w, f1);
            }
            Floor::Type0(_) => {
                return Err(Error::unsupported(
                    "Vorbis setup_writer: floor type 0 not supported",
                ));
            }
        }
    }

    // Residues.
    if setup.residues.is_empty() {
        return Err(Error::invalid("Vorbis setup_writer: no residues"));
    }
    w.write_u32(setup.residues.len() as u32 - 1, 6);
    for r in &setup.residues {
        w.write_u32(r.kind as u32, 16);
        write_residue(&mut w, r);
    }

    // Mappings.
    if setup.mappings.is_empty() {
        return Err(Error::invalid("Vorbis setup_writer: no mappings"));
    }
    w.write_u32(setup.mappings.len() as u32 - 1, 6);
    for m in &setup.mappings {
        w.write_u32(0, 16); // mapping type = 0
        write_mapping(&mut w, m, audio_channels);
    }

    // Modes.
    if setup.modes.is_empty() {
        return Err(Error::invalid("Vorbis setup_writer: no modes"));
    }
    w.write_u32(setup.modes.len() as u32 - 1, 6);
    for md in &setup.modes {
        w.write_bit(md.blockflag);
        w.write_u32(md.windowtype as u32, 16);
        w.write_u32(md.transformtype as u32, 16);
        w.write_u32(md.mapping as u32, 8);
    }

    // Framing.
    w.write_bit(true);
    Ok(w.finish())
}

fn write_codebook(w: &mut BitWriter, cb: &Codebook) -> Result<()> {
    w.write_u32(0x564342, 24); // sync pattern "BCV"
    w.write_u32(cb.dimensions as u32, 16);
    w.write_u32(cb.entries, 24);

    // Decide sparse vs dense. We always emit "not ordered, sparse-if-any-unused".
    let any_unused = cb.codeword_lengths.contains(&0);
    w.write_bit(false); // ordered = false
    w.write_bit(any_unused); // sparse flag
    for &l in &cb.codeword_lengths {
        if any_unused {
            if l == 0 {
                w.write_bit(false); // used = false
            } else {
                w.write_bit(true); // used = true
                w.write_u32(l as u32 - 1, 5);
            }
        } else {
            w.write_u32(l as u32 - 1, 5);
        }
    }

    // Lookup.
    match &cb.vq {
        None => {
            w.write_u32(0, 4); // lookup_type = 0
        }
        Some(vq) => {
            w.write_u32(vq.lookup_type as u32, 4);
            write_vorbis_float(w, vq.min);
            write_vorbis_float(w, vq.delta);
            w.write_u32(vq.value_bits as u32 - 1, 4);
            w.write_bit(vq.sequence_p);
            // Expected number of multiplicands is determined by lookup_type.
            let expected = expected_multiplicand_count(vq, cb.entries, cb.dimensions as u32);
            if vq.multiplicands.len() != expected {
                return Err(Error::invalid(format!(
                    "Vorbis setup_writer: codebook multiplicand count {} != expected {}",
                    vq.multiplicands.len(),
                    expected
                )));
            }
            for &m in &vq.multiplicands {
                w.write_u32(m, vq.value_bits as u32);
            }
        }
    }
    Ok(())
}

fn expected_multiplicand_count(vq: &VqLookup, entries: u32, dim: u32) -> usize {
    match vq.lookup_type {
        1 => lookup1_values(entries, dim) as usize,
        2 => (entries as usize).saturating_mul(dim as usize),
        _ => 0,
    }
}

fn lookup1_values(entries: u32, dim: u32) -> u32 {
    if dim == 0 {
        return 0;
    }
    if dim == 1 {
        return entries;
    }
    let mut n = (entries as f64).powf(1.0 / dim as f64) as u32;
    while pow_overflow_check(n + 1, dim).is_some_and(|v| v <= entries) {
        n += 1;
    }
    while n > 0 && pow_overflow_check(n, dim).map_or(true, |v| v > entries) {
        n -= 1;
    }
    n
}

fn pow_overflow_check(base: u32, exp: u32) -> Option<u32> {
    let mut acc: u32 = 1;
    for _ in 0..exp {
        acc = acc.checked_mul(base)?;
    }
    Some(acc)
}

fn write_floor1(w: &mut BitWriter, f: &Floor1) {
    w.write_u32(f.partition_class_list.len() as u32, 5);
    for &c in &f.partition_class_list {
        w.write_u32(c as u32, 4);
    }
    let n_classes = f.class_dimensions.len();
    for c in 0..n_classes {
        w.write_u32(f.class_dimensions[c] as u32 - 1, 3);
        w.write_u32(f.class_subclasses[c] as u32, 2);
        if f.class_subclasses[c] != 0 {
            w.write_u32(f.class_masterbook[c] as u32, 8);
        }
        for &sb in &f.class_subbook[c] {
            // sb is -1 for "no book" or >=0 for a book index.
            // Stored value = sb + 1 (0 means no book).
            w.write_u32((sb + 1) as u32, 8);
        }
    }
    w.write_u32(f.multiplier as u32 - 1, 2);
    w.write_u32(f.rangebits as u32, 4);
    // xlist: first 2 values are implicit (0 and 2^rangebits), skip those.
    for &x in f.xlist.iter().skip(2) {
        w.write_u32(x, f.rangebits as u32);
    }
}

fn write_residue(w: &mut BitWriter, r: &Residue) {
    w.write_u32(r.begin, 24);
    w.write_u32(r.end, 24);
    w.write_u32(r.partition_size - 1, 24);
    w.write_u32(r.classifications as u32 - 1, 6);
    w.write_u32(r.classbook as u32, 8);
    for &c in &r.cascade {
        let low_bits = c & 0x07;
        let high_bits = (c >> 3) & 0x1F;
        w.write_u32(low_bits as u32, 3);
        if high_bits != 0 {
            w.write_bit(true);
            w.write_u32(high_bits as u32, 5);
        } else {
            w.write_bit(false);
        }
    }
    for (c, books) in r.books.iter().enumerate() {
        for j in 0..8 {
            if (r.cascade[c] & (1 << j)) != 0 {
                let book = books[j];
                debug_assert!(book >= 0, "residue book with cascade bit set must be >= 0");
                w.write_u32(book as u32, 8);
            }
        }
    }
}

fn write_mapping(w: &mut BitWriter, m: &Mapping, audio_channels: u8) {
    if m.submaps > 1 {
        w.write_bit(true);
        w.write_u32(m.submaps as u32 - 1, 4);
    } else {
        w.write_bit(false);
    }
    if !m.coupling.is_empty() {
        w.write_bit(true);
        w.write_u32(m.coupling.len() as u32 - 1, 8);
        let field_bits = ilog((audio_channels as i32 - 1).max(0) as u32);
        for &(mag, ang) in &m.coupling {
            w.write_u32(mag as u32, field_bits);
            w.write_u32(ang as u32, field_bits);
        }
    } else {
        w.write_bit(false);
    }
    w.write_u32(0, 2); // reserved
    if m.submaps > 1 {
        // `m.mux` length is audio_channels at parse time — clone that size.
        for i in 0..audio_channels as usize {
            let v = m.mux.get(i).copied().unwrap_or(0);
            w.write_u32(v as u32, 4);
        }
    }
    for s in 0..m.submaps as usize {
        w.write_u32(0, 8); // time index (discarded)
        w.write_u32(m.submap_floor[s] as u32, 8);
        w.write_u32(m.submap_residue[s] as u32, 8);
    }
}

/// Encode `value` as a 32-bit Vorbis float (§9.2.2 reverse of
/// `read_vorbis_float`).
fn write_vorbis_float(w: &mut BitWriter, value: f32) {
    if value == 0.0 {
        w.write_u32(0, 32);
        return;
    }
    let abs = value.abs() as f64;
    let mut mantissa = abs;
    let mut exp: i32 = 0;
    while mantissa < (1u64 << 20) as f64 {
        mantissa *= 2.0;
        exp -= 1;
    }
    while mantissa >= (1u64 << 21) as f64 {
        mantissa /= 2.0;
        exp += 1;
    }
    let m = mantissa as u32 & 0x001F_FFFF;
    let biased = (exp + 788) as u32;
    debug_assert!(biased < 1024, "Vorbis float exponent out of range");
    let sign_bit = if value < 0.0 { 0x8000_0000u32 } else { 0 };
    let raw = sign_bit | ((biased & 0x3FF) << 21) | m;
    w.write_u32(raw, 32);
}

fn ilog(value: u32) -> u32 {
    if value == 0 {
        0
    } else {
        32 - value.leading_zeros()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::libvorbis_setup::LIBVORBIS_SETUP_MONO_48K_Q3;
    use crate::setup::parse_setup;

    /// Round-trip the libvorbis mono setup through our parse → write → parse
    /// pipeline. The re-serialised bytes must parse back into a Setup with
    /// identical structure (same counts, same codebook shapes).
    #[test]
    fn mono_roundtrip() {
        let original =
            parse_setup(LIBVORBIS_SETUP_MONO_48K_Q3, 1).expect("parse libvorbis mono setup");
        let re = write_setup(&original, 1).expect("write mono");
        let round = parse_setup(&re, 1).expect("re-parse");
        assert_eq!(round.codebooks.len(), original.codebooks.len());
        assert_eq!(round.floors.len(), original.floors.len());
        assert_eq!(round.residues.len(), original.residues.len());
        assert_eq!(round.mappings.len(), original.mappings.len());
        assert_eq!(round.modes.len(), original.modes.len());
        // Spot-check a few codebook shapes.
        for (a, b) in round.codebooks.iter().zip(original.codebooks.iter()) {
            assert_eq!(a.dimensions, b.dimensions);
            assert_eq!(a.entries, b.entries);
            assert_eq!(a.codeword_lengths, b.codeword_lengths);
            assert_eq!(a.vq.is_some(), b.vq.is_some());
            if let (Some(va), Some(vb)) = (&a.vq, &b.vq) {
                assert_eq!(va.lookup_type, vb.lookup_type);
                assert_eq!(va.value_bits, vb.value_bits);
                assert_eq!(va.sequence_p, vb.sequence_p);
                assert_eq!(va.multiplicands, vb.multiplicands);
            }
        }
    }

    /// Build a 2-channel setup from the mono one by changing the mapping's
    /// mux to cover 2 channels with no coupling. The result must parse
    /// cleanly under audio_channels=2.
    #[test]
    fn stereo_from_mono_parses() {
        let mut setup =
            parse_setup(LIBVORBIS_SETUP_MONO_48K_Q3, 1).expect("parse libvorbis mono setup");
        // Switch mappings to 2-channel, no coupling.
        for m in &mut setup.mappings {
            m.coupling.clear();
            m.mux = vec![0u8; 2];
        }
        let re = write_setup(&setup, 2).expect("write stereo");
        let round = parse_setup(&re, 2).expect("re-parse stereo");
        assert_eq!(round.codebooks.len(), setup.codebooks.len());
        assert_eq!(round.mappings.len(), setup.mappings.len());
        assert_eq!(round.mappings[0].coupling.len(), 0);
    }
}
