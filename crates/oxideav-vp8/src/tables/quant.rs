//! Quantiser step tables — RFC 6386 §14.1 (`dc_qlookup` and `ac_qlookup`).
//!
//! Indexed by the 7-bit quantiser index (0..=127). The DC table further
//! clamps Y2 DC steps to 132 — see `y2_dc_step`.

pub const DC_Q_LOOKUP: [u16; 128] = [
    4, 5, 6, 7, 8, 9, 10, 10, 11, 12, 13, 14, 15, 16, 17, 17, 18, 19, 20, 20, 21, 21, 22, 22, 23,
    23, 24, 25, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 37, 38, 39, 40, 41, 42, 43, 44,
    45, 46, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67,
    68, 69, 70, 71, 72, 73, 74, 75, 76, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 91,
    93, 95, 96, 98, 100, 101, 102, 104, 106, 108, 110, 112, 114, 116, 118, 122, 124, 126, 128, 130,
    132, 134, 136, 138, 140, 143, 145, 148, 151, 154, 157,
];

pub const AC_Q_LOOKUP: [u16; 128] = [
    4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28,
    29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52,
    53, 54, 55, 56, 57, 58, 60, 62, 64, 66, 68, 70, 72, 74, 76, 78, 80, 82, 84, 86, 88, 90, 92, 94,
    96, 98, 100, 102, 104, 106, 108, 110, 112, 114, 116, 119, 122, 125, 128, 131, 134, 137, 140,
    143, 146, 149, 152, 155, 158, 161, 164, 167, 170, 173, 177, 181, 185, 189, 193, 197, 201, 205,
    209, 213, 217, 221, 225, 229, 234, 239, 245, 249, 254, 259, 264, 269, 274, 279, 284,
];

/// Clamp a quantiser index into the valid 0..=127 range.
pub fn clamp_qindex(q: i32) -> usize {
    q.clamp(0, 127) as usize
}

/// Y2 DC step has a maximum of 132 (RFC 6386 §14.1).
pub fn y2_dc_step(qindex: i32) -> i32 {
    let v = DC_Q_LOOKUP[clamp_qindex(qindex)] as i32 * 2;
    v.min(132 * 2)
}

/// Y2 AC step is `ac_qlookup * 155 / 100`, clamped to a minimum of 8.
pub fn y2_ac_step(qindex: i32) -> i32 {
    let v = AC_Q_LOOKUP[clamp_qindex(qindex)] as i32 * 155 / 100;
    v.max(8)
}

/// UV DC step is `dc_qlookup` clamped to ≤ 132.
pub fn uv_dc_step(qindex: i32) -> i32 {
    let v = DC_Q_LOOKUP[clamp_qindex(qindex)] as i32;
    v.min(132)
}

/// UV AC step is `ac_qlookup`.
pub fn uv_ac_step(qindex: i32) -> i32 {
    AC_Q_LOOKUP[clamp_qindex(qindex)] as i32
}

/// Y DC step is `dc_qlookup`.
pub fn y_dc_step(qindex: i32) -> i32 {
    DC_Q_LOOKUP[clamp_qindex(qindex)] as i32
}

/// Y AC step is `ac_qlookup`.
pub fn y_ac_step(qindex: i32) -> i32 {
    AC_Q_LOOKUP[clamp_qindex(qindex)] as i32
}
