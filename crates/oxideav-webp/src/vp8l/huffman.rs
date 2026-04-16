//! Canonical Huffman trees for VP8L.
//!
//! VP8L's Huffman encoding has two shapes:
//!
//! * **Simple code** — 1 or 2 symbols encoded in 1-3 bits each. Used for
//!   alphabets that collapse to a single literal or a binary choice.
//! * **Normal code** — RFC 1951-style canonical Huffman. First the "code
//!   lengths of the code lengths" alphabet is read with fixed ordering,
//!   then the actual per-symbol code lengths via that meta-tree (with
//!   repeat/zero-run codes), then canonical codes are assembled.
//!
//! The implementation stores the tree as a flat `(left, right)` link
//! vector; each internal node is indexed by a u32 offset from the root.
//! Leaves store the decoded symbol. Decoding is a straight bit-by-bit
//! walk — this is plenty fast for still-image sized alphabets and keeps
//! the code short.

use oxideav_core::{Error, Result};

use super::bit_reader::BitReader;

/// Fixed order in which code lengths for the "meta alphabet" are read
/// (spec §6.2.5).
const CODE_LENGTH_ORDER: [usize; 19] = [
    17, 18, 0, 1, 2, 3, 4, 5, 16, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
];

/// Decoded symbol type. Kept as a plain u16 — VP8L alphabets fit
/// comfortably in 16 bits (largest is ~2072 = 256 + 24 + cache).
pub type HuffmanCode = u16;

/// A Huffman tree ready for bit-by-bit decode.
pub struct HuffmanTree {
    /// Single-symbol shortcut: if present, every read emits this symbol
    /// (consumes no bits).
    only_symbol: Option<HuffmanCode>,
    /// Flat node array. The root is node 0. Each non-leaf `Node::Internal`
    /// stores indices to its 0/1 children. Leaves store the decoded
    /// symbol.
    nodes: Vec<Node>,
}

#[derive(Clone, Copy, Debug)]
enum Node {
    Leaf(HuffmanCode),
    Internal { zero: u32, one: u32 },
}

impl HuffmanTree {
    /// Read a Huffman tree from the bitstream; `alphabet` is the number of
    /// symbols in the alphabet.
    pub fn read(br: &mut BitReader<'_>, alphabet: usize) -> Result<Self> {
        let simple = br.read_bit()?;
        if simple == 1 {
            Self::read_simple(br, alphabet)
        } else {
            Self::read_normal(br, alphabet)
        }
    }

    fn read_simple(br: &mut BitReader<'_>, alphabet: usize) -> Result<Self> {
        let num_symbols = br.read_bit()? + 1; // 1 or 2
        let is_first_8bits = br.read_bit()?;
        let sym0 = br.read_bits(if is_first_8bits != 0 { 8 } else { 1 })? as HuffmanCode;
        if (sym0 as usize) >= alphabet.max(256) {
            return Err(Error::invalid("VP8L: simple huffman symbol out of range"));
        }
        if num_symbols == 1 {
            return Ok(Self {
                only_symbol: Some(sym0),
                nodes: vec![Node::Leaf(sym0)],
            });
        }
        let sym1 = br.read_bits(8)? as HuffmanCode;
        if (sym1 as usize) >= alphabet {
            return Err(Error::invalid("VP8L: simple huffman symbol out of range"));
        }
        // 1-bit: 0 -> sym0, 1 -> sym1.
        Ok(Self {
            only_symbol: None,
            nodes: vec![
                Node::Internal { zero: 1, one: 2 },
                Node::Leaf(sym0),
                Node::Leaf(sym1),
            ],
        })
    }

    fn read_normal(br: &mut BitReader<'_>, alphabet: usize) -> Result<Self> {
        // Read the code-length-tree's own lengths.
        let num_code_lengths = (br.read_bits(4)? + 4) as usize;
        if num_code_lengths > CODE_LENGTH_ORDER.len() {
            return Err(Error::invalid("VP8L: too many code-length lengths"));
        }
        let mut code_length_code_lengths = [0u8; 19];
        for i in 0..num_code_lengths {
            code_length_code_lengths[CODE_LENGTH_ORDER[i]] = br.read_bits(3)? as u8;
        }
        let meta_tree = build_from_lengths(&code_length_code_lengths)?;

        // Read the per-symbol code lengths, possibly truncated.
        let (max_symbol, use_length) = if br.read_bit()? == 1 {
            // Length-bound mode.
            let length_nbits = 2 + 2 * br.read_bits(3)? as usize;
            let max = 2 + br.read_bits(length_nbits as u8)? as usize;
            (max.min(alphabet), true)
        } else {
            (alphabet, false)
        };

        let mut code_lengths = vec![0u8; alphabet];
        let mut sym = 0usize;
        let mut prev_len = 8u8;
        let mut count = 0usize;
        while sym < alphabet {
            if use_length && count >= max_symbol {
                break;
            }
            let code = meta_tree.decode(br)?;
            match code {
                0..=15 => {
                    code_lengths[sym] = code as u8;
                    if code != 0 {
                        prev_len = code as u8;
                    }
                    sym += 1;
                    count += 1;
                }
                16 => {
                    let repeat = 3 + br.read_bits(2)? as usize;
                    if sym + repeat > alphabet {
                        return Err(Error::invalid("VP8L: huffman repeat past alphabet"));
                    }
                    for _ in 0..repeat {
                        code_lengths[sym] = prev_len;
                        sym += 1;
                    }
                }
                17 => {
                    let repeat = 3 + br.read_bits(3)? as usize;
                    if sym + repeat > alphabet {
                        return Err(Error::invalid("VP8L: huffman zero-run past alphabet"));
                    }
                    for _ in 0..repeat {
                        code_lengths[sym] = 0;
                        sym += 1;
                    }
                }
                18 => {
                    let repeat = 11 + br.read_bits(7)? as usize;
                    if sym + repeat > alphabet {
                        return Err(Error::invalid("VP8L: huffman long-zero-run past alphabet"));
                    }
                    for _ in 0..repeat {
                        code_lengths[sym] = 0;
                        sym += 1;
                    }
                }
                _ => return Err(Error::invalid("VP8L: bad code length code")),
            }
        }
        build_from_lengths(&code_lengths)
    }

