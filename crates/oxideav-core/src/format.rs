//! Media-type and sample/pixel format enumerations.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MediaType {
    Audio,
    Video,
    Subtitle,
    Data,
    Unknown,
}

/// Audio sample format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SampleFormat {
    /// Unsigned 8-bit, interleaved.
    U8,
    /// Signed 8-bit, interleaved. Native format of Amiga 8SVX and MOD samples.
    S8,
    /// Signed 16-bit little-endian, interleaved.
    S16,
    /// Signed 24-bit packed (3 bytes/sample) little-endian, interleaved.
    S24,
    /// Signed 32-bit little-endian, interleaved.
    S32,
    /// 32-bit IEEE float, interleaved.
    F32,
    /// 64-bit IEEE float, interleaved.
    F64,
    /// Planar variants — one plane per channel.
    U8P,
    S16P,
    S32P,
    F32P,
    F64P,
}

impl SampleFormat {
    pub fn is_planar(&self) -> bool {
        matches!(
            self,
            Self::U8P | Self::S16P | Self::S32P | Self::F32P | Self::F64P
        )
    }

    /// Bytes per sample *per channel*.
    pub fn bytes_per_sample(&self) -> usize {
        match self {
            Self::U8 | Self::U8P | Self::S8 => 1,
            Self::S16 | Self::S16P => 2,
            Self::S24 => 3,
            Self::S32 | Self::S32P | Self::F32 | Self::F32P => 4,
            Self::F64 | Self::F64P => 8,
        }
    }

    pub fn is_float(&self) -> bool {
        matches!(self, Self::F32 | Self::F64 | Self::F32P | Self::F64P)
    }
}

/// Video pixel format.
///
/// The first six variants (`Yuv420P` through `Gray8`) are the original
/// formats produced by the early codec crates. Everything beyond that is
/// additional surface handled by `oxideav-pixfmt` and the still-image
/// codecs (PNG, GIF, still-JPEG). The enum is `#[non_exhaustive]` so new
/// variants can land without breaking downstream crates — consumers that
/// match must include a wildcard arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PixelFormat {
    /// 8-bit YUV 4:2:0, planar (Y, U, V).
    Yuv420P,
    /// 8-bit YUV 4:2:2, planar.
    Yuv422P,
    /// 8-bit YUV 4:4:4, planar.
    Yuv444P,
    /// Packed 8-bit RGB, 3 bytes/pixel.
    Rgb24,
    /// Packed 8-bit RGBA, 4 bytes/pixel.
    Rgba,
    /// Packed 8-bit grayscale.
    Gray8,

    // --- Palette ---
    /// 8-bit palette indices — companion palette carried out of band.
    Pal8,

    // --- Packed RGB/BGR swizzles ---
    /// Packed 8-bit BGR, 3 bytes/pixel.
    Bgr24,
    /// Packed 8-bit BGRA, 4 bytes/pixel.
    Bgra,
    /// Packed 8-bit ARGB, 4 bytes/pixel (alpha first).
    Argb,
    /// Packed 8-bit ABGR, 4 bytes/pixel.
    Abgr,

    // --- Deeper packed RGB ---
    /// Packed 16-bit-per-channel RGB, little-endian, 6 bytes/pixel.
    Rgb48Le,
    /// Packed 16-bit-per-channel RGBA, little-endian, 8 bytes/pixel.
    Rgba64Le,

    // --- Grayscale deeper / partial bit depths ---
    /// 16-bit little-endian grayscale.
    Gray16Le,
    /// 10-bit grayscale in a 16-bit little-endian word.
    Gray10Le,
    /// 12-bit grayscale in a 16-bit little-endian word.
    Gray12Le,

    // --- Higher-precision YUV ---
    /// 10-bit YUV 4:2:0 planar, little-endian 16-bit storage.
    Yuv420P10Le,
    /// 10-bit YUV 4:2:2 planar, little-endian 16-bit storage.
    Yuv422P10Le,
    /// 10-bit YUV 4:4:4 planar, little-endian 16-bit storage.
    Yuv444P10Le,
    /// 12-bit YUV 4:2:0 planar, little-endian 16-bit storage.
    Yuv420P12Le,

    // --- Full-range ("J") YUV ---
    /// JPEG/full-range YUV 4:2:0 planar.
    YuvJ420P,
    /// JPEG/full-range YUV 4:2:2 planar.
    YuvJ422P,
    /// JPEG/full-range YUV 4:4:4 planar.
    YuvJ444P,

    // --- Semi-planar YUV ---
    /// YUV 4:2:0, planar Y + interleaved UV (NV12).
    Nv12,
    /// YUV 4:2:0, planar Y + interleaved VU (NV21).
    Nv21,

    // --- Gray + alpha / YUV + alpha ---
    /// Packed grayscale + alpha, 2 bytes/pixel (Y, A).
    Ya8,
    /// Yuv420P with an additional full-resolution alpha plane.
    Yuva420P,

    // --- Mono (1 bit per pixel) ---
    /// 1 bit per pixel, packed MSB-first, 0 = black.
    MonoBlack,
    /// 1 bit per pixel, packed MSB-first, 0 = white.
    MonoWhite,

    // --- Interleaved YUV 4:2:2 ---
    /// Packed 4:2:2, byte order Y0 U0 Y1 V0.
    Yuyv422,
    /// Packed 4:2:2, byte order U0 Y0 V0 Y1.
    Uyvy422,
}

