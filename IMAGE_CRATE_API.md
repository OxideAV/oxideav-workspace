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
| `PixelFormat` | `pub enum XxxPixelFormat` + `pub type PixelFormat = XxxPixelFormat` | Variant names mirror `oxideav_core::PixelFormat` exactly (`Gray8`, `Rgb24`, `Rgba`, `Rgb48Le`, `Yuv420P`, `YuvJ420P`, `Yuv420P10Le`, `Gbrp`, `GrayF32Le`, `RgbF32Le`, `RgbaF32Le`, …). Only the variants the format can produce or accept. |
| `EncodeOptions` | struct, `#[non_exhaustive]`, `Default`, `with_*` | Quality / level / filter / chroma layout / which metadata to embed. One struct; behaviour variants are fields, never function suffixes. |
| `DecodeOptions` | struct, `#[non_exhaustive]`, `Default`, `with_*` | `max_width: Option<u32>`, `max_height: Option<u32>`, `max_pixels: Option<u64>`, `max_bytes: Option<u64>` (`None` = unlimited), `strict: bool`, plus format extras. |
| `ImageInfo` | struct, `#[non_exhaustive]` | `width`, `height`, `format`, `frames`, `has_alpha`, `color`, `has_icc`, `has_exif`, `has_xmp`. |
| `Error` | `pub enum XxxError` + `pub type Error = XxxError` | `std::error::Error` + `Display`; variants at least `InvalidData`, `Unsupported`, `LimitExceeded`, `Io(std::io::Error)`. Never a bare `enum Error`. |

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

## Rulings (2026-10-03, after wave 1)

Questions the wave-1 seats raised, decided here so every crate matches.
These are part of the contract.

- **`ColorInfo`** is exactly `{ range: ColorRange, primaries: u8, transfer: u8, matrix: u8 }` with `pub enum ColorRange { Unspecified, Limited, Full }` (never a `bool`).
- **`Metadata`** is exactly `{ icc: Option<Vec<u8>>, exif: Option<Vec<u8>>, xmp: Option<Vec<u8>>, gamma: Option<f32> }`.
- **`Palette`** is `{ entries: Vec<[u8; 4]> }` (RGBA, alpha 255 when the format has none). A crate whose format has no palette omits the `palette` field entirely (JPEG) — it is not an error to lack it.
- **`Plane`** is `{ stride: usize, data: Vec<u8> }`. A crate that depends on another image crate (avif on heif) may re-export that crate's `Plane`, `ColorInfo`, `Metadata`, `Palette` instead of copying them; the shape is identical either way.
- **`ImageInfo.frames`** is `u32`. Format extras (bit depth, interlace, JPEG process flags, HEIF primary item id, WebP loop count, …) are allowed as additional fields.
- **`DecodeOptions` limits** are `max_width: Option<u32>`, `max_height: Option<u32>`, `max_pixels: Option<u64>`, `max_bytes: Option<u64>`, `strict: bool`; `None` means unlimited; `Default` sets sane finite limits (reference: 1 GiB of decoded bytes) and `strict = false`. Format extras (tone mapping, layer / item selection, external tables, thread budget) are additional fields.
- **`Error::Io`** carries `std::io::Error` — never a `String` or an `ErrorKind` pair — with `impl From<std::io::Error>`. Error enums therefore do not derive `Clone` / `PartialEq`; tests match on variants or `Display`.
- **`to_rgb8` / `to_rgba8`** are infallible on any image the crate's own decoder produced. For caller-assembled images, `new(..)` validates plane geometry and returns `Result`, so an invalid image cannot exist; crates may additionally offer `try_to_rgb8` / `try_to_rgba8` for defensive callers.
- **`encode` of a layout the format cannot carry:** when the format has a natural layout for the input's colour model (RGB input into a YCbCr-only format such as JPEG / HEIF / AVIF / lossy WebP), `encode` converts to that layout exactly as `encode_rgb8` would and the README says so; `Error::Unsupported` is for inputs the format cannot represent at all (float into JPEG, 16-bit into GIF, alpha into a format with no alpha mechanism when the caller did not ask for it to be dropped — JPEG's `encode_rgba8` drops alpha and documents it).
- **Frame bridge under `registry`:** `From<XxxImage> for VideoFrame` and `XxxImage::from_video_frame(&VideoFrame, &CodecParameters) -> Result<XxxImage, Error>` (dimensions and format come from the parameters), plus `TryFrom<(&VideoFrame, &CodecParameters)>`.
- **Depth APIs keep their names.** Sub-format or depth entry points (`decode_apng`, `HeifFile`, `AvifFile`, `inspect`, Motion-JPEG and RTP surfaces) are not renamed to the contract verbs; the contract verbs are the common floor.
- **JPEG colour:** every YCbCr JPEG is full range (T.871), so its native format is the `YuvJ*` family regardless of JFIF presence, and the default code points are sYCC (primaries 1, transfer 13, matrix 5) per T.871 Note 3. Formats without normative colour defaults document the convention they chose.
- **Lossy animation encode** may be `Error::Unsupported` where the crate's encoder has no lossy animation path (WebP); the README says so.

