# The OxideAV image-crate API contract

**Status: ACCEPTED 2026-10-03 (revision 3).** Every `oxideav-<format>` crate that decodes or encodes still pictures
follows this contract, and a new image-format crate implements it
before its first release.

The raw surface is deliberately small: **RGB8 and RGBA8 in and out**,
plus the format's native layout for those who want it. Every other
conversion (16-bit, float, YCbCr re-layouts, resizing) is the job of
`oxideav-image` with `oxideav-pixfmt` underneath, not of the format
crates.

## Two layers

Video and audio codec crates in this workspace are framework-only:
they register into the `oxideav-core` registry and expose
`make_decoder` / `make_encoder` factories; they never offer a "decode
this file" document API. Image formats are the deliberate exception,
with two layers:

1. **The per-crate standalone layer.** `oxideav-png`, `oxideav-mjpeg`,
   `oxideav-heif`, … each expose the same small vocabulary at their
   root, usable with `default-features = false` and no `oxideav-core`.
   It returns pixels as plain `Vec<u8>` plus width, height and a
   pixel-format tag, which is what Rust image consumers expect from the
   `png`, `jpeg-decoder` or `image` crates. No shared types crate is
   needed; consistency comes from every crate using the same names and
   shapes.
2. **The gateway: `oxideav-image`.** A framework-side crate (depends on
   `oxideav-core`) that opens any image file through the registry,
   whatever image crates the application registered (normally all of
   them via `oxideav-meta`), and hands back frames with image-oriented
   helpers: raw bytes, RGBA8, metadata, save-as-another-format. It is
   how an application that already uses OxideAV reads "an image"
   without naming the format.

## Layer 1 — the per-crate standalone API

### In one screen

```rust
// Cargo.toml: oxideav-png = { version = "…", default-features = false }

let bytes = std::fs::read("in.png")?;
if oxideav_png::probe(&bytes) {
    let info  = oxideav_png::info(&bytes)?;         // header only: width, height, format, frames
    let img   = oxideav_png::decode(&bytes)?;       // PngImage, native layout
    let rgba: Vec<u8> = img.to_rgba8();             // tightly packed RGBA, 4 * width bytes per row
    let (w, h) = (img.width(), img.height());

    let opts = oxideav_png::EncodeOptions::default().with_level(2);
    let out: Vec<u8> = oxideav_png::encode_rgba8(w, h, &rgba, &opts)?;
    std::fs::write("out.png", out)?;
}
```

A consumer who only wants bytes never touches a planar type, a colour
matrix or the framework.

### Root items, with these exact names