    /// Decode a single symbol.
    pub fn decode(&self, br: &mut BitReader<'_>) -> Result<HuffmanCode> {
        if let Some(s) = self.only_symbol {
            return Ok(s);
        }
        let mut node = 0u32;
        loop {
            match self.nodes[node as usize] {
                Node::Leaf(s) => return Ok(s),
                Node::Internal { zero, one } => {
                    let b = br.read_bit()?;
                    node = if b == 0 { zero } else { one };
                }
            }
        }
    }
}

/// Build a canonical-Huffman tree from an array of code lengths (one per
/// symbol). Lengths of 0 mean "absent".
fn build_from_lengths(lengths: &[u8]) -> Result<HuffmanTree> {
    // Count symbols by length.
    let mut max_len = 0u8;
    let mut total_nonzero = 0usize;
    let mut lone_symbol: Option<u16> = None;
    for (i, &l) in lengths.iter().enumerate() {
        if l != 0 {
            total_nonzero += 1;
            if l > max_len {
                max_len = l;
            }
            lone_symbol = Some(i as u16);
        }
    }
    if total_nonzero == 0 {
        return Ok(HuffmanTree {
            only_symbol: Some(0),
            nodes: vec![Node::Leaf(0)],
        });
    }
    if total_nonzero == 1 {
        let s = lone_symbol.unwrap_or(0);
        return Ok(HuffmanTree {
            only_symbol: Some(s),
            nodes: vec![Node::Leaf(s)],
        });
    }

    // Canonical Huffman: assign codes in ascending (length, symbol) order.
    let mut bl_count = vec![0u32; (max_len + 1) as usize];
    for &l in lengths {
        if l > 0 {
            bl_count[l as usize] += 1;
        }
    }
    let mut next_code = vec![0u32; (max_len + 1) as usize];
    let mut code = 0u32;
    for bits in 1..=max_len as usize {
        code = (code + bl_count[bits - 1]) << 1;
        next_code[bits] = code;
    }

    // Insert each symbol into a flat tree. Walk the canonical code MSB-
    // first, allocating internal nodes on demand and then a final leaf.
    let mut nodes: Vec<Node> = vec![Node::Internal { zero: 0, one: 0 }];
    for (sym, &len) in lengths.iter().enumerate() {
        if len == 0 {
            continue;
        }
        let code_val = next_code[len as usize];
        next_code[len as usize] += 1;
        let mut node = 0u32;
        for b in (0..len).rev() {
            let bit = (code_val >> b) & 1;
            if b == 0 {
                let leaf_idx = nodes.len() as u32;
                nodes.push(Node::Leaf(sym as u16));
                match &mut nodes[node as usize] {
                    Node::Internal { zero, one } => {
                        if bit == 0 {
                            *zero = leaf_idx;
                        } else {
                            *one = leaf_idx;
                        }
                    }
                    Node::Leaf(_) => {
                        return Err(Error::invalid(
                            "VP8L: canonical Huffman length table self-collides",
                        ))
                    }
                }
            } else {
                let child = match nodes[node as usize] {
                    Node::Internal { zero, one } => {
                        if bit == 0 {
                            zero
                        } else {
                            one
                        }
                    }
                    Node::Leaf(_) => {
                        return Err(Error::invalid(
                            "VP8L: canonical Huffman length table self-collides",
                        ))
                    }
                };
                let next = if child == 0 {
                    let new_idx = nodes.len() as u32;
                    nodes.push(Node::Internal { zero: 0, one: 0 });
                    match &mut nodes[node as usize] {
                        Node::Internal { zero, one } => {
                            if bit == 0 {
                                *zero = new_idx;
                            } else {
                                *one = new_idx;
                            }
                        }
                        _ => unreachable!(),
                    }
                    new_idx
                } else {
                    child
                };
                node = next;
            }
        }
    }
    Ok(HuffmanTree {
        only_symbol: None,
        nodes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_two_symbols() {
        // Two symbols of length 1 each: 0 -> sym0, 1 -> sym1.
        let tree = build_from_lengths(&[1, 1]).unwrap();
        let buf = [0b0000_0010u8];
        let mut br = BitReader::new(&buf);
        assert_eq!(tree.decode(&mut br).unwrap(), 0);
        assert_eq!(tree.decode(&mut br).unwrap(), 1);
    }

    #[test]
    fn simple_one_symbol() {
        // Simple encoding: bit0=1 (simple), bit1=0 (num_symbols=1),
        // bit2=0 (is_first_8bits=0), bit3=1 (sym0 in 1-bit field).
        let buf = [0b0000_1001u8];
        let mut br = BitReader::new(&buf);
        let tree = HuffmanTree::read(&mut br, 256).unwrap();
        // Single-symbol tree — any read returns 1.
        assert_eq!(tree.decode(&mut br).unwrap(), 1);
    }
}