### Rulings added after wave 2 (2026-10-04)

- **Constructors are fallible.** `XxxImage::new(..)`, `from_rgb8(..)`, `from_rgba8(..)` (and `packed(..)` where offered) return `Result<Self, Error>` and reject geometry / length mismatches with `InvalidData`. Crates that shipped infallible `from_rgb8` / `from_rgba8` in waves 1–2 convert in the fleet sweep after wave 5 (one commit per crate; the old signature is not kept).
- **`from_video_frame` returns the crate's `Error`**, not `oxideav_core::Error`; the registry adapter maps it.
- **`strict` may be a documented no-op** for formats that have nothing to be strict about (QOI); it must still exist.
- **Colour-signal side-channel** on registry frames: stamped whenever the file carries colour information (always for formats that always signal, e.g. QOI's colourspace byte; only when present for PNG/BMP).
- **Registry adapter output is the native layout** with the palette / colour-signal side-channels (`Pal8` + palette for indexed formats, as png does), never a pre-converted `Rgba`. bmp and tga currently emit `Rgba` from their framework `Decoder`; they switch in the fleet sweep after wave 5 (umbrella CLI pins re-checked by the parent).
- **`decode_all` frames may use a different layout than `decode`** when composition requires it (GIF: `decode` is the first frame as `Pal8`, `decode_all` frames are composited `Rgba`); the README states it.
- **Names reused by the contract cannot keep deprecated aliases** (`GifImage`, `Frame`, `AvifImage` changed meaning; `register(codecs, containers)` → `register_registries`); the CHANGELOG "Removed"/"Changed" entry is the migration note.
- **`Metadata.gamma`** is the file's single encoding gamma as an exponent (PNG `gAMA` semantics, e.g. 0.45455 for sRGB-like), only when the format expresses gamma that way; formats with per-channel curves (BMP V4) leave it `None` and surface the curve through a depth record.
- **Float and deep layouts decode natively** when core has the layout (`Gray16Le`, `Rgb48Le`, `GrayF32Le`, `RgbF32Le`, `RgbaF32Le` — little-endian `f32` samples, one packed plane); tone-scaling happens only inside `to_rgb8` / `to_rgba8` (float: clamp [0, 1] then ×255, documented). tiff currently tone-scales float on decode; it moves to native float in the fleet sweep.
- **Frame extras for paged formats** (`index`, `page_number`, `new_subfile_type` for TIFF) are fine.
- **Internal records in `pub mod`s** (e.g. a JPEG-in-TIFF `jpeg::Plane`) fall under the hygiene rule: `pub(crate)` or `#[doc(hidden)]`, never a second public type with a contract name.

### Rulings added after wave 3 (2026-10-04)

- **Float layout names** are core's: `GrayF32Le`, `RgbF32Le`, `RgbaF32Le` (little-endian `f32` samples). Earlier text saying `Rgb96F` was wrong and is corrected above.
- **Colour-signal stamping, refined:** the registry frame carries a colour signal when the FORMAT defines colour semantics (Radiance HDR = linear light; QOI's colourspace byte; JPEG = sYCC) or the FILE carries them (PNG/BMP/TIFF chunks). A convention a crate merely assumes (farbfeld, Netpbm, TGA, PCX) stays on the standalone `ColorInfo` as a documented default and is NOT stamped on the frame. (farbfeld currently stamps; fixed in the fleet sweep.)
- **Format-defined colour defaults are followed as written** (Netpbm: BT.709 primaries and transfer per its own definition; Radiance: linear, primaries `2` when the file has none — no "closest" code point is invented).
- **`info` may succeed on geometry that cannot be allocated** (it is header-only); the decode functions then fail with `Unsupported` when `width × height × bytes_per_pixel` overflows the platform's `usize`, and with `LimitExceeded` when a configured limit is hit.
- **Streaming decoders (`decode_from`) may stop at the announced body** and ignore trailing bytes where the format has no end marker; `decode(&[u8])` on the full buffer still rejects trailing garbage under `strict`.
- **Multi-image encode is `encode_all(&[Frame], &EncodeOptions) -> Result<Vec<u8>, Error>`**, the mirror of `decode_all`, on every format that has several images (APNG, GIF, WebP animation, TIFF pages, HEIF/AVIF sequences and bursts, DCX, WBMP frames). Format-specific names (`encode_apng`, `encode_animation`, `encode_frames`, `encode_tiff_multi`, `encode_sequence`) remain as depth aliases. Applied in the fleet sweep.
- **Single-encoding formats need no `rle` option** (PCX defines encoding 1 only); options exist only for choices the format actually offers.
- **Palettes read back padded** to the geometry's table length (PCX 16 / 256) are an accepted deviation from strict `decode(encode(img)) == img`, documented per crate.
- **Container-style multi-image files** (DCX, multi-page TIFF): `probe` accepts them, `decode` returns the first page, `decode_all` the rest.
- **Format extras in the raw surface** (`encode_gray8`, `decode_rgba16`, polarity selection on `DecodeOptions`) are allowed alongside the contract functions as long as the contract set is complete.
- **Every published crate sets `exclude = ["/tests", "/fuzz"]`** in `Cargo.toml` (crates.io 10 MiB cap); checked in the fleet sweep.

### Rulings added after wave 4 (2026-10-04)

- **Decode-only and encode-only variants** in a crate's `PixelFormat` are fine (PICT decodes `Rgba`, accepts `Rgb24` for encode; PCX accepts `Rgba` for encode only); the README's layout table marks the direction.
- **Composed-alpha formats** (ICO/CUR XOR+AND masks) decode to `Rgba`, never to an indexed layout: a per-pixel mask cannot ride on `Pal8`. Entry depth / sub-format extras keep re-encode faithful.
- **`info` returns `Unsupported`** naming the stored layout when a file's layout has no contract `PixelFormat` (DDS UINT/SINT/depth/YUV, EXR parts without an RGB(A)/Y view); the depth header API covers inspection of those files.
- **`decode_all` may skip images that have no contract view** (EXR deep / AOV-only parts), leaving gaps in `Frame.index`; `strict` turns the skip into an error; a file with no viewable image at all is `Unsupported`.
- **Two-channel layouts widen to RGBA with B = 0, A = 1** (the D3D missing-component convention) where core has no two-channel layout; `A8`-only decodes to `Rgba [0, 0, 0, a]`.
- **Depth-specific core labels** (`Gray10Le`, `Gray12Le`, `Gbrp10Le`, …) are used when the sample depth matches exactly; other depths ride the wider label (`Gray16Le`) with the significant-bits extra. `Gbrp` and `Gbrp8` both exist in core; use the one core maps to the crate's 8-bit planar RGB (check `format.rs`).
- **`encode_all` may derive container shape from `Frame` extras** (DDS mip level / face / slice; EXR part names) rather than from option fields.
- **Depth entry points with per-channel or arbitrary-layout power** (EXR `parse_exr -> ExrPart`, `encode_exr_scanline` with named channels) stay undeprecated when the contract `encode`/`decode` cannot express what they do; wrappers that merely duplicate the contract are deprecated.
- **Per-opcode or per-chunk internal budgets** (PICT's 256 MiB raster cap) may stay fixed even when `DecodeOptions.max_bytes` is `None`, as long as the README states the cap.
- **Workspace compression goes through `compcol`** (openexr moved off `flate2`); compressed-bytes pins are replaced by decoded-pixel pins when the deflate output changes.

### Rulings added after wave 5 (2026-10-04) — Layer 1 complete on all 25 image crates

- **Decoder-only crates** (JPEG XL today) expose the full `encode*` vocabulary returning `Error::Unsupported` with an empty `#[non_exhaustive] EncodeOptions`, and their `PixelFormat` lists only the layouts the decoder can produce (no speculative float variants); the README's layout table says "decode only".
- **Vector crates** (SVG) are conformant when `probe` / `info` work standalone, `decode*` / `encode*` return `Unsupported` AFTER parsing and limit checks (so `InvalidData` / `LimitExceeded` surface first), and `parse` / `write` are the real pair; `info` reports CSS pixels at 96 dpi and `0×0` when the document has no intrinsic size.
- **Component sets with no core layout** (JPEG 2000 signed / mixed-depth / ≥5 components, JPEG XS CFA / 2-component) are `Unsupported` on the contract surface and reachable through the crate's depth API; a crate does not invent non-core layout names. Two-component codestreams are read as grey + alpha (`Ya8` / `Ya16Le`) where the format gives no other meaning.
- **Fourth components are alpha** (`Gbrap*` / `Yuva*` / `Rgba`) unless the format's channel-definition box says otherwise.
- **Unsignalled YCbCr codestreams** are labelled `Yuv*` (range-neutral) with the range on `ColorInfo`; `to_rgb8` uses the format's documented default matrix and range (JPEG 2000: sYCC full range; JPEG XS: BT.709 limited) — stated per crate.
- **Depth entry points that never changed shape stay undeprecated** (IFF's `parse_ilbm` / `encode_ilbm`); a `#[deprecated]` wrapper is only for a replaced or renamed entry.
- **Palette-free encodes of alpha input into alpha-less formats** either drop alpha with a documented statement (JPEG, HDR) or refuse with `Unsupported` when the format offers an alpha mechanism the caller could have chosen (IFF: `form = Deep` / masking); the README says which.
- **Profile-level resource bounds** (JPEG XL Annex M) default to the lowest level and widen only when the file signals a higher one (`jxll`).
- **Framework demuxers of indexed formats** keep emitting what they emit today until the fleet sweep flips bmp / tga / iff to native `Pal8` + palette side-channel together, so umbrella pins change once.

Layer 1 status: waves 1–5 done 2026-10-04 — png, mjpeg, heif, avif, webp, gif, tiff, bmp, qoi, tga, pcx, pbm, farbfeld, wbmp, hdr, openexr, dds, ico, pict, icer, jpeg2000, jpegxs, jpegxl, iff, svg. jpegxl, iff and svg build without `oxideav-core` for the first time. The fleet sweep (round 470) applied the deferred items on every crate; next is Layer 2.

### Rulings added after the fleet sweep (round 470, 2026-10-04)

- **Fallible constructors apply to the contract image type** (`PngImage`, `JpegImage`, `HeifImage`, …) and to `RgbImage` / `RgbaImage` where they offer `new`. Depth types that users never assemble pixel-by-pixel (`ApngImage`, HEIF `DecodedImage` / `LinearRgbImage`, AVIF `StillImage` / `OverlayLayerImage`, SVG `FilterImage`) keep whatever constructor shape fits them; the gate only checks `impl XxxImage` blocks.
- **`encode_all` is universal** on the multi-image formats (png APNG, gif, webp, tiff, heif, avif, pcx DCX, wbmp, openexr, dds, ico, iff, icer). Where a format has several multi-image mechanisms the crate picks from the frames: HEIF/AVIF write an item burst when no frame carries a delay and a timed sequence track otherwise; WebP writes a single still for one delay-less frame; DCX and Netpbm concatenate. The README states the choice.
- **A `Frame` extra the encoder needs may be mandatory** when the crate offers `Frame::from_image(image)` that synthesises it (Netpbm `header`); callers then never hand-build the record.
- **`from_video_frame` returns the crate Error everywhere** — the wave-2 ruling is now applied to gif and qoi as well; the registry adapter maps it.
- **Registry output of indexed formats is native** (`Pal8` + palette side-channel) in bmp, tga and iff from this sweep on; umbrella pins were re-checked once.
- **tiff decodes float natively** (`GrayF32Le` / `RgbF32Le` / `RgbaF32Le`); tone-scaling lives only in `to_rgb8` / `to_rgba8`.
- **farbfeld no longer stamps** its sRGB convention on registry frames (wave-3 ruling applied); a caller-set `color` still travels.
- **`oxideav-meta` wires every image crate.** bmp, ico and tiff were missing and are added (features of the same name in the `image` group); a new image crate is not done until its meta row exists.
- **Contract type aliases are accepted by the gate** (`pub type Plane = AvifPlane;`) as long as the aliased struct has the contract fields.
- **`exclude = ["/tests", "/fuzz"]`** is present on all 25 crates (gate-checked).
- **A vector crate has no contract image type**, so a depth record that happens to carry the `XxxImage` name (`oxideav_svg::image::SvgImage`, the `<image>` element) keeps it; the hygiene rule targets plumbing, not named model records.
- **Colour-signal stamping needs a real signal**: a wrapper box that merely restates the layout default (JPEG XS non-CICP `colr`) is not one; only CICP / ICC / format-defined semantics stamp.
- **16-bit packed RGB with no core name** (BMP `Rgb555` / `Rgb565`) widens to `Rgb24` on the registry frame and in `decode`; the stored layout stays on the crate's depth record / `PixelFormat` so re-encode is faithful.
- **Registry stills vs animations may differ in layout** the same way `decode` and `decode_all` do (GIF: a single-image file decodes to native `Pal8` + palette, an animation to composited `Rgba`); the README states it.

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
   - wave 1: png, mjpeg, heif, avif, webp (the production-HEIF path) — done 2026-10-04
   - wave 2: gif, tiff, bmp, qoi, tga — done 2026-10-04
   - wave 3: pcx, pbm, farbfeld, wbmp, hdr — done 2026-10-04
   - wave 4: openexr, dds, ico, pict, icer — done 2026-10-04
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
- [ ] `decode_all` / `encode_all` if the format has multiple images
- [ ] `Cargo.toml` `exclude = ["/tests", "/fuzz"]`
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