| Item | Signature | Notes |
|---|---|---|
| `probe` | `fn probe(bytes: &[u8]) -> bool` | Signature sniff; no allocation, no panic, `false` on short input. |
| `info` | `fn info(bytes: &[u8]) -> Result<ImageInfo, Error>` | Header only: dimensions, native `PixelFormat`, `frames`, alpha, metadata presence. |
| `decode` | `fn decode(bytes: &[u8]) -> Result<XxxImage, Error>` | Primary / first image, native layout, colour + metadata filled. |
| `decode_with` | `fn decode_with(bytes: &[u8], opts: &DecodeOptions) -> Result<XxxImage, Error>` | Limits (max dimensions / pixels / bytes) and strictness. `decode` uses `DecodeOptions::default()`. |
| `decode_rgb8` / `decode_rgba8` | `fn decode_rgb8(bytes: &[u8]) -> Result<RgbImage, Error>`, `fn decode_rgba8(bytes: &[u8]) -> Result<RgbaImage, Error>` | The one-call raw paths: `RgbImage { width, height, data: Vec<u8> }` (3 bytes/pixel) and `RgbaImage` (4 bytes/pixel), tightly packed, row-major. Alpha is dropped / set opaque as appropriate. |
| `decode_all` | `fn decode_all(bytes: &[u8]) -> Result<Vec<Frame>, Error>` | Only for formats with several images (APNG, GIF, WebP, HEIF sequences and bursts, TIFF pages, ICO entries). `Frame { image: XxxImage, delay: Option<Duration>, … }`. |
| `decode_from` | `fn decode_from<R: Read>(r: R) -> Result<XxxImage, Error>` | Reads to end; genuinely streaming decoders may specialise. |
| `encode` | `fn encode(image: &XxxImage, opts: &EncodeOptions) -> Result<Vec<u8>, Error>` | Writes the image as given; `Error::Unsupported` for a layout the format cannot carry, never a silent conversion. |
| `encode_rgb8` / `encode_rgba8` | `fn encode_rgb8(width: u32, height: u32, rgb: &[u8], opts: &EncodeOptions) -> Result<Vec<u8>, Error>`, `fn encode_rgba8(width, height, rgba: &[u8], opts) -> …` | The one-call raw paths; the crate picks its natural native layout (PNG: `Rgb24` / `Rgba`; JPEG/HEIF/AVIF/WebP-lossy: 4:2:0 with the format's default colour signalling, alpha as the format's alpha mechanism or `Error::Unsupported` if it has none — documented per crate). |
| `encode_to` | `fn encode_to<W: Write>(image: &XxxImage, opts: &EncodeOptions, w: W) -> Result<(), Error>` | Streaming variant. |
| `XxxImage` | struct, `#[non_exhaustive]` | See below. |
| `RgbImage`, `RgbaImage` | `struct RgbImage { pub width: u32, pub height: u32, pub data: Vec<u8> }`, same for `RgbaImage` | Tightly packed, row-major, 3 / 4 bytes per pixel. Same definition in every crate (a copy, not a shared dependency), with `into_raw() -> Vec<u8>` and `as_bytes()`. |
| `PixelFormat` | `pub enum XxxPixelFormat` + `pub type PixelFormat = XxxPixelFormat` | Variant names mirror `oxideav_core::PixelFormat` exactly (`Gray8`, `Rgb24`, `Rgba`, `Rgb48Le`, `Yuv420P`, `YuvJ420P`, `Yuv420P10Le`, `Gbrp`, `Rgb96F`, …). Only the variants the format can produce or accept. |
| `EncodeOptions` | struct, `#[non_exhaustive]`, `Default`, `with_*` | Quality / level / filter / chroma layout / which metadata to embed. One struct; behaviour variants are fields, never function suffixes. |
| `DecodeOptions` | struct, `#[non_exhaustive]`, `Default`, `with_*` | `max_width`, `max_height`, `max_pixels`, `max_bytes`, `strict`, plus format extras. |
| `ImageInfo` | struct, `#[non_exhaustive]` | `width`, `height`, `format`, `frames`, `has_alpha`, `color`, `has_icc`, `has_exif`, `has_xmp`. |
| `Error` | `pub enum XxxError` + `pub type Error = XxxError` | `std::error::Error` + `Display`; variants at least `InvalidData`, `Unsupported`, `LimitExceeded`, `Io`. Never a bare `enum Error`. |

The format name is in the crate path (`oxideav_png::decode`), never in
the function name (`decode_png`). Format-specific depth stays under its
own names (`oxideav_heif::HeifFile`, `oxideav_png::PngMetadata`,
`oxideav_tiff::Page`, …); the contract is the common floor, not a
ceiling.

### The image type