impl PixelFormat {
    /// True if this format stores its components in separate planes.
    pub fn is_planar(&self) -> bool {
        matches!(
            self,
            Self::Yuv420P
                | Self::Yuv422P
                | Self::Yuv444P
                | Self::Yuv420P10Le
                | Self::Yuv422P10Le
                | Self::Yuv444P10Le
                | Self::Yuv420P12Le
                | Self::YuvJ420P
                | Self::YuvJ422P
                | Self::YuvJ444P
                | Self::Nv12
                | Self::Nv21
                | Self::Yuva420P
        )
    }

    /// True if the format is a palette index format (`Pal8`).
    pub fn is_palette(&self) -> bool {
        matches!(self, Self::Pal8)
    }

    /// True if this format carries an alpha channel.
    pub fn has_alpha(&self) -> bool {
        matches!(
            self,
            Self::Rgba
                | Self::Bgra
                | Self::Argb
                | Self::Abgr
                | Self::Rgba64Le
                | Self::Ya8
                | Self::Yuva420P
        )
    }

    /// Number of planes in the stored layout. Packed and palette formats
    /// return 1; NV12/NV21 return 2; planar YUV without alpha returns 3;
    /// YuvA variants return 4.
    pub fn plane_count(&self) -> usize {
        match self {
            Self::Nv12 | Self::Nv21 => 2,
            Self::Yuv420P
            | Self::Yuv422P
            | Self::Yuv444P
            | Self::Yuv420P10Le
            | Self::Yuv422P10Le
            | Self::Yuv444P10Le
            | Self::Yuv420P12Le
            | Self::YuvJ420P
            | Self::YuvJ422P
            | Self::YuvJ444P => 3,
            Self::Yuva420P => 4,
            _ => 1,
        }
    }

    /// Rough bits-per-pixel estimate, useful for buffer sizing. Not exact
    /// for chroma-subsampled YUV — intended for worst-case preallocation
    /// rather than wire-accurate accounting.
    pub fn bits_per_pixel_approx(&self) -> u32 {
        match self {
            Self::MonoBlack | Self::MonoWhite => 1,
            Self::Gray8 | Self::Pal8 => 8,
            Self::Ya8 => 16,
            Self::Gray16Le | Self::Gray10Le | Self::Gray12Le => 16,
            Self::Rgb24 | Self::Bgr24 => 24,
            Self::Rgba | Self::Bgra | Self::Argb | Self::Abgr => 32,
            Self::Rgb48Le => 48,
            Self::Rgba64Le => 64,
            Self::Yuyv422 | Self::Uyvy422 => 16,
            // Planar YUV: 4:2:0 ≈ 12, 4:2:2 ≈ 16, 4:4:4 ≈ 24
            // 10/12-bit variants double the byte count but we report the
            // packed-bits-per-pixel estimate for a uniform heuristic.
            Self::Yuv420P | Self::YuvJ420P | Self::Nv12 | Self::Nv21 => 12,
            Self::Yuv422P | Self::YuvJ422P => 16,
            Self::Yuv444P | Self::YuvJ444P => 24,
            Self::Yuv420P10Le | Self::Yuv420P12Le => 24,
            Self::Yuv422P10Le => 32,
            Self::Yuv444P10Le => 48,
            Self::Yuva420P => 20,
        }
    }
}
