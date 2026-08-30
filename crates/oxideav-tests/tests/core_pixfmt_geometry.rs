//! core 0.1.35 pixel-format families as consumers see them: the 4:4:0
//! planar YUV ladder (full-width, half-height chroma) and the
//! scene-referred F32 family, pinned through the plane-geometry helpers
//! a decoder uses to size its output (`plane_dimensions`,
//! `plane_row_bytes`, `plane_size_bytes`, `frame_size_bytes`).

use oxideav_core::PixelFormat;

const YUV440: [(PixelFormat, usize); 4] = [
    (PixelFormat::Yuv440P, 1),
    (PixelFormat::Yuv440P10Le, 2),
    (PixelFormat::Yuv440P12Le, 2),
    (PixelFormat::Yuv440P16Le, 2),
];

/// (format, planes, bytes per sample position per plane)
const F32: [(PixelFormat, usize, usize); 5] = [
    (PixelFormat::GrayF32Le, 1, 4),
    (PixelFormat::RgbF32Le, 1, 12),
    (PixelFormat::RgbaF32Le, 1, 16),
    (PixelFormat::GbrpF32Le, 3, 4),
    (PixelFormat::GbrapF32Le, 4, 4),
];

#[test]
fn yuv440_ladder_has_half_height_chroma() {
    for (fmt, bps) in YUV440 {
        assert_eq!(fmt.chroma_subsampling(), Some((0, 1)), "{fmt:?}");
        assert_eq!(fmt.plane_count(), 3, "{fmt:?}");
        assert!(!fmt.is_float(), "{fmt:?}");
        // Odd height: chroma rows use ceiling division; width is untouched.
        let (w, h) = (641, 481);
        assert_eq!(fmt.plane_dimensions(0, w, h), Some((641, 481)), "{fmt:?}");
        assert_eq!(fmt.plane_dimensions(1, w, h), Some((641, 241)), "{fmt:?}");
        assert_eq!(fmt.plane_dimensions(2, w, h), Some((641, 241)), "{fmt:?}");
        assert_eq!(fmt.plane_dimensions(3, w, h), None, "{fmt:?}: no 4th plane");
        for plane in 0..3 {
            assert_eq!(
                fmt.plane_row_bytes(plane, w),
                Some(641 * bps),
                "{fmt:?}/{plane}"
            );
        }
        assert_eq!(
            fmt.plane_size_bytes(0, w, h),
            Some(641 * 481 * bps),
            "{fmt:?}"
        );
        assert_eq!(
            fmt.plane_size_bytes(1, w, h),
            Some(641 * 241 * bps),
            "{fmt:?}"
        );
        assert_eq!(
            fmt.frame_size_bytes(w, h),
            Some(641 * (481 + 241 + 241) * bps),
            "{fmt:?}"
        );
    }
    // 4:4:0 sits between 4:2:2 and 4:4:4 in chroma density: same
    // chroma sample count as 4:2:2 at even geometry, laid out the other way.
    assert_eq!(
        PixelFormat::Yuv440P.frame_size_bytes(640, 480),
        PixelFormat::Yuv422P.frame_size_bytes(640, 480)
    );
    assert_eq!(
        PixelFormat::Yuv440P.plane_dimensions(1, 640, 480),
        Some((640, 240))
    );
    assert_eq!(
        PixelFormat::Yuv422P.plane_dimensions(1, 640, 480),
        Some((320, 480))
    );
}

#[test]
fn f32_family_strides_are_four_bytes_per_component() {
    for (fmt, planes, bpp) in F32 {
        assert!(fmt.is_float(), "{fmt:?}");
        assert_eq!(fmt.chroma_subsampling(), None, "{fmt:?}: no chroma grid");
        assert_eq!(fmt.plane_count(), planes, "{fmt:?}");
        let (w, h) = (1_000, 7);
        for plane in 0..planes {
            assert_eq!(
                fmt.plane_dimensions(plane, w, h),
                Some((w, h)),
                "{fmt:?}/{plane}"
            );
            assert_eq!(
                fmt.plane_row_bytes(plane, w),
                Some(1_000 * bpp),
                "{fmt:?}/{plane}"
            );
            assert_eq!(
                fmt.plane_size_bytes(plane, w, h),
                Some(1_000 * 7 * bpp),
                "{fmt:?}/{plane}"
            );
        }
        assert_eq!(
            fmt.plane_row_bytes(planes, w),
            None,
            "{fmt:?}: past last plane"
        );
        assert_eq!(
            fmt.frame_size_bytes(w, h),
            Some(1_000 * 7 * bpp * planes),
            "{fmt:?}"
        );
        // F32 rows are exactly 4× their 8-bit-per-component counterparts.
        assert_eq!(fmt.plane_row_bytes(0, w).unwrap() % 4, 0, "{fmt:?}");
    }
    assert!(PixelFormat::RgbaF32Le.has_alpha());
    assert!(PixelFormat::GbrapF32Le.has_alpha());
    assert!(!PixelFormat::RgbF32Le.has_alpha());
    assert!(!PixelFormat::GbrpF32Le.has_alpha());
    // Packed RGB F32 and planar GBR F32 carry the same bytes per frame.
    assert_eq!(
        PixelFormat::RgbF32Le.frame_size_bytes(33, 17),
        PixelFormat::GbrpF32Le.frame_size_bytes(33, 17)
    );
}

/// The sizing trio is checked arithmetic: absurd geometry yields
/// `None`, never a wrapped size a decoder could allocate against.
#[test]
fn geometry_helpers_refuse_overflow() {
    for fmt in [
        PixelFormat::Yuv440P16Le,
        PixelFormat::RgbaF32Le,
        PixelFormat::GbrapF32Le,
    ] {
        assert_eq!(fmt.frame_size_bytes(u32::MAX, u32::MAX), None, "{fmt:?}");
        assert_eq!(fmt.plane_size_bytes(0, u32::MAX, u32::MAX), None, "{fmt:?}");
        assert!(
            fmt.plane_row_bytes(0, u32::MAX).is_some(),
            "{fmt:?}: one row fits"
        );
    }
}
