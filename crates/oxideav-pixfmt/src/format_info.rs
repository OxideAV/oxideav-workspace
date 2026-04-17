//! Per-format metadata: subsampling, planes, and bit depth.
//!
//! Callers that need to allocate or walk plane strides for a given
//! [`PixelFormat`] can look the format up here instead of open-coding
//! the decision tree.

use oxideav_core::PixelFormat;

/// Compact description of a pixel format's layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormatInfo {
    /// Component bit depth (before packing). 8 for Rgb24, 16 for Gray16Le,
    /// 10 for Yuv420P10Le, …
    pub bit_depth: u8,
    /// Number of distinct planes — matches [`PixelFormat::plane_count`].
    pub planes: u8,
    /// Chroma horizontal subsampling factor (1 = no subsample). 2 for
    /// 4:2:x, 1 for 4:4:4 / non-YUV.
    pub chroma_w_sub: u8,
    /// Chroma vertical subsampling factor (1 = no subsample). 2 for
    /// 4:2:0, 1 for 4:2:2 / 4:4:4.
    pub chroma_h_sub: u8,
    /// True for any planar YUV-style layout.
    pub is_planar: bool,
    /// True when alpha is carried as part of the format (explicit or
    /// through a separate plane).
    pub has_alpha: bool,
    /// True for `Pal8`.
    pub is_palette: bool,
}

impl FormatInfo {
    /// Look up static metadata for `fmt`.
    pub const fn of(fmt: PixelFormat) -> Self {
        use PixelFormat as P;
        match fmt {
            // 8-bit YUV planar
            P::Yuv420P | P::YuvJ420P => Self::yuv(8, 2, 2),
            P::Yuv422P | P::YuvJ422P => Self::yuv(8, 2, 1),
            P::Yuv444P | P::YuvJ444P => Self::yuv(8, 1, 1),
            P::Yuv420P10Le => Self::yuv(10, 2, 2),
            P::Yuv422P10Le => Self::yuv(10, 2, 1),
            P::Yuv444P10Le => Self::yuv(10, 1, 1),
            P::Yuv420P12Le => Self::yuv(12, 2, 2),
            P::Yuva420P => Self {
                bit_depth: 8,
                planes: 4,
                chroma_w_sub: 2,
                chroma_h_sub: 2,
                is_planar: true,
                has_alpha: true,
                is_palette: false,
            },
            P::Nv12 | P::Nv21 => Self {
                bit_depth: 8,
                planes: 2,
                chroma_w_sub: 2,
                chroma_h_sub: 2,
                is_planar: true,
                has_alpha: false,
                is_palette: false,
            },
            // Packed 4:2:2
            P::Yuyv422 | P::Uyvy422 => Self::packed(8, false),
            // RGB family
            P::Rgb24 | P::Bgr24 => Self::packed(8, false),
            P::Rgba | P::Bgra | P::Argb | P::Abgr => Self::packed(8, true),
            P::Rgb48Le => Self::packed(16, false),
            P::Rgba64Le => Self::packed(16, true),
            // Gray
            P::Gray8 => Self::packed(8, false),
            P::Gray16Le => Self::packed(16, false),
            P::Gray10Le => Self::packed(10, false),
            P::Gray12Le => Self::packed(12, false),
            P::Ya8 => Self::packed(8, true),
            P::MonoBlack | P::MonoWhite => Self {
                bit_depth: 1,
                planes: 1,
                chroma_w_sub: 1,
                chroma_h_sub: 1,
                is_planar: false,
                has_alpha: false,
                is_palette: false,
            },
            // Palette
            P::Pal8 => Self {
                bit_depth: 8,
                planes: 1,
                chroma_w_sub: 1,
                chroma_h_sub: 1,
                is_planar: false,
                has_alpha: false,
                is_palette: true,
            },
            // The enum is `#[non_exhaustive]`; future variants fall back
            // to a conservative "single packed 8-bit plane" descriptor.
            _ => Self::packed(8, false),
        }
    }

    const fn yuv(bits: u8, wsub: u8, hsub: u8) -> Self {
        Self {
            bit_depth: bits,
            planes: 3,
            chroma_w_sub: wsub,
            chroma_h_sub: hsub,
            is_planar: true,
            has_alpha: false,
            is_palette: false,
        }
    }

    const fn packed(bits: u8, alpha: bool) -> Self {
        Self {
            bit_depth: bits,
            planes: 1,
            chroma_w_sub: 1,
            chroma_h_sub: 1,
            is_planar: false,
            has_alpha: alpha,
            is_palette: false,
        }
    }
}