```rust
#[non_exhaustive]
pub struct PngImage {           // same shape in every crate
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,     // native layout
    pub planes: Vec<Plane>,      // packed layouts: exactly one plane
    pub color: ColorInfo,        // range + H.273 primaries / transfer / matrix (u8 code points)
    pub metadata: Metadata,      // icc / exif / xmp as Option<Vec<u8>>, gamma: Option<f32>
    pub palette: Option<Palette>,// indexed formats
}
pub struct Plane { pub stride: usize, pub data: Vec<u8> }

impl PngImage {
    pub fn new(width, height, format, planes) -> Self;              // + with_color / with_metadata / with_palette
    pub fn from_rgb8(width, height, data: Vec<u8>) -> Self;         // packed Rgb24, stride 3 * width
    pub fn from_rgba8(width, height, data: Vec<u8>) -> Self;        // packed Rgba, stride 4 * width
    pub fn width(&self) -> u32;  pub fn height(&self) -> u32;  pub fn format(&self) -> PixelFormat;
    pub fn as_bytes(&self) -> Option<&[u8]>; // Some for packed layouts (the single plane); None for planar — use into_raw
    pub fn into_raw(self) -> Vec<u8>;       // packed: the plane; planar: planes concatenated in order, strides as reported
    pub fn to_rgb8(&self) -> Vec<u8>;       // always available: exact integer kernels, palette expanded, deep/float sources tone-scaled to 8-bit
    pub fn to_rgba8(&self) -> Vec<u8>;      // alpha opaque when the source has none
    // nothing else: 16-bit, float and re-layouts belong to oxideav-image / oxideav-pixfmt
}
```

`to_rgba8` is implemented inside each crate for its own native layouts
(a JPEG crate converts its 4:2:0 YCbCr with the colour it signalled; a
GIF crate expands its palette). This keeps the standalone crate free of
`oxideav-pixfmt`, which stays the fast, complete converter for
framework users. `ColorInfo`, `Metadata`, `Palette`, `Plane` are small
per-crate structs with identical fields across crates.

### Features

```toml
[features]
default  = ["registry"]
registry = ["dep:oxideav-core", …]   # framework integration
```

- `default-features = false` gives Layer 1 and nothing else; it must
  build and pass its tests on CI (inline `ci-standalone` job).
- With `registry`: `register(&mut RuntimeContext)`, `make_decoder` /
  `make_encoder`, `From<XxxImage> for VideoFrame` and back (the
  pixel-format enums map 1:1 by name), and the framework `Decoder` /
  `Encoder` that call the Layer-1 functions. One implementation; the
  registry path is a thin adapter.
- Container image formats whose pixels come from a video codec
  (HEIF/HEIC via HEVC or AV1, AVIF, ICO-with-PNG, TIFF-with-JPEG) keep
  `decode` / `decode_rgba8` behind `registry`, because codec crates are
  framework-only by rule. Standalone they still expose `probe`, `info`,
  the container model, and composition over caller-supplied planes,
  and say so in their README.
- No crate may be `oxideav-core`-hard. jpegxl, iff and svg are the
  current exceptions to fix (svg's standalone `decode` rasterises only
  if `oxideav-raster` becomes core-optional; otherwise `probe` / `info`
  + its vector API).

### Behaviour

- Hostile input: every function returns `Error`, never panics; limits
  are enforced before allocation; the fuzz targets cover `probe`,
  `info`, `decode`.
- `decode` returns the native layout; `decode_rgba8` / `to_rgba8` are
  the convenience path. Encoders never convert silently.
- `color` is filled from the file (ICC, nclx, cICP, JFIF/Adobe,
  gAMA/cHRM) with the format's documented default when absent.
- Lossless formats: `decode(encode(img)) == img` for planes and
  metadata, pinned by a test.
- `#[non_exhaustive]` + constructors on every public record; parser
  plumbing is `pub(crate)` or `#[doc(hidden)]`.

### README order every crate follows

*Standalone use* (the one-screen example), *Framework use* (`register`,
factories), *Supported layouts* (decode / encode tables), *Options*,
*Metadata and colour*, *Limits*, then format-specific material.

## Layer 2 — `oxideav-image`, the gateway

A new crate depending on `oxideav-core` (not on any format crate). The
application registers formats itself, normally
`oxideav_meta::register_all(&mut ctx)`; `oxideav-image` only consumes
the registry, so it stays publishable (`oxideav-meta` resolves only in
the umbrella).

