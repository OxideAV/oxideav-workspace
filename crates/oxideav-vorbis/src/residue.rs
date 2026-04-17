//! Vorbis residue decoding (types 0, 1, 2).
//!
//! Reference: Vorbis I §8 — the residue layer turns a sequence of codebook
//! entries into a per-channel spectral residual that gets multiplied with
//! the floor curve to form the final frequency-domain spectrum.
//!
//! Types 0 and 1 share the same classification/cascade structure and differ
//! only in how the decoded codebook values are laid out *within* a
//! partition (§8.3 / §8.4):
//!
//! - Type 1: the `psz / dim` codewords are stored concatenated, i.e. entry
//!   `i`'s element `j` lands at `offset + i*dim + j`.
//! - Type 0: the codewords are *interleaved*; entry `i`'s element `j` lands
//!   at `offset + i + j*step` where `step = psz / dim` (see §8.6.3).
//!
//! Type 2 reduces to type 1 on a single pre-interleaved vector of length
//! `n_channels * n` which the caller then deinterleaves back to per-channel
//! slots — we do that inline in `decode_residue`.

use oxideav_core::{Error, Result};

use crate::bitreader::BitReader;
use crate::codebook::Codebook;
use crate::setup::Residue;

/// Decode `n_channels` residue vectors of length `n` from the bitstream and
/// add them into `vectors[ch][bin]`. Channels marked `do_not_decode[ch] =
/// true` are skipped (still consume zero bits but produce no output).
pub fn decode_residue(
    residue: &Residue,
    codebooks: &[Codebook],
    n: usize,
    do_not_decode: &[bool],
    vectors: &mut [Vec<f32>],
    br: &mut BitReader<'_>,
) -> Result<()> {
    let n_channels = vectors.len();
    if do_not_decode.len() != n_channels {
        return Err(Error::invalid("residue: do_not_decode length mismatch"));
    }
    if residue.kind == 2 {
        // Type 2: decode a single interleaved vector of length n_channels * n,
        // then deinterleave into per-channel slots.
        let total_len = n_channels * n;
        // If every channel is "do not decode", the entire residue is zero.
        if do_not_decode.iter().all(|&v| v) {
            return Ok(());
        }
        let mut interleaved = vec![0f32; total_len];
        decode_partitioned(
            residue,
            codebooks,
            total_len,
            &mut [&mut interleaved[..]],
            br,
        )?;
        // Deinterleave: sample i*n_channels + ch goes into channel ch.
        for ch in 0..n_channels {
            if do_not_decode[ch] {
                continue;
            }
            for i in 0..n {
                vectors[ch][i] += interleaved[i * n_channels + ch];
            }
        }
        Ok(())
    } else {
        // Type 0 / 1: per-channel decode. The classification/cascade layout
        // is identical; the placement rule inside each partition differs
        // (see module docs) and is handled by `decode_partitioned` via
        // `residue.kind`.
        let mut slots: Vec<&mut [f32]> = vectors.iter_mut().map(|v| v.as_mut_slice()).collect();
        decode_partitioned(residue, codebooks, n, &mut slots[..], br)?;
        Ok(())
    }
}

