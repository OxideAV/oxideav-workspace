//! State tables, context modelling and default quant tables (RFC 9043 §3.8).
//!
//! We use FFmpeg's default 8-bit quant11 table which yields 666 contexts —
//! this is the same table that a FFmpeg-produced FFV1 file will emit when
//! `-context 0` (default) is used.

/// FFmpeg's `quant11[256]` — a signed lookup from 0..=255 (mapping via `u8`
/// cast on the signed residual byte) to a 5-level magnitude label.
#[rustfmt::skip]
pub const QUANT11: [i8; 256] = [
     0,  1,  2,  2,  2,  3,  3,  3,  3,  3,  3,  3,  4,  4,  4,  4,
     4,  4,  4,  4,  4,  4,  4,  4,  4,  4,  4,  4,  4,  4,  4,  4,
     4,  4,  4,  5,  5,  5,  5,  5,  5,  5,  5,  5,  5,  5,  5,  5,
     5,  5,  5,  5,  5,  5,  5,  5,  5,  5,  5,  5,  5,  5,  5,  5,
     5,  5,  5,  5,  5,  5,  5,  5,  5,  5,  5,  5,  5,  5,  5,  5,
     5,  5,  5,  5,  5,  5,  5,  5,  5,  5,  5,  5,  5,  5,  5,  5,
     5,  5,  5,  5,  5,  5,  5,  5,  5,  5,  5,  5,  5,  5,  5,  5,
     5,  5,  5,  5,  5,  5,  5,  5,  5,  5,  5,  5,  5,  5,  5,  5,
    -5, -5, -5, -5, -5, -5, -5, -5, -5, -5, -5, -5, -5, -5, -5, -5,
    -5, -5, -5, -5, -5, -5, -5, -5, -5, -5, -5, -5, -5, -5, -5, -5,
    -5, -5, -5, -5, -5, -5, -5, -5, -5, -5, -5, -5, -5, -5, -5, -5,
    -5, -5, -5, -5, -5, -5, -5, -5, -5, -5, -5, -5, -5, -5, -5, -5,
    -5, -5, -5, -5, -5, -5, -5, -5, -5, -5, -5, -5, -5, -5, -5, -5,
    -5, -5, -5, -5, -5, -5, -5, -5, -5, -5, -5, -5, -5, -5, -5, -5,
    -4, -4, -4, -4, -4, -4, -4, -4, -4, -4, -4, -4, -4, -4, -4, -4,
    -4, -4, -4, -4, -4, -3, -3, -3, -3, -3, -3, -3, -2, -2, -2, -1,
];

/// Five sub-tables of a quant-table set. Each is 256 signed-16 values that
/// map a sample-difference byte index to a contribution to the context.
pub type QuantTables = [[i16; 256]; 5];

/// Build FFmpeg's default 8-bit quant-table set: scales 1, 11, 121 for the
/// first three tables; zero for the last two (unused = single context).
pub fn default_quant_tables() -> QuantTables {
    let mut tables = [[0i16; 256]; 5];
    for i in 0..256 {
        let q = QUANT11[i] as i16;
        tables[0][i] = q;
        tables[1][i] = 11 * q;
        tables[2][i] = 11 * 11 * q;
        // tables[3] and tables[4] stay zero.
    }
    tables
}

/// Context count for a set of quant tables, per RFC 9043 §3.8.
/// `context_count = ceil(product_of_(2*len_count[i] - 1) / 2)` where
/// `len_count` is the count of distinct values taken by each table (including
/// zero). For FFmpeg's 8-bit default this evaluates to 666.
pub fn context_count(tables: &QuantTables) -> usize {
    let mut scale: i64 = 1;
    for tbl in tables.iter() {
        // `len_count` is the count of distinct non-negative values taken by
        // the table (including zero). For FFmpeg's 8-bit default this is 6
        // on tables 0..2 and 1 on tables 3..4.
        let mut vals: Vec<i16> = tbl.iter().copied().filter(|&v| v >= 0).collect();
        vals.sort_unstable();
        vals.dedup();
        let len_count = vals.len() as i64;
        scale *= 2 * len_count - 1;
    }
    ((scale + 1) / 2) as usize
}

/// Per-plane slice state: `[context_count][32]` range-coder state bytes.
pub struct PlaneState {
    pub states: Vec<[u8; 32]>,
}

impl PlaneState {
    pub fn new(context_count: usize) -> Self {
        Self {
            states: vec![[128u8; 32]; context_count],
        }
    }

    pub fn reset(&mut self) {
        for s in &mut self.states {
            *s = [128u8; 32];
        }
    }
}

/// Apply the five-neighbour FFV1 context formula given the pixel neighbourhood
/// (L, l, t, tl, T, tr) (see §3.8).
///
/// Each sub-table handles one specific sample difference, in this order
/// (matching `ffv1_template.c`'s `get_context`):
/// * `tables[0]` → l - tl
/// * `tables[1]` → tl - t
/// * `tables[2]` → t - tr
/// * `tables[3]` → big_l (LL) - l
/// * `tables[4]` → big_t (TT) - t
///
/// Indices into each table are taken as `delta as u8` (the low 8 bits), so
/// negative differences wrap into the upper half of the 256-entry table,
/// which is pre-populated with the signed-negation mirror.
#[inline]
#[allow(clippy::too_many_arguments)]
pub fn compute_context(
    tables: &QuantTables,
    big_l: i32,
    l: i32,
    t: i32,
    tl: i32,
    big_t: i32,
    tr: i32,
) -> i32 {
    tables[0][(l.wrapping_sub(tl) as u8) as usize] as i32
        + tables[1][(tl.wrapping_sub(t) as u8) as usize] as i32
        + tables[2][(t.wrapping_sub(tr) as u8) as usize] as i32
        + tables[3][(big_l.wrapping_sub(l) as u8) as usize] as i32
        + tables[4][(big_t.wrapping_sub(t) as u8) as usize] as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_context_count_is_666() {
        let tables = default_quant_tables();
        // quant11 ranges over {-5..=5}, len_count=6, factor=11. Three active
        // tables (11*11*11), two inactive (len_count=1, factor=1). Total
        // scale = 11*11*11 = 1331. context_count = (1331+1)/2 = 666.
        assert_eq!(context_count(&tables), 666);
    }

    #[test]
    fn quant11_symmetric() {
        assert_eq!(QUANT11[0], 0);
        assert_eq!(QUANT11[1], 1);
        // Index 255 = residual -1 as u8, which maps to -1.
        assert_eq!(QUANT11[255], -1);
        // Index 128 = residual -128 as u8, which maps to a negative boundary.
        assert_eq!(QUANT11[128], -5);
    }
}