```rust
let mut ctx = oxideav_core::RuntimeContext::new();
oxideav_meta::register_all(&mut ctx);                       // or register the few crates you want

let file  = oxideav_image::open(&ctx, "photo.heic")?;       // probe → demuxer → decoder, via the registry
let img   = file.primary()?;                                // oxideav_image::Image (wraps a VideoFrame + params)
let rgba: Vec<u8> = img.to_rgba8()?;                        // through oxideav-pixfmt, fast
let (w, h, fmt) = (img.width(), img.height(), img.format());// oxideav_core::PixelFormat
img.metadata().icc(); img.color_signal();                   // from the container / codec
for frame in file.frames()? { … }                           // sequences, bursts, pages, animations

oxideav_image::save(&ctx, &img, "out.png", &SaveOptions::default().with_quality(90))?;
let bytes = oxideav_image::encode(&ctx, &img, "avif", &SaveOptions::default())?;  // by format name
```

- `Image` wraps the framework's `VideoFrame` + `CodecParameters`; it
  adds `to_rgb8`, `to_rgba8`, `to_rgba16`, `to_format(PixelFormat)`
  (every conversion `oxideav-pixfmt` knows), `into_raw` (planes
  concatenated), `crop`, `metadata()`, `color_signal()`, and
  `into_video_frame()` for users who continue in the pipeline.
- `open` / `decode_bytes` pick the demuxer and decoder through the
  registry's probe and priority rules (the same resolution every
  framework path uses); `save` / `encode` pick the encoder and muxer by
  extension or explicit format name and map `SaveOptions` onto the
  encoder's declared options schema.
- `oxideav-io`'s image half (`RgbaImage`, open/save) moves here;
  `oxideav-cli-convert` and `oxideav-render` drop their private
  `RgbaImage` copies in favour of this crate.

## Rollout

1. Document (this file) + umbrella README section.
2. Layer 1 in waves of five crates, one seat per crate, old functions
   kept as `#[deprecated]` thin wrappers for one release so in-workspace
   consumers migrate without a cascade:
   - wave 1: png, mjpeg, heif, avif, webp (the production-HEIF path)
   - wave 2: gif, tiff, bmp, qoi, tga
   - wave 3: pcx, pbm, farbfeld, wbmp, hdr
   - wave 4: openexr, dds, ico, pict, icer
   - wave 5: jpeg2000, jpegxs, jpegxl, iff/ilbm, svg (core made optional)
3. Bootstrap `oxideav-image` (new repo per the new-crate rules) after
   wave 1, so its first tests run against conforming crates; migrate
   `oxideav-io` / cli-convert / render onto it.
4. Umbrella README: an "Image formats" section pointing here; per-crate
   rows carry a contract badge once conformant.

## Checklist for a new image-format crate

- [ ] `oxideav-core` optional behind default-on `registry`; builds and
      tests with `--no-default-features` on CI
- [ ] root: `probe`, `info`, `decode`, `decode_with`, `decode_rgb8`,
      `decode_rgba8`, `decode_from`, `encode`, `encode_rgb8`,
      `encode_rgba8`, `encode_to`, `XxxImage`, `RgbImage`, `RgbaImage`,
      `PixelFormat` alias, `ImageInfo`, `EncodeOptions`,
      `DecodeOptions`, `XxxError` + `Error` alias
- [ ] `decode_all` if the format has multiple images
- [ ] `XxxImage::{to_rgb8, to_rgba8, into_raw, from_rgb8, from_rgba8}`
- [ ] with `registry`: `register`, `make_decoder`, `make_encoder`,
      frame conversions; the registry path calls the standalone fns
- [ ] native layouts decoded, `color` + `metadata` filled, encoder
      refuses unsupported layouts with `Error::Unsupported`
- [ ] lossless round-trip test; fuzz on `probe` / `info` / `decode`;
      hostile inputs never panic
- [ ] `#[non_exhaustive]` + constructors on public records; plumbing
      hidden
- [ ] README in the contract's section order