/// Shared partition/cascade decode loop for residue types 0/1/2.
///
/// `vectors` carries one or more output channel slices, each of length
/// `n`. For type 2 the caller pre-flattens to a single slice; for types
/// 0/1 each entry is a per-channel slice.
fn decode_partitioned(
    residue: &Residue,
    codebooks: &[Codebook],
    n: usize,
    vectors: &mut [&mut [f32]],
    br: &mut BitReader<'_>,
) -> Result<()> {
    let psz = residue.partition_size as usize;
    let begin = residue.begin as usize;
    let end = (residue.end as usize).min(n);
    if end <= begin {
        return Ok(());
    }
    let n_to_read = end - begin;
    if n_to_read % psz != 0 {
        return Err(Error::invalid(
            "Vorbis residue: (end-begin) not divisible by partition_size",
        ));
    }
    let n_partitions = n_to_read / psz;
    let n_channels = vectors.len();

    let classbook = &codebooks[residue.classbook as usize];
    let classwords_per_codeword = classbook.dimensions as usize;
    let classifications = residue.classifications as usize;
    let trace = std::env::var_os("OXIDEAV_VORBIS_TRACE").is_some();
    if trace {
        eprintln!(
            "[vorbis] residue kind={} begin={} end={} psz={} parts={} classbook_dim={} classes={} bitpos={}",
            residue.kind,
            begin,
            end,
            psz,
            n_partitions,
            classwords_per_codeword,
            classifications,
            br.bit_position()
        );
    }

    // Pre-decode per-channel partition class assignments. For each pass
    // (cascade level), partitions whose class has a book at that pass get
    // their values read.
    // Number of classifications-assignments to decode = ceil(n_partitions / classwords_per_codeword).
    let n_class_codewords = n_partitions.div_ceil(classwords_per_codeword);
    let mut classifications_table: Vec<Vec<u32>> = vec![vec![0; n_partitions]; n_channels];

    // Vorbis cascades 8 passes; on pass 0 we also fetch the per-partition class.
    // Per Vorbis I §8.6.3, hitting end-of-packet mid-decode is NOT an error —
    // remaining partitions/passes stay zero-filled. We use `Error::Eof` from
    // the bit reader as the EOP signal.
    'cascade: for pass in 0..8u32 {
        let mut partition_idx = 0usize;
        while partition_idx < n_partitions {
            if pass == 0 {
                for ch in 0..n_channels {
                    let class_id = match classbook.decode_scalar(br) {
                        Ok(v) => v,
                        Err(Error::Eof) => break 'cascade,
                        Err(e) => return Err(e),
                    };
                    if trace {
                        eprintln!(
                            "[vorbis] part_idx={} ch={} class_id={} bitpos={}",
                            partition_idx,
                            ch,
                            class_id,
                            br.bit_position()
                        );
                    }
                    // Vorbis I §8.6.2 step 6: decompose into
                    // base-`classifications` digits HIGH-DIGIT-FIRST. The
                    // LAST partition in the group gets class_id % classifications;
                    // the FIRST partition gets the high-order digit.
                    let mut tmp = class_id;
                    for i in (0..classwords_per_codeword).rev() {
                        if partition_idx + i < n_partitions {
                            classifications_table[ch][partition_idx + i] =
                                tmp % classifications as u32;
                        }
                        tmp /= classifications as u32;
                    }
                }
            }
            // Decode `classwords_per_codeword` partitions per outer step.
            for k in 0..classwords_per_codeword {
                let pidx = partition_idx + k;
                if pidx >= n_partitions {
                    break;
                }
                for ch in 0..n_channels {
                    let class_id = classifications_table[ch][pidx] as usize;
                    let book_index = residue.books[class_id][pass as usize];
                    if book_index < 0 {
                        if trace {
                            eprintln!(
                                "[vorbis] pass={} pidx={} ch={} class={} no book — skip",
                                pass, pidx, ch, class_id
                            );
                        }
                        continue;
                    }
                    let book = &codebooks[book_index as usize];
                    let dim = book.dimensions as usize;
                    let bin_start = begin + pidx * psz;
                    let bin_end = bin_start + psz;
                    if trace {
                        eprintln!(
                            "[vorbis] pass={} pidx={} ch={} class={} book={} dim={} bitpos={}",
                            pass,
                            pidx,
                            ch,
                            class_id,
                            book_index,
                            dim,
                            br.bit_position()
                        );
                    }
                    // Spec requires `psz` to be an integer multiple of the
                    // book dimension; otherwise layout is undefined.
                    if dim == 0 || psz % dim != 0 {
                        return Err(Error::invalid(
                            "Vorbis residue: partition_size not a multiple of book dimension",
                        ));
                    }
                    let n_codewords = psz / dim;
                    if residue.kind == 0 {
                        // Type 0 (§8.6.3): step = n/dim; codeword i's element
                        // j is placed at bin_start + i + j*step (interleaved).
                        let step = n_codewords;
                        for i in 0..step {
                            let entry = match book.decode_scalar(br) {
                                Ok(v) => v,
                                Err(Error::Eof) => break 'cascade,
                                Err(e) => return Err(e),
                            };
                            let vq = book.vq_lookup(entry)?;
                            for j in 0..dim {
                                let bin_ij = bin_start + i + j * step;
                                if bin_ij < bin_end && bin_ij < vectors[ch].len() {
                                    vectors[ch][bin_ij] += vq[j];
                                }
                            }
                        }
                    } else {
                        // Type 1 / 2 (§8.6.4): codewords stored concatenated;
                        // element j of codeword i lands at bin_start + i*dim + j.
                        let mut bin = bin_start;
                        while bin < bin_end {
                            let entry = match book.decode_scalar(br) {
                                Ok(v) => v,
                                Err(Error::Eof) => break 'cascade,
                                Err(e) => return Err(e),
                            };
                            let vq = book.vq_lookup(entry)?;
                            for j in 0..dim {
                                if bin + j < bin_end && bin + j < vectors[ch].len() {
                                    vectors[ch][bin + j] += vq[j];
                                }
                            }
                            bin += dim;
                        }
                    }
                }
            }
            partition_idx += classwords_per_codeword;
            let _ = n_class_codewords;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bitwriter::BitWriter;
    use crate::codebook::{Codebook, VqLookup};

    /// Build a 1-entry zero-length classbook. `decode_scalar` consumes 0
    /// bits and returns entry 0 (single-used-entry special case).
    fn make_classbook_single() -> Codebook {
        let mut cb = Codebook {
            dimensions: 1,
            entries: 1,
            codeword_lengths: vec![0],
            vq: None,
            codewords: Vec::new(),
        };
        cb.build_decoder().expect("classbook builds");
        cb
    }

    /// Build a 4-entry dim-2 VQ book (codewords 00/01/10/11). Entry e maps
    /// via lookup type 1 multiplicands `[0, 1]` (min=0, delta=1) to the
    /// vector `(e % 2, e / 2)` — i.e. entries 0..3 are `[0,0] [1,0] [0,1]
    /// [1,1]`.
    fn make_vq_book_dim2() -> Codebook {
        let mut cb = Codebook {
            dimensions: 2,
            entries: 4,
            codeword_lengths: vec![2, 2, 2, 2],
            vq: Some(VqLookup {
                lookup_type: 1,
                min: 0.0,
                delta: 1.0,
                value_bits: 1,
                sequence_p: false,
                multiplicands: vec![0, 1],
            }),
            codewords: Vec::new(),
        };
        cb.build_decoder().expect("VQ book builds");
        // Sanity: libvorbis-style _make_words assigns 00,01,10,11 to
        // entries 0..3.
        assert_eq!(cb.codewords, vec![0b00, 0b01, 0b10, 0b11]);
        cb
    }

    /// Emit `entry`'s codeword MSB-first into `w` at the given length. Our
    /// `decode_scalar` accumulates stream bits MSB-first into `code`, so
    /// emitting bits high-to-low recovers the entry on the other side.
    fn emit_msb_first(w: &mut BitWriter, code: u32, len: u32) {
        for i in (0..len).rev() {
            w.write_bit(((code >> i) & 1) != 0);
        }
    }

    /// Common residue descriptor: kind configurable, begin=0, end=8, psz=4,
    /// 1 class, classbook=0 (single-entry), cascade pass 0 uses book 1.
    fn make_residue(kind: u16) -> Residue {
        Residue {
            kind,
            begin: 0,
            end: 8,
            partition_size: 4,
            classifications: 1,
            classbook: 0,
            cascade: vec![0b001],
            books: vec![[1, -1, -1, -1, -1, -1, -1, -1]],
        }
    }

    /// Type 0 deinterleaving: with psz=4, dim=2 → step=2, codeword i's
    /// element j lands at bin_start + i + j*step. So reading entries
    /// `[1, 2]` into partition 0 yields values `[1, 0, 0, 1]` at bins
    /// 0,1,2,3 — NOT the concatenated `[1, 0, 0, 1]` that type-1 would
    /// produce (which happens to coincide here). Use asymmetric entries
    /// to force divergence: `[2, 1]` → codeword 0 = [0,1] at bins 0,2,
    /// codeword 1 = [1,0] at bins 1,3 → `[0, 1, 1, 0]`.
    #[test]
    fn type0_deinterleaves_codewords_within_partition() {
        let classbook = make_classbook_single();
        let vq = make_vq_book_dim2();
        let codebooks = vec![classbook, vq];

        // Emit entries for 2 partitions in cascade pass 0:
        //   partition 0: codewords entry 2, entry 1  → vectors [0,1], [1,0]
        //   partition 1: codewords entry 3, entry 0  → vectors [1,1], [0,0]
        let mut w = BitWriter::new();
        // Pass 0: classbook consumes 0 bits per class-codeword group (1
        // partition per group, dim=1, 1 class). 2 partitions → 2 class
        // codeword reads, each 0 bits. Then read 2 VQ codewords per
        // partition.
        emit_msb_first(&mut w, 0b10, 2); // entry 2 → [0,1]
        emit_msb_first(&mut w, 0b01, 2); // entry 1 → [1,0]
        emit_msb_first(&mut w, 0b11, 2); // entry 3 → [1,1]
        emit_msb_first(&mut w, 0b00, 2); // entry 0 → [0,0]
        let data = w.finish();

        let residue = make_residue(0);
        let mut br = BitReader::new(&data);
        let mut vectors: Vec<Vec<f32>> = vec![vec![0f32; 8]];
        decode_residue(&residue, &codebooks, 8, &[false], &mut vectors, &mut br)
            .expect("type-0 decode");
        // Partition 0 (bins 0..4): codeword 0 → [0,1] at bins 0,2;
        // codeword 1 → [1,0] at bins 1,3 → [0, 1, 1, 0].
        // Partition 1 (bins 4..8): codeword 0 → [1,1] at bins 4,6;
        // codeword 1 → [0,0] at bins 5,7 → [1, 0, 1, 0].
        assert_eq!(vectors[0], vec![0.0, 1.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0]);
    }

    /// Spot-check: type 1 on the same codeword stream produces the
    /// concatenated layout — proof the two code paths actually differ.
    #[test]
    fn type1_concatenates_codewords_within_partition() {
        let classbook = make_classbook_single();
        let vq = make_vq_book_dim2();
        let codebooks = vec![classbook, vq];

        let mut w = BitWriter::new();
        emit_msb_first(&mut w, 0b10, 2); // entry 2 → [0,1]
        emit_msb_first(&mut w, 0b01, 2); // entry 1 → [1,0]
        emit_msb_first(&mut w, 0b11, 2); // entry 3 → [1,1]
        emit_msb_first(&mut w, 0b00, 2); // entry 0 → [0,0]
        let data = w.finish();

        let residue = make_residue(1);
        let mut br = BitReader::new(&data);
        let mut vectors: Vec<Vec<f32>> = vec![vec![0f32; 8]];
        decode_residue(&residue, &codebooks, 8, &[false], &mut vectors, &mut br)
            .expect("type-1 decode");
        // Partition 0: [0,1] then [1,0] concatenated → [0,1,1,0].
        // Partition 1: [1,1] then [0,0] concatenated → [1,1,0,0].
        assert_eq!(vectors[0], vec![0.0, 1.0, 1.0, 0.0, 1.0, 1.0, 0.0, 0.0]);
    }
}
