# oxideav

[![Donate](https://img.shields.io/badge/Donate-Stripe-635BFF?logo=stripe&logoColor=white)](https://donate.stripe.com/7sY8wPcnS9dO2Dqgvg4gg01)

A **pure-Rust** media transcoding and streaming framework. Every codec, container, and filter is implemented from the spec — no C libraries, no `*-sys` crates, no Rust wrappers around a userspace codec library.

The only place we use FFI is the optional **hardware-acceleration crates** (`oxideav-videotoolbox` / `-audiotoolbox` / `-vaapi` / `-vdpau` / `-nvidia` / `-vulkan-video`), which are thin bridges to the OS-provided HW engines — there's no other way to talk to GPU/ASIC encoder blocks. Those bridges load the system frameworks at runtime via `libloading` (no compile-time link, no `*-sys` build dep, no header shipped); the framework still builds and runs without any of them present. Disable hardware entirely with `--no-hwaccel` or by not enabling the `hwaccel` feature.

## Goals

- **Pure-Rust codec implementations.** No C codec library is wrapped, linked, or depended on — directly or transitively. Every codec, container, and filter is implemented from the spec.
- **Clean abstractions** for codecs, containers, timestamps, and streaming formats.
- **Composable pipelines**: media input → demux → decode → transform → encode → mux → output, with pass-through mode for remuxing without re-encoding.
- **Modular workspace**: per-format crates for complex modern codecs/containers, a shared crate for simple standard formats, and an `oxideav-meta` aggregator that wires them together behind Cargo features (preset bundles `audio` / `video` / `image` / `subtitles` / `hwaccel` / `source-drivers` / `all`; `pure-rust` = `all` minus `hwaccel` for zero-FFI builds; plus per-crate flags for fine slimming).
- **Hardware acceleration via the OS**: `oxideav-videotoolbox` / `-audiotoolbox` / `-vaapi` / `-vdpau` / `-nvidia` / `-vulkan-video` open the host OS's HW engine through `libloading` (runtime-loaded, no `*-sys` build dep). The OS's driver stack is the only path to GPU/ASIC codec blocks; we wrap the smallest possible surface (encode/decode session lifecycle + buffer in/out) and never re-implement OS APIs.

## Image formats: one API

Every image-format crate (`oxideav-png`, `oxideav-mjpeg`, `oxideav-heif`, `oxideav-avif`, `oxideav-webp`, …) exposes the same small standalone surface — `probe` / `info` / `decode` / `decode_rgb8` / `decode_rgba8` / `encode` / `encode_rgb8` / `encode_rgba8`, raw `Vec<u8>` in and out, usable with `default-features = false` and no `oxideav-core` — and the `oxideav-image` gateway opens any registered format through the framework. The contract, naming, feature layout and the checklist for a new image crate are in [`IMAGE_CRATE_API.md`](IMAGE_CRATE_API.md). Video and audio codec crates stay framework-only (registry + `make_*` factories); images are the deliberate exception.

## Non-goals

- Wrapping or linking userspace C codec libraries (ffmpeg, x264/x265, libvpx, libaom, libvorbis, libopus, libjxl, OpenJPEG, …).
- Perfect feature parity with FFmpeg on day one. Codec and container coverage grows incrementally.
- Re-implementing the GPU driver stack — for HW codecs we go through the OS, never around it.

## Workspace policy: clean-room, no external code

This is the **strict and universal rule** every contributor and every automated agent must follow. It is not a list of named libraries — it is a categorical prohibition:

> **No external library source code may be consulted, quoted, paraphrased, or used as a cross-check oracle while implementing any codec, container, protocol, or filter in this workspace.**

The rule applies to **every** external implementation, not a specific blocklist. That includes (but is in no way limited to): `ffmpeg` / `libav*`, `x264`, `x265`, `libvpx`, `libaom`, `dav1d`, `SVT-AV1`, `libvorbis`, `libopus`, `libspeex`, `fdk-aac`, `LAME`, `libjxl`, `jxlatte`, `jxl-rs`, `FUIF`, `brunsli`, `OpenJPEG`, `OpenJPH`, `Kakadu`, `schroedinger`, `xeve` / `xevd`, `VTM`, `JM`, `mp4v2`, every reference implementation distributed alongside a spec, and every third-party Rust crate that wraps or implements the same format (`lewton`, `claxon`, `image`'s codec submodules, `png`, `jpeg-decoder`, anything else of similar shape).

**"Cross-checking" counts.** Reading an external implementation "just to verify a table value" or "just to see how they handle this edge case" still contaminates the code. If you couldn't have written it without that reference, the resulting code is no longer clean-room.

**Allowed references:**
- Spec PDFs (ISO, ITU, ATSC, ETSI, RFC, IETF drafts, Annex documents)
- Clean-room behavioural-trace docs commissioned for this project (these are explicitly source-quote-free; the strict-isolation cleanroom workspace pattern at `docs/video/msmpeg4/`, `docs/video/magicyuv/`, `docs/audio/tta-cleanroom/` is the bar — Specifier role never reads the reference implementation source. Earlier behavioural-trace doc-only formats were retired 2026-05-06 under fruits-of-poisonous-tree)
- Reverse-engineered docs derived from disassembly of binary codecs whose source is unavailable (see `docs/video/msmpeg4/spec/01..13`)
- Public test corpora (raw fixture files: `.jxl`, `.j2k`, `.opus`, `.flac` etc.)

**Allowed validators (black-box only):** Decoder/encoder binaries — `ffmpeg`, `cjxl` / `djxl`, `ojph_compress` / `ojph_expand`, `opusdec`, etc. — may be invoked as opaque processes for output comparison. Feed input, compare output bytes. Their **source** stays off-limits.

**What to do when stuck:** If the spec PDF is ambiguous and no clean-room trace doc covers your case, the right move is to **ask the docs collaborator to commission a behavioural-trace writeup**, not to peek at the reference implementation. Park the work and document the gap.

This policy exists for legal and provenance reasons. Violations have to be expunged from history (force-push), not just reverted, because git blame would still tie the contaminated commit to the project.

## Workspace layout

The workspace is a set of Cargo crates under `crates/`, grouped by role:

- **Infrastructure** — `oxideav-core` (primitives: Packet / Frame / Rational /
  Timestamp / PixelFormat (incl. 16-bit planar YUV + deep Yuva 4:2:2/4:4:4 at 10/12/16-bit; VideoFrame palette + per-plane significant-bits side-channels for mixed-depth streams; r429: deep Yuva 4:2:0 + 8/16-bit planar GBR(A) formats + Ogg BOS-magic codec resolution; r449: 4:4:0 YUV ladder + scene-referred F32 family + plane-geometry/sizing helpers; r478: packed 16-bit RGB 555/565 + padded 32-bit RGBX labels, `Muxer::set_metadata` string-pair hook, `CodecCapabilities::embedded_blob_kinds`, `<producer>-<chunk>` blob naming rule — 82 formats) / ExecutionContext + **DoS framework: `DecoderLimits`
  caps, `arena::ArenaPool` (Rc-based, single-threaded) + `arena::sync::ArenaPool`
  (Arc-based, Send + Sync) refcounted bump-allocator pools, refcounted `Frame`
  whose drop returns the buffer to the pool, `Decoder::receive_arena_frame()`
  trait method with default impl that wraps `receive_frame()` for true zero-copy
  per-decoder opt-in (h261, h263, vp6 ports done)** — Decoder / Encoder /
  Demuxer / Muxer traits + their registries also live here, in
  `oxideav_core::registry::*`; numeric core is overflow-total (checked +
  rounding-mode Rational/rescale), LSB/MSB bit-I/O parity, property-tested +
  benched, 100% rustdoc under `missing_docs`), `oxideav-pipeline`
  (source → transforms → sink composition; r449 error-recovery
  contract — typed RunFailure attribution incl. partial stats +
  opt-in failed-output disposal down to partial-file deletion).
- **I/O** — `oxideav-source` (generic SourceRegistry + 5 scheme drivers
  (file/mem/data/slice/concat) with a typed URI triad + `open_bytes`
  dispatch + sticky-error prefetch ring; openers register as **bytes /
  packets / frames** and `SourceRegistry::open` returns the matching
  `SourceOutput::{Bytes, Packets, Frames}` variant so the executor can
  branch per shape; conformance + differential + fuzz suites, benched),
  `oxideav-http` (HTTP/HTTPS bytes driver, opt-in via feature — RFC 9110
  Range-seek with span accounting, If-Range-guarded transparent resume,
  forward-seek drain + GET range-probe for HEAD-hostile origins +
  driver-owned RFC 9110 §15.4 redirects with RFC 3986 resolution; lacks
  cookies/auth),
  `oxideav-rtmp` (`rtmp://` packet driver — registers via
  `oxideav_rtmp::register(&mut sources)`, default-on in `oxideav-cli`).
- **Effects + conversions** — `oxideav-audio-filter` (Volume / NoiseGate /
  Echo / Resample / Spectrogram), `oxideav-image-filter` (stateless
  single-frame Blur / Edge / Resize), `oxideav-pixfmt` (pixel-format
  conversion matrix — 3224/3306 ordered pairs (912 direct) via direct
  rows + staged fallback + the computed planar-family tier (full 24-member
  Yuv/Yuva + 9-member GBR families), reference-model +
  black-box-validated matrices + palette generation + dither).
- **Containers** — one crate each for `oxideav-ogg` / `-mkv` / `-mp4` /
  `-avi` / `-iff`. Simple containers (WAV, raw PCM, slin) live inside
  `oxideav-basic`.
- **Codec crates** — one crate per codec family; see the
  [Codecs table](#codecs) below for the per-codec status. Tracker formats
  (`oxideav-mod`, `oxideav-s3m`) are decoder-only by design.
  Recent sibling crates: `oxideav-evc` (MPEG-5 EVC, ISO/IEC 23094-1),
  `oxideav-jpegxs` (JPEG XS, ISO/IEC 21122), `oxideav-midi` (Standard
  MIDI File + soft-synth + UMP/MIDI 2.0 packet container & protocol), `oxideav-pbm` (Netpbm: PBM/PGM/PPM/PNM/PAM),
  `oxideav-nsf` (NES Sound Format — 6502 emu + 2A03 APU); image-format
  bootstrap wave: `oxideav-dds`, `oxideav-openexr`, `oxideav-farbfeld`,
  `oxideav-hdr` (Radiance RGBE), `oxideav-qoi`, `oxideav-tga`,
  `oxideav-icer` (JPL Mars-rover), `oxideav-wbmp`, `oxideav-pcx`,
  `oxideav-pict` (Apple QuickDraw); `oxideav-iff` extended with ILBM;
  `oxideav-embroidery` (machine-embroidery stitch formats — Tajima
  DST + Brother PEC/PES + Melco EXP + Janome JEF family (JEF/JEF+/JPX/PTN)
  + Husqvarna HUS/VIP decode+encode over an in-house GL bitstream
  decoder, PHC/PHB decode, .edr/.inf companions; typed stitch-design
  model + registry/probe framework fit (13-extension map, dual API);
  r440 corpus-pinned encoder choices, 33/33 real GL streams exact;
  lacks a real PHX sample + VP3/XXX stitch docs — ART is a documented
  no-grant).
  AVIF decodes end-to-end via `oxideav-av1` (pixel fidelity tracks the
  AV1 intra decoder).
- **Vector graphics + text** — `oxideav-svg` (read+write SVG; rounds 1-3
  ship full shape set + text/filters/masks/clipPath + use/symbol + svgz +
  animate/set@t=0), `oxideav-pdf` (multi-page writer + Scene
  metadata via `/Info` dict; reader: bytes → Scene with xref +
  FlateDecode + content-stream operator parser + the full §8.9
  image model (any BPC / colour space / Decode, stencil + colour-key +
  SMask masking, inline-image splicing; gs-validated) + r418 navigation surface (outline/links/named-dests/page-labels/article-threads) + embedded-CMap Type0 text extraction + vertical writing; lacks predefined CJK CMap tables), `oxideav-raster`
  (vector→raster rendering kernel — scanline AA, bilinear/Lanczos2/Lanczos3 + Mitchell/Catmull-Rom/B-spline cubic image resampling,
  trapezoidal coverage, soft masks, patterns, filter primitives, ICC
  pipeline, bitmap cache keyed by `Group::cache_key`, SVG2 stroke-linejoin miter-clip/arcs §13.5.5; r449 byte-exact perf pass — 1.79× geo-mean over 32 benches, caption-heavy scene 32.5×), `oxideav-ttf`
  (TrueType parser — cmap 0/4/6/12/14 incl. Variation Sequences, GSUB
  ligatures, GPOS kerning + per-script feature selection + coherent `Font::shape()` engine (all GSUB+GPOS lookup types, IGNORE_MARKS-aware, Arabic joining + mark attachment validated) + v1.1 FeatureVariations, COLR + CPAL + sbix tables + r369 CFF/CFF2 PostScript+variable outlines (Type 2 charstrings, CID-keyed, blend per-instance) + MATH + JSTF, TTC subfont
  selection, AGL glyph-name→Unicode, full `name`-table accessor API + gvar IUP inferred-delta variable-glyph interpolation + set-axis-by-tag/named-instance API), `oxideav-otf` (CFF / Type 2 charstrings incl. CID-keyed ROS/FDArray/FDSelect + arithmetic/stack/storage/conditional ops + Top-DICT FontMatrix/PaintType/CharstringType/StrokeWidth, ISOAdobe/Expert/ExpertSubset predefined charsets, cubic outlines; r222 GDEF + Coverage + ClassDef common-layout primitives + `GlyphClass` enum + GPOS ValueRecord/ValueFormat + Lookup Type 1 single-adjustment + CFF2 §12 ItemVariationStore for variable fonts; r352 GPOS Lookup Types 1-9 + GSUB Types 1-7 incl. mark-to-ligature + contextual/chained via shared module; r369 CFF2 variation-aware outlines (blend/vsindex) + GSUB Type 8 reverse-chaining + Device/VariationIndex tables + cmap formats 2/13/14+UVS; r372 variable-font GPOS/GDEF/BASE VariationIndex resolution — Device/VariationIndex deltas applied to kerning/marks/cursive/carets/baselines + CFF2 blend; r375 §6.3.6.2.1 MATH variable-font MathValueRecord resolution + BASE BaseCoordFormat3 VariationIndex baseline coordinates; r380 full OFF variable-font table set (fvar/avar/STAT/MVAR/HVAR/VVAR/BASE + ItemVariationStore/DeltaSetIndexMap + `Font::normalize_coords` axis→region-scalar) + vertical metrics (vhea/vmtx/VORG) + legacy kern formats 0/2; r394 Font::shape TEXT-SHAPING pipeline — GSUB 1–8 + GPOS 1–9 + FeatureVariations + variable-instance deltas, black-box byte-validated; r407 COLR v0+v1 paint graph (all 32 paint formats, ColorLine/ClipList, varIndexBase resolution) + avar v2 cross-axis deltas + 243-tag feature registry + UAX #24 Script_Extensions, r413 full colour-font surface: CPAL v0/v1 + COLR paint-graph→concrete-RGBA, sbix, EBLC/EBDT/EBSC, CBLC/CBDT, SVG table — real-font black-box validated; r418 COLR v1 paint-graph decode (all 32 formats, variable, budget-capped validator) + avar v2 cross-axis normalisation; r435 UAX #24 itemized shaping over vendored UCD 17 (`Font::shape_runs` per-run script tags) + script/language/baseline tag registries + CPAL linear-light gradient interpolation + IVS LONG_WORDS + CJK em-box/ICF; r443 OFF common-formats completion — DeltaSetIndexMap format 1, LONG_WORDS delta rows, NULL IVD offsets, variable-COLR conformance fixture end-to-end + gvar repeated-point accumulation fix),
  `oxideav-scribe` (shaper with vector-first `Shaper::shape_to_paths`
  API — no rasterizer dep; trapezoidal horizontal AA, GPOS mark-to-mark,
  COLR/CBDT colour glyphs via raster bilinear/composer; bidi UAX #9
  data-complete at Unicode 16.0 — Bidi_Class ranges + bracket pairs +
  mirror table; r445 full UAX #24 §5 itemisation — Script_Extensions sets + paired-bracket refinement + universal §5.2 cluster model + cluster-atomic font fallback; lacks per-script USE syllable grammars — docs-gapped).
- **3D scenes & assets** — typed `oxideav-mesh3d` (Scene3D / Mesh /
  Material PBR / Skin / Animation / Camera / Light / AudioEmitter +
  area-weighted vertex-normal recompute + MikkTSpace-style tangent-space basis (Lengyel 2001) +
  full skeletal pipeline (joint matrices + linear-blend skinning + weight repair + pose sampling + animated/rest instantiation + skin-root LCA) +
  r448 morph model complete (typed target names, USD in-between stations, sampled-MorphWeights synthesis) +
  `Mesh3DRegistry` parallel to `CodecRegistry` + `AssetSource`
  lazy-bytes trait with `raw_storage` pass-through for archive-backed
  sources). Per-format codecs `oxideav-stl` / `-obj` / `-gltf` / `-usdz`
  register into the registry; `oxideav-meta::populate_mesh3d_registry`
  walks every enabled format. See the
  [3D scenes & assets table](#3d-scenes--assets) below for per-format
  status. `oxideav convert in.obj out.gltf` (or `--probe in.gltf`) is
  the CLI entry point. Cross-format integration tests live under
  `crates/oxideav-tests/tests/mesh3d_*.rs`.
- **Facade** — `oxideav` is a thin re-exporter over `oxideav-core` +
  `oxideav-pipeline` + `oxideav-source`. Holds no codec deps; the
  high-level invoke API will live here.
- **Aggregator** — `oxideav-meta` exposes
  `register_all(&mut RuntimeContext)` which explicitly invokes every
  enabled sibling's `register(ctx)` fn. Each sibling is a Cargo
  feature; `default = ["all"]` pulls everything. Preset bundles
  available: `audio`, `video`, `image`, `subtitles`, `hwaccel`,
  `source-drivers`, `all`, and `pure-rust` (= `all` minus `hwaccel`,
  for builds that avoid all FFI to OS HW-engine APIs). Slim builds via
  `oxideav-meta = { default-features = false, features = ["image"] }`
  (or any per-crate combo). `register_all` body is auto-generated by
  `oxideav-meta`'s `build.rs` from its own `Cargo.toml` — adding a
  sibling means adding one line to `Cargo.toml`; the build script
  regenerates the call list (r475: the script parses the normalized manifest cargo ships, so `register_all` is populated for crates.io consumers too — an off-umbrella CI job proves it). **Version policy:** a sibling meta reaches
  only through the `register!` entry is required as `version = "0"`
  (any 0.x) — no API contract is consumed, and a narrower line would
  let a sibling's minor bump leave two copies in a consumer's graph;
  only dependencies meta calls a real function or type on
  (`oxideav-core`, `oxideav-mesh3d` + the 3D format crates,
  `oxideav-render`) keep a versioned requirement. (Earlier attempt at a `linkme`-based
  distributed-slice approach was dropped: linkme has open issues on
  `wasm32` targets, and its DCE workaround required a manual
  `ensure_linked()` call from main anyway.)
- **Binaries** — `oxideav-cli` (the `oxideav` CLI: `list` / `probe` /
  `remux` / `transcode` / `run` / `validate` / `dry-run` / `convert`)
  and `oxideplay` (reference SDL2 + TUI player). Windows-codec
  forensic debugging now lives in [`KarpelesLab/univdreams`](https://github.com/KarpelesLab/univdreams)
  via `ud vfw {probe,decode,encode}` — see Windows codec sandbox below.

(`oxideav-job` and `oxideav-tracevfw` are retired — `oxideav-job`'s
functionality moved into `oxideav-pipeline`; `oxideav-tracevfw`'s
debugger CLI moved into `ud-cli` from univdreams, which also hosts
the underlying x86/PE/Win32 sandbox. Both archived on GitHub.)

Use `cargo run --release -p oxideav-cli -- list` to enumerate the codec
and container matrix actually compiled into the release binary.

## Core concepts

- **Packet** — a chunk of compressed (encoded) data belonging to one stream, with timestamps.
- **Frame** — a chunk of uncompressed data (audio samples or a video picture).
- **Stream** — one media track inside a container (audio, video, subtitle…).
- **TimeBase / Timestamp** — rational time base per stream; timestamps are integers in that base.
- **Demuxer** — reads a container, emits Packets per stream.
- **Decoder** — turns Packets of a given codec into Frames.
- **Encoder** — turns Frames into Packets.
- **Muxer** — writes Packets into an output container.
- **Pipeline** — connects these pieces. A pipeline can pass Packets straight from Demuxer to Muxer (remux, no quality loss) or route through Decoder → [Filter] → Encoder.
- **Scene** — a time-based composition of objects (images, videos,
  text, shapes, audio cues) on a canvas, animated over a timeline via
  keyframed properties. One model covers three workloads that would
  otherwise be separate stacks: a single-frame **document layout**
  (e.g. a PDF page — text stays selectable, vectors stay crisp), a
  long-running **live compositor** driven by external operations
  (add/move/fade — the shape an RTMP overlay control plane needs),
  and an **NLE timeline** with tracks, transitions, and per-object
  effect chains. A Scene feeds the pipeline as a Source: the renderer
  rasterises a frame at a given timestamp, so scenes can be encoded,
  streamed, or re-exported like any other media stream. Lives in
  [`oxideav-scene`](https://github.com/OxideAV/oxideav-scene) — type
  model is in place; rendering backends live in `oxideav-render`
  (scanline rasteriser + Whitted raycast).

## Using a codec directly (no containers, no pipeline)

Every codec crate in OxideAV is designed to be usable on its own.
Pull only `oxideav-core` (types + the `Decoder` / `Encoder` traits +
`CodecRegistry`) and the codec itself:

```toml
[dependencies]
oxideav-core = "0.1"
oxideav-g711 = "0.0"   # or any other codec crate
```

```rust
use oxideav_core::{CodecId, CodecParameters, Frame, Packet, RuntimeContext, TimeBase};

let mut ctx = RuntimeContext::new();
oxideav_g711::register(&mut ctx);

let mut params = CodecParameters::audio(CodecId::new("pcm_mulaw"));
params.sample_rate = Some(8_000);
params.channels = Some(1);

let ulaw_bytes: Vec<u8> = vec![0xFF; 160]; // 20 ms of µ-law silence
let mut dec = ctx.codecs.first_decoder(&params)?;
dec.send_packet(&Packet::new(0, TimeBase::new(1, 8_000), ulaw_bytes))?;
let Frame::Audio(a) = dec.receive_frame()? else { unreachable!() };
// `a.data[0]` is S16 PCM.
# Ok::<(), oxideav_core::Error>(())
```

Each codec crate's README has a concrete example tailored to its
payload shape.

## Current status

`oxideav list` (via the CLI) prints the live, build-time-accurate
codec + container matrix with per-implementation capability flags —
that's the source of truth at any point. The tables below are the
human-readable summary, grouped + collapsible so the page stays
scannable.

Legend: ✅ = working end-to-end at the scope described.
🚧 = scaffold or partial — the row spells out what is present and
what is still pending. `—` = not implemented.

<details>
<summary><strong>Containers</strong> (click to expand)</summary>

Container format detection is content-based: each container ships a
probe that scores the first 256 KB against its magic bytes. The file
extension is a tie-breaker hint, not the source of truth — a `.mp4`
that's actually a WAV opens correctly.

| Container | Demux | Mux | Seek | Notes |
|-----------|:-----:|:---:|:----:|-------|
| WAV       | ✅ | ✅ | ✅ | Full metadata-chunk family (BWF bext, LIST/INFO, iXML, smpl/inst, Acidizer acid, MCI cue/plst/adtl, ADM chna + BW64 axml/bxml/sxml XML carriers + DISP/id3/PAD + write-symmetric LIST INFO/smpl/inst) + r448 WAVE_FORMAT_EXTENSIBLE COMPLETE (SubFormat GUID↔tag routing incl. IEC 61937 identification, dwChannelMask→typed core layouts, container-size dispatch fix, wSamplesPerBlock union, auto-EXTENSIBLE mux with legacy opt-out; interop fixtures black-box byte-exact) + RF64/BW64 64-bit form (read + write; ds64/JUNK on-the-fly promotion) + hostile-input hardened (bounded chunk allocs, count-caps, i64-safe seeks) + fuzz |
| FLAC      | ✅ | ✅ | ✅ | All metadata blocks (VORBIS_COMMENT / PICTURE / CUESHEET / SEEKTABLE) + §8 typed whole-chain parse/write; encoder five-window default apodization + compression-quality regression guards (−5% on 7.1/24-bit, bit-exact); decode verify-decoder MD5 (§8.2); muxer SEEKTABLE generation (§8.5, configurable density); decode spans 8–32 bit incl. the 33-bit decorrelated side channel (RFC 9639 §4.2/App. A.2) |
| Ogg       | ✅ | ✅ | ✅ | Vorbis/Opus/Theora/Speex + chained streams + page-bisection seek + Skeleton 3.0/4.0 read AND write incl. keyframe-index fast-path seek + granulepos→playback-time mapping + §4 mixed grouping+chaining (unique-serial enforced) + nil-page + multi-page packet reassembly coverage + Skeleton-4.0 fishead time anchors (basetime/presentation start_time, chained segment-length index check) + Opus pre-skip granule semantics (RFC 7845 §4.3, seek-bisection axis) + FLAC-in-Ogg header-packet count + Vorbis-comment (RFC 9639 §10.1) + Speex/FLAC ID-header sample-rate/channels → 1/rate time-base (correct duration/seek) + Speex comment metadata + Theora mapping COMPLETE (ID-header-driven demux with per-packet pts/keyframe flags + Skeleton-free seek on plain .ogv; mux with KFGSHIFT split-granule packing, header-section drain + cross-stream time-ordered A/V interleave — 11/11 fixtures remux byte-identical, external-validator-clean, Theora+Vorbis/Opus merges) + 10-target structure-aware fuzz suite (phantom-Skeleton-serial fix + open()-header-budget DoS bound + mux page-order invariant); r430 registry-first codec ID (payload magics, shared-resolver chained links) |
| Matroska  | ✅ | ✅ | ✅ | MKV/MKA/MKS; full RFC 9559 + RFC 8794 element surface demux↔mux symmetric, CELLAR schema validator, damage-resilient open, TrackOperation applied, daily fuzz; r480 cue-less seek by byte-range bisection (48 MB open 534 → 12 requests), VFW/ACM compatibility tracks through the registry |
| WebM      | ✅ | ✅ | ✅ | First-class: separate fourcc, codec whitelist (VP8/VP9/AV1/Vorbis/Opus); inherits Matroska Cues seek |
| MP4       | ✅ | ✅ | ✅ | mp4/ismv/fMP4 (DASH/HLS/CMAF), CENC all four schemes both ways, HEIF item graph, full §8.6.6 edit-list timeline, 6-target fuzz; r480 10 external PRs merged, edit list from DTS fixed; lacks iTunSMPB gapless (docs) |
| MOV (QuickTime) | ✅ | ✅ | ✅ | QTFF + ISO BMFF fragments / edit lists / `cmov` / muxer; full QuickTime Metadata atom both ways; HEIF lives in `oxideav-heif`; r480 codec-config records (avcC/hvcC/av1C/esds/…) as extradata, H.264 MOV bit-exact; fuzz clean |
| AVI       | ✅ | ✅ | ✅ | AVI 1.0 + OpenDML 2.0 spec-complete (super/std indices, multi-RIFF, txts, vprp), 3-target fuzz; r480 DIB/RGBA → rawvideo with pixel format, bit-exact vs ffmpeg; lacks a YUV FourCC map (docs) |
| Blu-ray (BD-ROM) | ✅ | — | — | UDF 2.50 + BDMV + `.m2ts` + `bluray://`; playlists / chapters / multi-angle + EP_map keyframe seek + AACS hook + HDMV nav title-engine (index.bdmv→MOBJ; inter-title Jump/Call/Resume, PSR4 seed) + PGS subtitle-segment parser (PCS/WDS/PDS/ODS + RLE) + Display-Set grouping + multi-ODS fragment reassembly + PGS renderer (palette resolution + window compositing) + HDMV navigation-command opcode decode + PSR/GPR register model + HDMV VM execution (Set/Compare/Branch interpreter + Movie-Object runner over Jump/Call/Resume) + CLPI SequenceInfo/AtcSequence/StcSequence + ClipInfo byte/SPN index + ProgramInfo PID lookups + CPI EP-map keyframe seek-index accessors (BD-ROM AV §5.5.4/§5.7) + UO_mask_table/is_repeat_SubPath round-trip fidelity + BDMV fuzz/hostile-input hardening + r443 SubPath/SubPlayItem (typed SubPathType, multi-clip) + typed 64-bit UO-mask (30 operations + reserved-bit forensics) + title-timeline sync windows; lacks SubPath clip playback + IG button-state machine + BD-J |
| DVD-Video | ✅ | — | — | ISO 9660 + UDF 1.02 + IFO/VOB + `dvd://`; navigation VM (incl. PCI NSML_AGLI non-seamless angle jump) + SPU subpictures + RGBA compositor + time seek + VOB → MKV + DTS core frame-header decode + generic audio-substream header (FrmCnt/FirstAccUnit + access-unit offset) + PCI_GI vobu_isrc/c_eltm decode + PCI RECI raw-region capture + 16/20/24-bit LPCM width (bytes_per_sample ratio) + DSI nav-pack typed accessors (VOBU_SRI/SYNCI/SML_PBI) + IFO PGC program-map navigation + PGC_AST_CTL/PGC_SPST_CTL stream-control tables + PGC_SPST display-mode sub-stream resolver + typed StillTime + §6.2 MPEG-2 video elementary-stream header stack (Sequence / Sequence-Extension / Sequence-Display-Extension / GOP / Picture / Picture-Coding-Extension headers) + full navigation engine (domain-transition legality + angle-aware cell walk + PgcRunner pre/cell/post state machine + Type-1 Link resolution + menu D-pad/button bridge + disc-absolute TitlePlan) + playback runtime (stills + NavTimer + audio/SPU stream-select + karaoke routing + VOBU trick-play + backward-SRI index fix) + LPCM frame packing (16/20/24-bit) + synthetic-disc e2e suite; + full 5-band private_stream_1 map (incl. SDDS) + substream census + CGMS-A/APS copy-control decode + r436 second-pass ratification (vobu_cat/CPR_MAI raw-preserve pinned) + full PES header-extension decode + pack classifier + DVD-compliance auditor + 20/24-bit LPCM sample unpacking; lacks CSS auth + odd-channel 20-bit LPCM nibble order |
| MP3       | ✅ | — | ✅ | ID3v2/v1 + Xing/Info VBR + CBR/VBR seek; stereo decode via oxideav-mp3 |
| IFF (EA IFF 85) | ✅ | ✅ | — | `FORM/LIST/CAT` family — Amiga 8SVX + ILBM (EHB/HAM, palette-change chunks) + ANIM op-0/1/2/3/4/5/7/8 + true-colour FORM RGB8/RGBN/DEEP decode + encode (genlock-RLE + TVDC chunky) + FORM ACBM/ABIT plane-contiguous decode+encode+mux + Apple AIFF/AIFF-C + fuzz harness + r449 8SVX voice-structure COMPLETE (typed VHDR + per-octave voice trees + ADSR/SEQN/FADE/PAN read+write; looping-duration + stereo-split demux fixes; svx_decode fuzz target — one forged-ckSize OOM fixed across all FORM readers); lacks 16SV + ANIM op-J (docs-asked) |
| IVF       | ✅ | — | — | VP8 elementary stream container |
| MPEG-TS   | ✅ | ✅ | ✅ | ISO/IEC 13818-1 TS demux/mux — full PSI/SI (DVB + ATSC A/65), T-STD model, splice signalling, keyframe-accurate seek, 4-target fuzz; r480 MPEG-2 program-stream demuxer, one packet per audio syncframe, stream params from the ES; lacks MPEG-1 Systems (11172-1 not staged), PS seek |
| AMV       | ✅ | ✅ | — | Chinese MP4-player format — demuxer + muxer + seek + strict-mode validators + symmetric demux→pixels/demux→PCM conveniences + S16-mono audio stream params + §4b IMA/DVI-ADPCM audio decode (1116 blocks → 93.0 s; decode_audio_payload convenience + PCM ffprobe-validated) + §4a in-crate baseline-JPEG video decode to RGB (device-hardcoded quant/Huffman tables) + video rate control + device-profile muxer validation + fuzz harness |
| FLV       | ✅ | ✅ | — | MP3/AAC/H.264 audio + VP6/H.264 video + Enhanced-RTMP extensions (incl. v2 audio-silence discard) + AMF0 metadata + multitrack + HDR colorInfo + fuzz; muxer covers tags / seek-table / cue-points / multitrack join + AMF0 Date (SCRIPTDATADATE) write + AMF3 value encoder + Enhanced-RTMP multichannel-config writer + onMetaData full AMF0 value matrix (nested Object/EcmaArray/StrictArray/Xml/Null) + Enhanced-RTMP-v2 per-track info maps + typed onCuePoint params + ModEx timestamp-offset-nano side-channel + multitrack join_tracks→demux round-trip + FLV-encryption muxer (Annex F.2 |AdditionalHeader + F.3 filtered-tag) + AMF0 Long String (>64 KB onXMPData) + Annex B.1 typed Flash-metadata accessors + Annex B.2 onImageData embedded-image harvest (read+write) + E.4.2.1 Speex/Nellymoser rate+channel pinning + legacy muxer write surface COMPLETE (PCM/ADPCM/G.711/Nellymoser/Speex/MP3-8k/screen-video + audio-silence signal) + fuzz |
| WebP      | ✅ | ✅ | — | RIFF/WEBP (lossy + lossless + animation; ANIM + ANMF emit) + VP8L encoder density push (cost-priced LZ77 DP + entropy-merge clustering + stacked transforms; −13.3% aggregate, byte-smaller than reference max-effort on 9/10 corpus images; max-effort wall −44% via cache-sweep hoists + DP planner) |
| TIFF      | ✅ | ✅ | — | TIFF 6.0 single-image + BigTIFF + PhotometricInterpretation=5/8 CMYK + CIE L*a*b* decode/encode + CCITT T.4 2-D + T.6 (Group 4) fax decode/encode + tiled-image layout + float Predictor=3 (IEEE 16/32/64-bit gray+RGB, strip/tile/planar) + tiled JPEG-in-TIFF (Compression=7) + 4:4:4 YCbCr planar+predictor + planar CMYK coverage |
| PNG / APNG| ✅ | ✅ | — | 8/16-bit all colour types, APNG, full PNG3 chunk set, colour management, bomb-bounded inflate; r480 9 external PRs merged, APNG frames rendered whole through the registry (ffmpeg framemd5 match), loop count both ways, `Ya16Le` encode; lacks caller-chosen chunk order |
| GIF       | ✅ | ✅ | — | 87a/89a + LZW + animation + NETSCAPE loop + disposal compositor + typed extension accessors + truecolor RGBA encode (median-cut + nearest-entry remap + Floyd–Steinberg dither + shared-palette animations) + interlaced-encode surface (§20.c.vii) |
| JPEG      | ✅ | ✅ | — | Still-image wrapper around the MJPEG codec |
| BMP       | ✅ | ✅ | — | Windows bitmap — DIB headers BITMAPINFOHEADER / V4 / V5, 1/4/8/16/24/32-bit + explicit-mask BI_BITFIELDS / BI_ALPHABITFIELDS V3 encoder (5 presets, 32-bpp lossless) + OS/2 file-magic recognise-reject (BA/CI/CP/IC/PT); also exposes the DIB helpers used by ICO / CUR sub-images |
| Netpbm    | ✅ | ✅ | — | All seven PNM magics + PAM; 1/8/16-bit; ASCII + binary fast paths (up to ~50 GiB/s) |
| ICO / CUR | ✅ | ✅ | — | Windows icon + cursor — multi-resolution, BMP and PNG sub-images; body-dim `(0,256]` reject + CUR hotspot body-derived bound + dir wBitCount vs body biBitCount cross-check + ANI (RIFF/ACON) framework Demuxer (anih + seq/rate timeline → packet stream) |
| slin      | ✅ | ✅ | — | Asterisk raw-PCM: .sln/.slin/.sln8..192 |
| MOD / S3M / STM / IT | ✅ | — | — | Tracker modules (decode-only by design) — see Trackers table |

Cross-container remux works for any pair whose codecs don't require
rewriting (FLAC ↔ MKV, Ogg ↔ MKV, MP4 ↔ MOV, etc.).

### Content protection

| Layer | Status | Notes |
|-------|:-------|-------|
| AACS  | ✅ Common 0.953 + BD-Prerecorded 0.953 | `oxideav-aacs` clean-room — full key-derivation chain (Device Key → VUK), Aligned-Unit decryption, SCSI MMC drive layer + Drive-Host AKE, MKB (incl. Type-4 verify-precursor/KCD Media-Key resolution)/Content-Certificate/CRL verification + GET CONFIGURATION / AACS Feature Descriptor host capability discovery + CPS Unit Usage File / CCI + AACS On-line Enhanced-Title key derivation + hostile-input hardening battery. Lacks AACS 2.0 |

</details>

### Codecs

> Each row below is a current-state summary. For round-by-round history, design notes, and per-feature trade-offs, see the per-crate `README.md` and `CHANGELOG.md` in `crates/oxideav-<codec>/`.

<details>
<summary><strong>Audio</strong> (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **PCM** (s8/16/24/32/f32/f64) | ✅ 100% | ✅ 100% |
| **slin** (Asterisk raw PCM) | ✅ 100% | ✅ 100% |
| **FLAC** | ✅ 100% | ✅ 100% |
| **Vorbis** | ✅ ~98% — sample-exact decode incl. floor-0 + hostile-codebook fence | 🟡 ~93% — psy VBR+ABR + refinement rungs closing the class-ladder gap + genuine low-bitrate mode (~15 kbps) + ffmpeg/oggdec bit-agreement on every stream; lacks reference-level tonal SNR below 60 kbps |
| **Opus** | ✅ ~96% — SILK bit-exact vs the RFC listing, CELT/hybrid, all §4.2.9 output rates, FEC/PLC/DTX | ✅ ~98% — unified encoder, every §2.1 knob + §4.5 transitions, signal-adaptive speech/music election (own analyser, default on), 40/60 ms CELT/Hybrid packets, SILK §5.2.3.3 gain + continuous rate knob; lacks CELT→Hybrid re-entry tail |
| **MP1 / MP2** | ✅ ~99% — Layer I + II decode (PCM-bit-exact) + CRC-16 + free-format probe + ISO 13818-3 LSF; r429 fuzzed — 5-target harness incl. encode→decode conformance oracle, 609M execs clean (ASan), daily scheduled | ✅ ~95% — Layer I + II encoders end-to-end + Annex D Model-2 Table D.3a 32 kHz partition table (49 rows) + §C.1.5.2.5/Table C.4 perceptual SCFSI selection + complete Annex D Model-2 numeric table set (D.1a-c threshold-in-quiet + D.3a-c calc-partition + spreading operators + D.4a-c per-FFT-line absolute threshold + D.1d-f LTq, all rates); Model-2 allocator wired into BOTH Layer I + Layer II encoders + intensity-stereo (L+R)/2 combine + auto intensity-bound + per-frame VBR + full Annex D Model 1 (D.1 Steps 1–9: Hann FFT/SPL, tonal classification, decimation, LTg/LTmin, SMR; both example models selectable for Layer I+II) — encoder ~99% |
| **MP2** | ✅ ~97% — L2→PCM ≤1 LSB full mode×rate matrix + §2.5 MC + opt-in '10' surround stage; lacks the unparameterised dynamic expansion | ✅ ~98% — both Annex D models at all six rates, every §2.5 election, extension streams, alias-cancellation guard (beats the black-box reference at equal rate) |
| **MP3** | ✅ ~99% — bit-exact decode + free-format frames + ID3v2/Xing seek + 13818-3 Table B.2 LSF bands + full MPEG-2.5 (8/11.025/12 kHz; 11.025 kHz validated vs reference PCM) + demux fuzz (free-format sub-header panic fixed) + 8 kHz mixed-block foreign decode (band-relative 72-line coding split, 4-validator unanimous; LSF mixed+intensity fix) + gapless-trimmed duration + r432 5-target fuzz surface (encode-loop panic-freedom + encode→decode roundtrip invariant; nightly Fuzz CI; 114M-exec clean campaign) | ✅ ~100% — full Layer III + joint stereo + MPEG-2 LSF/MPEG-2.5 encode (CBR/VBR/MS/CRC) + §2.4.3.2 LSF + §2.4.3.4.9.3 short-block (per-window) intensity-stereo incl. auto-MS + short + intensity combined + auto-block-type short granules with intensity (incl. intensity-only non-MS) + §C.1.5.2 LSF/MPEG-2.5 auto block-type + §C.1.5.3.2.1 Model-2-driven (pe>1800) block-type switching + §C.1.5.3 scfsi scalefactor-selection-info + §C.1.5.4.4.6 band-aligned SUBDIVIDE + Model 2 psychoacoustic threshold in the outer loop + named quality presets (Transparent/High/Standard/Fast, Model-2-driven at 32/44.1/48 kHz) + encode benchmarked (whole-stream + per-stage; inner rate loop is the ~10× hotspot) + MPEG-2.5 deployed band tables measured by observer-trace (5 conformance fixes: 16 kHz-LSF-pair tables, band-relative short region-0, short band-12, mixed alias butterfly, Start/End region splits) + §2.4.2.7 mixed bursts flag flanking Start/End granules (second-validator divergence closed, ≤4.6e-5; LSF auto-mixed demotes to pure-short) + 39-case sweep float-perfect on 4 validators (r440: mixed@8 kHz emit LANDED — window split spec-grounded at 36 subband lines across all rates) + r409 output-invariant perf axis: encode 3–13× / decode ~2× faster (golden-hash-pinned) |
| **AAC** | 🚧 ~97% — LC/HE-v1/v2/LD/SSR/BSAC decode + SBR/PS; r480 9 external PRs: PS first-envelope + independent-CPE fixes, opt-in Apple-compat (sine first block, shared PNS seed, declaration order, core-only, f64 view); lacks the ER-LD TNS filter record | ✅ ~99% — LC + HE-AAC v1/v2 encoders, NMR within −0.2..−2 dB of the black-box reference; lacks a 960-line LC core, 22.2 height layouts (docs) |
| **CELT** | ✅ ~98% — reference-lockstep + PLC, fuzzed (RFC 8251 §8 energy-cap fix), ~20× realtime decode | ✅ ~98% — ahead of the RFC listing's encoder at every LM × rate × material point (mean +1.0–1.6 dB), custom modes 18/18, measured-cost election; lacks an encode-throughput bench |
| **Speex** | 🚧 ~88% — NB/WB/UWB + intensity stereo, exact HB gain laws; lacks bit-exactness, enhancer, fixed-point path (docs staged) | 🚧 ~80% — NB/WB/UWB CELP q0–10 incl. UWB 44 kbit/s, VAD/DTX; r480 LPC-analysis sign bug fixed (#7: every encoder sent a flat envelope) |
| **GSM 06.10** | ✅ — FR bit-exact both directions + 06.11 RX lost-frame legs; HR (06.20) decoder 0.99 corr vs the ETSI outputs (lattice sign fix); bit-exact blocked on the unstaged 06.06 arithmetic | ✅ — FR bit-exact + DTX; HR (06.20) encoder complete (INT_LPC/MODE 91%, unvoiced codes ~80%, voiced 20%, lags ±1 66%), registry `gsm-hr`; lacks HR bit-exactness (06.06) + HR DTX/VAD (06.41/42 unstaged) |
| **G.711** (μ/A-law) | ✅ 100% | ✅ 100% |
| **G.722** | ✅ ~97% — SB-ADPCM decoder BIT-EXACT vs the ITU conformance corpus (97,536/97,536 octets, all 3 modes) + QMF + auxiliary-data channel + clause-2/2.4 operational conformance (S/D gates, idle-channel limits, group delay ≤4 ms) + pcm16 API (clause-5.2 Note-2 rescaling) + fuzz + robustness totality | ✅ ~95% — SB-ADPCM encoder BIT-EXACT vs the ITU corpus (48,768/48,768; 3 arithmetic bugs fixed: QQ4 row addressing, FILTEP timing, UPPOL1 stability window) + Mode 2/3 round-trip + conformance meters + r435 Appendix II T-series 11/11 BIT-EXACT (enc+dec, all modes) + r439 Appendix IV PLC rebuilt fixed-point on the staged tables + STL operators (erasure-free prefixes bit-exact, SNR ≈13–14 dB, exactness floors raised); r442 FITTED smaller-pitch preference (user-ruled calibration vs staged vectors, labeled fitted): pitch ground truths 17/18 + 7/8 held-out, SNR ≈14–15 dB; residual attributed to the unstaged IV.7 instruction sequences |
| **G.723.1** | ✅ 100% | ✅ ~98% — both 5.3k + 6.3k, ITU-vector-verified wire (2816/2816 byte-identical repack) + near-bit-exact decode (PATHD53 corr 1.0000/+54 dB, OVERD53 0.9993; 3 vector-arbitrated interop corrections: LSP band 0 in the MSB byte, 1-subframe framer delay, unshifted Word16 output rail; 2 LPC→LSP bugs fixed) + §3.6 pitch postfilter; bit-exact OVER/TAME needs the clause-5 overflow protocol (trace ask filed) | 🟢 ~95% — full clause-2 chain with the printed §2.15/§2.16 searches (ACL±1 up to 99%, FCB subframe-exact 16→70%) + §2.3 fixed-point HPF + Annex A VAD/SID encoder (frame types 94%) + dtx fuzz; lacks bit-exact LSP-VQ/taming + CNG RNG |
| **G.728** | ✅ 100% | ✅ 100% |
| **G.729** | 🟢 ~92% — fixed-point §4.1 + §4.2 cascade + Annex A postfilter on the clause-5 grid; r461 exact §4.2.5 inversion oracle, eq (78) ×2 fix, eq (90) re-pin: corr ≥0.9988 on every clean vector, FIXED 57% byte-exact; lacks the eq (90) pole / §4.2.2 LSB schedule (#440) | 🟡 ~78% — fixed-point clause-3 chain + Annex B DTX/SID; locked-VAD frame-exact 27.5%; lacks bit-exact wire (same LSB gaps) + §B.3 VAD |
| **IMA-ADPCM (AMV)** | ✅ 100% | ✅ 100% |
| **MS-ADPCM / IMA-ADPCM (WAV)** | ✅ 100% | ✅ 100% |
| **G.726** | ✅ 100% | ✅ 100% — ITU Appendix II conformance-proven (112/112 byte-exact), A/µ-law + SYNC, registry law option + r437 G.72x-in-WAV sub-block bit-cell framing (registry framing=wav both directions, stereo lanes, fmt-extension, VBR rate-switching pinned; the long-standing WAV blocker retired) + r442 staged VBR demo reference BIT-EXACT (52 736/52 736 linear-leg words, 3 295 mid-stream switches; 16-sample frame period settled empirically) + r445 IMA reference-compressor interchange mode (Appendix D §6.1/§6.2 ladder, double-entry oracle) + r449 WAV tags 0x0045/0x0064 claimed with container-derived framing defaults (10 WAV tags + ima4 total across the adpcm crate; encoders advertise wire tag + fmt extradata) |
| **OKI / Dialogic VOX** | ✅ 100% | ✅ 100% — mono + Dialogic stereo (encode_packet_multi) + r449 OKIADPCMWAVEFORMAT wPole fmt-extension + WAV tag 0x0017 claimed |
| **8SVX** | ✅ 100% | ✅ 100% |
| **iLBC** (RFC 3951) | ✅ 100% | ✅ 100% |
| **AC-3** (Dolby Digital) | ✅ ~97% — AC-3 + Annex E + JOC/OAMD objects; r480 raw `.ac3`/`.eac3` demuxer, opt-in F32, dynrng no longer on LFE, 57–93 dB vs ffmpeg on a 120-cell broadcast matrix; lacks a 7.1 / ecpl external oracle | ✅ ~98% — all Annex E tools, within 0.5–3 dB of the reference at 192–384 kbps; lacks low-rate coupling, high-rate transient coding, AHT RD tuning |
| **AC-4** (Dolby) | 🚧 ~99% — channel / immersive / object decode incl. A-JOC + dialogue enhancement; r461 5.X/7.X A-CPL ACPL_1/2/3 synthesis on the coded carriers (the r456 silence finding closed: out/in RMS 0.78–1.12 per channel), fuzz-hardened; lacks §5.6 `d_pcm` output delay, LFE alignment on non-A-CPL A-SPX routes | 🚧 ~96% — framework Encoder, all layouts; r461 parametric 5.X/7.X A-CPL (ACPL_1/2, 5.X ACPL_3, `acpl` option); lacks short-frame ASF, rate control, encoder-side companding |
| **MIDI** (SMF) | ✅ ~99% — SMF 0/1/2 → PCM + soundfonts (SF2 LFOs/modulators/global zones) + native MIDI 2.0 mixer (16/32-bit, per-note bend/controllers, RC/NRPN, MIDI-CI profiles), .midi2 native playback | ✅ ~97% — SMF + .midi2 clip writers (fixed-point proven) + synthesis; lacks live-CC SF2 modulators + Standard Profile specs (unstaged) |
| **NSF** (NES) | 🚧 ~98% — full 6502 + 2A03 APU + six expansion chips + VRC7/OPLL pipeline incl. envelope ladder + rhythm mode + NSF v1/v2/NSFe + NSFe mixe per-device default mix + §8a VRC7/OPLL AM tremolo (silicon-measured 14-level truncated triangle, ≈4.8 dB) + VIB bit-exact (§8b 8×8 PM table) + Namco 163 sum/divide multi-channel mixing + YM2413 rhythm noise generator (23-bit x²³+x⁹+1 LFSR) + §7 EG rate-increment model (eg_shift/eg_select) + §9 phase-gen 10.9 fixed-point (correct VRC7/OPLL pitch) + end-to-end frame-render gate + §7 global-counter EG model wired into live decay/release + §7 global-counter-driven OPLL attack envelope (silicon-measured 12-level sequence) + VRC7 user-patch live-reload fix + 2A03 APU per-sample accuracy (full-rate noise LFSR clock + frame-counter 29830/33254 schedule + post-DAC HP/HP/LP filter chain + pulse bass-note sweep-mute fix) + cycle-exact frame counter (half-cycle events, 3-cycle IRQ window, 5-step cadence fix) + $4015/$4017 contracts + DMC DMA CPU stalls + pre-INIT scrub + all six expansion chips batch-invariant (VRC6 $9003 shift + MMC5 APU-rate pulses + S5B/N163/FDS remainder-carry + MMC5 polarity) + NSFe VRC7 patch-sets + time/fade schedule + r394 dedicated-page sweep muting (unconditional shift-0 adder mute) + load/reload DMC DMA cadence + $4014 OAM-DMA halt (was a no-op); + full NSF/NSF2/NSFe container read+write (lossless v1↔NSFe, well-formedness, UTF-8 strings; fixed a real wrong-starting-track bug) + typed per-track metadata; + r434 NSF2 §2.6 reconciled (forbidden-four typed, NEND advisory, $7C bit-7 semantics locked) + registry glue tested + track durations surfaced + r448 sub-instruction DMA engine (halt-cycle-exact DMC/OAM incl. the documented stop bugs, $4017 reset-delay parity fix, true-write-cycle register deferral, KIL clock semantics); lacks §7a attack recurrence + VRC7 rhythm phases (both docs-gapped; rhythm.md staging asked) + per-access 6502 read timing | — synthesis only |
| **Shorten** (.shn) | ✅ ~95% — fuzzed (5 daily targets; 6 defects fixed incl. a hostile-reservation OOM); lacks the spec-blocked filetype codes | ✅ ~90% — v2/v3 QLPC auto-select, property-tested round trips |
| **TTA** (True Audio) | ✅ ~99% — TTA1 fmt 1/2 + password + trailers + streaming + random-access + §04 decorrelation pinned vs captured reference tape (31-row §7.1 + N>2 cascade + end-to-end pipeline) + §4.3 unseekable-mode (corrupt seek-table CRC → linear-only, random-access refused) + §4.4 empty-stream + fuzz; decode −18% then r386 −17…−25% more (u64 reader + slice-by-8 CRC), byte-identical | ✅ ~97% — bit-exact self-roundtrip, wire-identical to the external encoder (black-box), decode +12% r456, 4 fuzz targets; lacks format=3 float (spec undefined — ruling asked) |
| **Musepack** | ✅ ~98% — SV7+SV8 ±1 LSB vs oracles; seek-table thinning + ID3v2/APEv2 tag pass-through | ✅ ~95% — from-PCM SV7+SV8 at 1-LSB reference parity; SMR quality ladder; dual-generation typed registry options |
| **Cook** (RealMedia) | 🚧 ~70% — r450 §0.2 pre-spectral walk + §2.2 stage-2 refinement vendor-exact (categories 34/34 bit-exact on all traced live frames; synthetic frames → non-silent PCM; multi-frame streaming) — blocked on unstaged envelope/coupling VLC trees (asks filed); flavor/cookie parsers + every extracted DSP table behind typed range-guarded APIs + decode-session orchestrator + per-band quantiser primitives + backend frame-syntax codebook/vector-dim geometry + joint-stereo mirror-index rotation + MSB-first frame bit reader (spec/05) + §1 gain-control envelope (segment-count + √2 ladder) + §0–§3 frame-body decode orchestrator (gain application + §2.1 subband geometry) driven on a real RA Cook stream + §2.2 full per-band quantiser closed form (clip·round·divisor) + §3.1 dequant scale triple + spectral-coefficient assembly + §3.1 per-band symbol/coefficient grouping arithmetic + §3.1 per-band/spectrum reconstruction + §4.2 joint-stereo decouple + frame-body integration (FrameSpectrum mono/stereo) + §2.2 division-free quantiser-index decomposition + §4.1 coupling-control read + full §5 synthesis back end (O(N log N) IMLT → Princen-Bradley window → gain → overlap-add → 16-bit PCM; per-call cadence byte-exact vs validator) + §2.2 category-assignment/bit-allocation loop recovered + wired (validator-exact base pass 25/25 + uniform refinement; real-frame decode blocked on stack-resident v[]/budget/M + §1.2 gain VLC) + §3.2 spectral codebooks vendored (all 1301 symbols round-trip, 7 codebooks) + §3.1 VLC walk + §2.2 cost LUT + entropy→digit bridge + §3.1 band decode (codebook-by-category, magnitude+sign) + level→value expectation rows + recovered §4.3 coupling + N=1024 window synthesis + registry/make_decoder — ~86%; entropy→PCM audible; r433 §2.2 category-assignment constants vendored+bit-locked (last unwired staged table); real-stream PCM gated on the pre-spectral read layout (v[] formation + gain/quant VLC selection; ask filed) | — |
| **WMA** | ✅ ~95% — vendor v2 decode 1763/1763 boundaries, LSP-envelope family bit-exact (3 931 vendor conversions), validated dequantisers + cutoff/LCG; lacks v1 stream, window/abs-scale read | 🟢 ~90% — vendor-wire encoder: 47 catalogue cells black-box accepted (VLC + LSP envelopes), masking/M-S/noise-sub, reservoir RC; lacks LSP short-block/stereo vendor evidence |
| **WavPack** | ✅ ~100% decode (post-2026-05-18 orphan) — v4 block/metadata/entropy parse + full §4.2 entropy ladder + multi-block PCM composer + inverse entropy encoder (bit-exact round-trip) + r447 ENCODE ORIGINATION MATRIX CLOSED — every format × channel width, lossless + hybrid pair (stereo-pair member mode search 50.8–61.1%, float/int32 + hybrid multichannel, registry-wired with per-member-chain carries; §4.4/§4.4.1 shaping spec-pinned closing #309; 90 black-box bit-exact checks) + typed-refuses joint-stereo/cross-decorrelation (flag bits 4/5) + §5 running block-CRC compute/verify + §3.1/§3.4/§3.5 decorrelation inverse-prediction weight arithmetic (apply/update_weight primitives) + §3.2/§3.7 decorrelation inverse-prediction loop (all terms, mono+stereo, round-trip-pinned) + mono-lossless decode_samples → reconstructed PCM (§3.7 reverse-storage + multi-pass assembly) + §5 decoded-CRC verify + extension-stream CRC accumulator (crc_x / ExtensionCrc) + §3.2/§3.7 stereo decorrelation prediction loop + joint-stereo undo wired into block decode + §5.6 stream-level CRC mute gate + §1 left-shift final-normalization fixup (sub-byte 12/20-bit depths, CRC folds pre-shift) + §4.1 hybrid correction-fold arithmetic (fold/split + placement selector, block-level) + multichannel grouping decode (member-set channel interleave, bit-exact) + foreign-file decode bit-exact (19/19 reference-encoded fixtures via wp_log2/exp2 + 0x04 seed-prefix rule) + float (0x08, wvx layouts + crc_x erratum found; SHIFT_SAME + EXCEPTIONS sentinel/NaN layouts) + int32 (0x09/0x0C) + sample-rate surface (0x27, seek_seconds) + typed ChannelInfo (0x0D) + hybrid lossy decode end-to-end (§6.5 error_limit/bitrate model, mono/stereo/5.1/24-bit/float, black-box bit-exact) + hybrid-lossless wv+wvc pair decode (0x0B bracket completion + 0x07 shaping; pairs reproduce original encoder input bit-exactly) + false-stereo output fix + joint-stereo/float/int32/multichannel hybrid-lossless wv+wvc pairs bit-exact (54 fixtures / 18 pairs; pair-CRC gate, pair-aware seek, differential pair fuzz + 2 overflow fixes) — every reference-encoder shape decodes bit-exact; lacks DSD (docs-gapped) | ✅ ~97% — lossless + hybrid + float/int32 encode with 0x07 noise-shaping origination (Off/Static/Ramp, .wvc correction pairs) + registry-wired options; 36/36 reference-bit-exact battery incl. r418 hybrid edge probes + ramp rail-saturation pin + r436 hostile-input decode budgets on every eager surface (typed pre-decode refusal, per-packet registry budget, bounded seek, ~1.2M-run differential fuzz clean); lacks DSD + negative-IIR-past-rail (both docs-gapped) |
| **APE** (Monkey's Audio) | ✅ ~95% — 3990 decode byte-exact both directions vs the vendor corpus (levels 1000–5000); lacks pre-3990 archive proof | ✅ ~90% — full 3990 encoder, all levels at 8/16/24-bit, vendor-verified (280/280); lacks pre-3990 + multichannel |
| **DTS** (Core) | ✅ ~98% — Core complete; output level reference-calibrated | 🟢 ~85% — Core encoder: QMF/LFE front end, ADPCM, HF-VQ, transients, Huffman, all rates/layouts, DRC/joint/aux opt-in, oracle-validated; lacks the unprinted SA129–SE129 books (docs ask) |
| **aptX** (classic + HD) | 🚧 stub — NDA-blocked; clean-room QMF + 4-subband quantiser source-of-record purged in the 2026-05-06 audit (trace docs failed clean-room separation). Awaiting a non-contaminated `docs/audio/aptx/` (public-primary-source tables or black-box observer trace) | — |

</details>

<details>
<summary><strong>Video</strong> (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **MJPEG** | ✅ 100% — every SOFn family (Huffman + arithmetic, 8/12-bit, CMYK/YCCK, RTP, DNL), ICC/Exif/XMP on frames; r480 opt-in djpeg-matching mode (bit-exact: integer IDCT, triangular chroma, fixed-point YCbCr), arithmetic colour fixed | ✅ 100% — general T.81 writer, 19 layouts, metadata both ways; lacks Extended XMP + IPTC (docs) |
| **FFV1** | ✅ 100% | ✅ 100% |
| **MPEG-1 video** | ✅ ~96% — whole-stream I/P/B/D reference-conformant, arbitrary slice structures, r480 streaming decode shared with MPEG-2; lacks 11172-2 scalability lower layers | 🚧 ~90% — I/P/B/D, GOP assembly, Annex C CBR rate control with exact VBV; encoder feature-complete |
| **MPEG-2 video** | ✅ ~97% — 12/12 conformance + data partitioning + scalable two-layer loops; r480 streaming decode (5-min 720p peak 49 MB, was 12.5 GB), coded-order / AU mode, opt-in concealment; lacks interlaced spatial weight tables, chroma_simulcast | ✅ ~95% — interlaced, 3:2 pulldown, 4:2:2/4:4:4, scalable enhancement encoders; lacks field pictures in layered loops |
| **MPEG-4 Part 2** | ✅ ~97% — 24-stream corpus + GMC/DP/RVLC/interlaced + short header; spec default with opt-in ecosystem compat (r480: even IDCT-half rounding), coded-order output, `output_pixel_format`; lacks integer-IDCT compat | 🟢 ~93% — I/P/B/S(GMC) + interlaced + short header, two-pass RC ±0.2–4%; lacks static-sprite / N-bit encode |
| **Theora** | ✅ 100% | ✅ ~97% — RDOQ EOB search, MV candidates + iterated half-pel, lookahead scene-cut/VBV RC, elected intra matrices (−15% BD-rate vs r453, 1.5× faster); lacks a reference-encoder comparison (no encoder binary available) |
| **H.263** | ✅ ~99% (post-2026-05-18 orphan) — r450 registry wired (streaming decoder/encoder adapters, dual-API factories, tags + payload magics) + Annex R ISD + Annex V DPS + Annex L/W.6 SEI + §W.5.3 fixed-point IDCT0/FDCT oracle-exact + 2 fuzz DoS fixes (lacks Annex U); r447 UMV+ Table-D.3 nonconformance CLOSED both directions (fixture + black-box proven; UUI "Unlimited" range decode, RRU×UMV pseudo-vectors, AP×UMV+ four-vector emission, §P.2.2 RPRP emulation-prevention fix) + baseline + Annexes D/F/I/J + OBMC + PLUSPTYPE + Annex G PB-frames + Annex K slice-structured + Annex M improved-PB-frames + Annex T modified-quantization (MQ-active picture decode end-to-end on baseline + Annex I AIC paths: §T.2 DQUANT + §T.3 QUANT_C + §T.4 extended-range) + Annex S Alt INTER VLC (§S.2/§S.3) + Annex Q reduced-resolution update (§Q.6 prediction-error upsample + §Q.7 block-boundary filter) + §5.2.2 first-GOB header elision + §5.1.11–§5.1.16 PLUSPTYPE scalability (Annex N/O/P enhancement-layer header) + Annex O scalability macroblock-layer VLCs (Tables O.1–O.4) + Annex O EI + EP enhancement-layer end-to-end reconstruction + Annex N forward-channel RPS to pixels (§N.5 store + §N.4.1 picture/per-GOB/per-slice NEWPRED TRP re-selection) + Annex P RPR (implicit §P.1 + explicit §P.2 reference resampling/warp → pixels) + §5.1.3 decode_sequence PLUSPTYPE picture dispatch (extended-PTYPE end-to-end + custom-PCF accept) + Annex-K slice PEI/SEPB1 ordering fix + Annex T MQ + Annex S AIV threaded through the slice driver + 6 mode-coverage byte-exact conformance fixtures + reference-encoder-fixture baseline decode conformance; + Annex G PB-frames + Annex M Improved-PB streamed through `decode_sequence` + Deblocking-Filter-mode four motion vectors (§5.3.8/Table J.1, no-OBMC INTER4V); + r443 Figure-F.1 predictor fixes + skip-MB OBMC (spec default, compat flag) + Annex Q RRU decode + §N.4.2 BCM parse; lacks EP-picture lower-layer RPRP | 🟡 ~91% — Annex M Improved PB + AP/UMV/AIC/K/CPM/RRU-DF composed both ways, oracle ≤1 LSB; lacks RRU+AP/K, SAC+Improved-PB, EP CPM/AP/SAC/K |
| **H.261** | ✅ ~99% — I+P + loop filter + BCH error correction + RTP/RTCP/SDP + Annex A + Annex C conformance + fuzz-hardened (6 targets; packetiser MV-desync panic fixed) | ✅ ~98% — ME + rate control + §3.4 forced-update cyclic INTRA refresh + §4.2.3.1 MBA-stuffing emit/pad (§5.2 HRD buffer regulation) + BCH/RTP framing + §3.1/§4.2.1.2 temporal-reference + picture-rate (TR tracking, picture-rate-driven encode, §4.3.1 freeze clock); 45 dB at 64 kbit/s QCIF |
| **MS-MPEG-4** (v1/v2/v3) | 🚧 ~95% — v1/v2/v3 decode end-to-end, exact spec IDCT; r480 MC references span the MB-aligned coded area (170×98 within 2 of ffmpeg); lacks v1/v2 intra parity on ffmpeg streams, clamp-extent trace (docs) | ✅ ~92% — v1/v2/v3 I+P decoder-verified |
| **H.264** | ✅ ~98% — all JVT streams byte-exact; r480 8 external PRs: MBAFF 1080i P/B MD5-identical to ffmpeg, slice + deblock threading; lacks frame threading | ✅ ~98% — PAFF + MBAFF, 8–14-bit CAVLC/CABAC + lossless, 12 registry layouts byte-exact vs a black-box decoder; lacks B in MBAFF, 4:0:0, Intra_8x8 above 8-bit, High-family B / RC |
| **H.265 (HEVC)** | ✅ ~99% — conformance surface byte-exact (RExt 46/46, MV-HEVC), HEIF sweep 236/257 items; r480 1080p inter ~10× faster single-threaded (#16), missing references named; lacks SCC recon exactness, CGS, SHVC conformance | 🟢 ~96% — intra/inter RDOQ + RC, every lossy HEIC layout byte-exact, WPP; r480 speed-first mode (~6 fps 1080p on 8 threads), forced keyframes, TRAIL_N, level/tier + bitrate retargeting; lacks 16-bit, SCC tools |
| **H.266 (VVC)** | ✅ 100% — 56/56 JVET streams byte-exact (subpictures, 4:2:2/4:4:4, IBC + SCIPU dual-tree chroma); r477 through the registry too: streaming Annex B / `vvcC` packets, POC output order, truthful 8/10-bit labels, limits before allocation, 56/56 match in output order; 3-target cargo-fuzz sub-crate + daily Fuzz (8 hostile-input fixes); lacks multilayer | 🟡 ~75% — stills encoder on the registry: 8 declared layouts, `quality` → QP (100 = lossless), `format=annexb|vvcc` with the `vvcC` record as extradata; r477 MIP elected by default (−1 % bytes at +0.03 dB), lossless BDPCM (−10…−28 %), ISP conformant but opt-in (its RD does not price the partition-edge deblocking); lacks production inter, VUI (H.274 not staged, #447), lossy BDPCM |
| **VP6** | 🚧 ~92% — r450 P-frame MB(0,31) inter MC+recon pixel-exact gate (§11.4 fractional-pel + inter residual on vendor coefficients; §10 mode grammar synthesis-confirmed, §11.1 MV wire fixture-under-determined — extraction asked); §16 IDCT BIT-EXACT (555-block arbitration; r390 rounding erratum corrected) + r447 §13 Table-18 MSB-first extra-bit erratum (fixture-arbitrated) + P-frame beachhead (arithmetic tokens vendor-wire-pinned through the 189-block prefix + first content MB; 31-MB pixel-exact prefix; §10/§11 P-frame wire extraction-asked) + keyframe parse-exact prefix 12→31 MBs via black-box bit-flip probing (chroma DC tree/+128-seed findings) + BoolCoder + DC/AC coefficient decode + MV decode/reconstruction + custom scan + per-block reconstruction + §11.4 FilterVarThresh resolve + §11.5 variance edge-clamp + header-driven FilterConfig + §2/§13/§17 block-to-plane raster frame assembly + §9 output-scaling typed surface + §10 macroblock coding-mode traversal + §10 frame-level MB mode-decode pass (availability-aware prob-row select) + §9 BoolCoder frame-header tail + §7.3 BoolEncoder (decoder-matched range encoder) + §10/§11 per-MB MV resolution (Zero/New/Nearest/Near) + §16 inverse DCT (i64-widened descale) + §17.1 per-MB intra decode loop (I-frame decodes END-TO-END to output pixels) + §17.2/.3 integer-MV inter recon + FourMV macroblock resolution + §17.4 sub-pixel MC predictor (§11.4 bilinear/bicubic) + §11.3 loop filter (BoundaryX/Y round-toward-zero fix) + fused inter (P-frame) per-MB decode driver → END-TO-END pixels + §4 golden-frame ReferenceFrames bookkeeping + top-level per-frame Vp6Decoder assembly driver (§9 header→keyframe/inter dispatch) + registry `Decoder` (codec id vp6, tags VP60/VP61/VP62/vp6f) + §8 Figure-1/Figure-5 coeff-prob-update sub-stream ordering (I+P, real NewNodeProbValue round-trip) + keyframe→P GOP end-to-end + r439 full-frame Huffman coefficient decode (Extractor-03 band selectors + corrected prob banks + §7.2.1 tie-break + carry-forward fill; §13.3.1 Prec-seed, §14 truncation and chroma-seed errata fixed): real vp6f keyframe PIXEL-EXACT end-to-end, 854×480 all 9 720 blocks, CI-gated | 🚧 ~70% — P-frame encode (encode_inter_frame all-CODE_INTER_NO_MV + §10 mode_encode; keyframe→P GOP round-trip) + intra (I-frame) encode: §16-dual forward DCT + quantise + §13 token coding (DC/AC trees + §13.3.3.1 zero-run) + §9/§14 header emit → decoder-reconstructible keyframe (flat-exact; ~44 dB at q=48) + §11.1 MV component encoder (decode_mv inverse) + motion-estimated P-frame encode (two-stage box+¼-pel SAD ME → CODE_INTER_PLUS_MV/NEAREST/NEAR, decoder-reconstructible) + self-describing ME packets + Golden-frame + FourMV encode modes + bit-budget rate control + oxideav-core Encoder registration + §13 Huffman coefficient coder (decode+encode) + §6 MultiStream two-partition transport (every P shape, bit-identical to single-stream) + §13 P-frame prob re-training + cross-frame bank persistence + errata #155 FourMV representative + third-party vp6f fixture: 4 printed-spec errata fixed (MB-unit geometry, partition-1 spans, DC-tree fold, IDCT rounding) + leading-MB pixel-exact real-stream decode, CI-gated; + r439 multistream Huffman keyframe encode with carry-encoded retrained banks (round-tripped); + r442 §9 output scaling BOTH ways (typed placement modes, downsampled encode, letterbox/center gates; §14/Prec-seed errata spec-prose-corroborated); lacks P-frame wire conformance (behavioural trace ask filed) |
| **VP8** | ✅ 100% | ✅ 100% |
| **VP9** | ✅ ~98% — 41/41 corpus byte-exact, 12-format matrix; r480 6 external PRs: 1080p 65 fps on 8 threads | ✅ ~99% — §8.4 adaptation, two-pass RC, RDOQ (−4.8% bytes); lacks pareto-tail forward-update scoring, downscaled first pass |
| **AV1** | ✅ 100% — 155-stream corpus + 114/114 stills byte-exact; r480 frame-parallel + tile/post-filter threaded decode, 1080p 170 fps on 8 workers; lacks u16 in-loop + SIMD kernels | 🟢 ~99% — GOP drivers corpus 164 byte-exact; still encoder 12 MP Fast 3.0 s on 8 workers at +7.9% BD-rate vs libaom s6, peak RSS 221–259 MiB; lacks 8-bit sample storage, compound inter-layer |
| **Dirac / VC-2** | ✅ ~99% — VC-2 LD+HQ + Dirac intra/inter + OBMC + 7 wavelets + 10/12-bit + fragmented pictures + asymmetric transforms + §15.8.10 reference-exact out-of-frame sub-pel edge extension + DEEP COLOUR to 16-bit (>12-bit on Yuv*P16Le, 11/12-bit native P12Le; corpus 12/12 bit-exact incl. 16-bit fragments) + DIRAC_TRACE MV/plane tracing per the docs trace contract | ✅ ~99% — HQ+LD + sub-pel 2-ref bipred + rate control + inter sequence-level rate driver (PerPicture/Cbr/Vbv/VbvHysteresis residue-byte) + §11.3.3 spatial-partition codeblock grid (per-codeblock differential quantiser; bit-exact ≥4×4-sample; 1-ref + bipred; codeblock-aware rate control) + asymmetric transforms + §11.2.6 global-motion encode (P/B/sequence-driver + per-block gmode + pan auto-fit from ME grid; oracle bit-exact cross-decode + 120-case fuzz) + affine/perspective global-model estimation (−31% zoom; estimated-pan oracle bit-exact) + §14 fragment emitter (bit-exact reassembly) + pan_tilt_all zero-matrix fix + §15.8.5 per-component intra-DC fix + 1-ref/bipred inter chains cross-decode bit-exact (literal block params + explicit zero-residue route around two reference-decoder quirks) + 13–16-bit deep-colour intra AND INTER + rate control (u16 P/B incl. codeblock grid + global motion + all four residue-rate drivers; 10/10 roundtrips q0 bit-exact at 10/12/16-bit, 16-bit Cbr within ±2.4% of target; deep inter self-validated — the external validator corrupts >8-bit inter chains it accepts) + r436 closed-loop reference-P chains + I/P/B GOP driver (8–16-bit, 4-variant rate control) + legacy container tags co-claimed (V_DIRAC/drac, 0.95 long-GOP probe); lacks third-party deep-inter ground truth (docs-asked) |
| **VC-2** (standalone `oxideav-vc2`) | ✅ ST 2042-1 intra decode complete — §14 fragment reassembly (bit-identical to unfragmented) + full Annex D (D.1–D.8 incl. asymmetric) + registered `Decoder` (8–16-bit YUV incl. Table-10 presets 7/8 on Yuv*P16Le) + MIXED ≤12-bit custom depths via the core significant-bits side-channel (represent-don't-promote ruling; 12/10 fixture pinned) + FourCC `BBCD` tag claimed + r435 registry-grounded container tags (Matroska V_DIRAC + MP4 drac / OTI 0xA4; weak probe so Dirac wins contested streams) + Table-10 presets 1–8 grounded from the staged transcription + full §11.4 source-parameter retention + ST 2042-2 levels + §12.2 conformance walker + ST 2042-4 MXF mapping data + 8-fixture matrix + truncation/DoS-hardened | ✅ — full ST 2042-1 encoder: LD+HQ, all 7 wavelets, asymmetric, fragments, rate control; 11/14 black-box bit-exact (rest = validator envelope) |
| **AMV video** | ✅ ~92% — r451 both device profiles conformant (2nd fixture 2928/2928 + graded duration cross-check; §4b step-index settled u8@+2) + hostile chunk-size prealloc capped (fuzz find #3) + geometry×fps corpus matrix; typed frame-geometry binding + §4 demuxer 1:1 video:audio interleave cross-check + §4a device-stripped JPEG reconstruction (splices Annex K DQT/DHT + baseline SOF0 4:2:0 → conforming JFIF) + full baseline-JPEG frame decode to RGB (device-hardcoded video tables wired, #127) + r432 selectable triangle chroma upsampling (MAE 0.04/ch vs black-box reference, 30×) + 2 fuzz-found hostile-input crash fixes | 🚧 ~85% — §4a RGB→00dc device-locked baseline-JPEG video encoder + §4b PCM→01wb IMA-ADPCM audio encoder + full decode→encode→`AmvMuxer`→demux→decode round-trip on comedian.amv (1116/1116 chunks, video MAE <3/ch, audio byte-idempotent) + intrinsic codecs registry-wired (amv_video + adpcm_amv as oxideav-core Decoder/Encoder; demuxer declares ids so the pipeline auto-resolves) + native planar YUV420P decode/encode path (skips the lossy RGB hop) + video rate control (measured bitrate targeting, >94% budget utilization) + device-envelope muxer validation + 9× encode speedup (precomputed cosine basis) + fuzz + r432 exact Lagrangian RD coefficient planning (worst sampled MAE 6.87→4.69/ch at identical rate) + λ warm start (255 µs steady-state binding encode); lacks second-device fixture + §4b non-zero initialStepIndex confirmation |
| **ProRes** | ✅ 100% | ✅ 100% |
| **EVC** (MPEG-5) | 🟢 ~90% — Baseline + Main tools, §8.3 RPL bookkeeping, errata #313; 23094-4 gate red on filed entropy asks | 🟢 ~88% — intra/P/B + hierarchical B, BTT, IQT/ATS, ALF, EIPD, exact-bit RD + RDOQ, one-/two-pass RC (ADCC writer lands default-off); lacks CU QP delta/AQ, ADCC trellis |
| **HuffYUV** / FFVHuff | ✅ ~97% — HFYU/FFVH + 6 predictors + interlaced + fast-LUT decode + fuzz; r419 profile round: decode wall −13..−23% (173–247 MiB/s), 78 golden pins byte-invariant; r420 two-worker interlaced decode 1.3–1.5× under ExecutionContext (serial default, 78 pins byte-invariant, differential fuzz oracles); r429 serial decode −23..−33% (full pair LUTs; r440 RGB24 phase-flip pairing — every codeword on pair-LUT reads, 2-worker 720p decode 1.35×, serial≡parallel pinned) | ✅ ~97% — v1.x + v2.x symmetric encode across YUY2/RGB24/RGB32; encode 1.9–2.9× faster (ClassicV2 up to ~1 GiB/s), HD bench gates; two-worker interlaced encode 1.7–2.0×; r429 full dual-API (registry Encoder + 78-pin trait-path byte-lock), budget-threaded auto encode 2.15×@3w; r440 lean CustomV2 tables (encode −7..−10%) — perf near-saturated, remaining levers Amdahl/entropy-bounded |
| **Lagarith** | ✅ 100% — vendor corpus 187/188 byte-exact (solid-plane 0xff rule, Rule B, lazy RLE zero-count, pow2 normaliser 189/189), 42/52 vendor-lossy reproduced, fuzzed; lacks W≥4 DIB pad bytes + odd-width YUY2 vendor validation | ✅ ~95% — all 11 wire types, 45/179 vendor-identical (never larger than the vendor), NULL emission, PTS passthrough |
| **Ut Video** | ✅ ~98% — 5 FourCCs × 4 predictors + slice-parallel decode (5.6× at 720p) + opt-in strict-padding conformance decode + panic-free inspector Kraft accessors + 19-fixture reference-golden byte-exact corpus (spec/04 §5.0 median-mode modular-gradient fix); r420 ExecutionContext contract (serial default, budgeted slice-parallel 5.8×@4, 19-fixture invariance-proven) | ✅ ~98% — slice-parallel encode (3.3×) + reference-decoder-verified interop (incl. gradient mode 2) + zero-length-slice interop guard + fuzz oracle + reference-seeded fuzz corpora; budgeted encode 3.5×@4 under the same contract |
| **MagicYUV** | ✅ 100% | ✅ 100% |
| **Cinepak** (CVID) | ✅ ~98% — full CVID intra/inter + Sega FILM demuxer + Saturn/3DO deviants + typed walkers + fuzz + 42-rule spec-cited wire-format conformance linter (frame→strip→chunk→vector, vintage/seek profiles; own encoder held to zero findings); decode 4.4 GiB/s | ✅ ~98% — rolling codebooks + RDO/LBG + rate control + skip-free-aware 0x3000/0x3200 inter vector dispatch + encode-roundtrip fuzz; 34.2 dB PSNR; r430 profile round: encoder hot paths −23…−31% (output-invariant, 15-scenario golden-pinned) |
| **SVQ1/SVQ3** (Sorenson) | 🚧 ~97% — SVQ1 complete + 9-target fuzz; SVQ3 I/P decode byte-exact 10/10 fixture AUs incl. the vendor post-reconstruction edge filter (spec/09) + extended-mode read; lacks B slices / extended-mode MB layer (docs-unspecified) | ✅ SVQ1 encoder (unchanged) |
| **Indeo 3** (IV31/IV32) | ✅ ~95% — both IV32 fixtures 16/16 frames pixel-exact (intra + inter with MC, cell-geometry banks, families A/E), fuzzed; lacks families C/D + a >320-wide fixture | — |
| **Indeo 2/4/5** | 🚧 ~75% — IV50 intra byte-exact host buffer (YUY2 153 600/153 600) + 8/8 checksums on both fixtures (quantiser banks 288/288, tile phases, DC chain), fuzzed; lacks an IV50 inter fixture + IV20/IV41 specs | — |

</details>

<details>
<summary><strong>Image</strong> (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **PNG / APNG** | ✅ 100% | ✅ 100% |
| **GIF** | ✅ 100% | ✅ 100% |
| **WebP** (VP8 + VP8L) | ✅ 100% | ✅ 100% |
| **JPEG** (still) | ✅ ~99% — via `oxideav-mjpeg`; contract API (probe/info/decode, exact T.871 colour, `YuvJ*` native, limits); r477 ICC (APP2 chunks reassembled) / Exif / XMP as core blobs on stream + frame, `Metadata` ≡ blobs; lacks Extended XMP, IPTC | ✅ ~99% — via `oxideav-mjpeg`; registry ≡ contract bytes; 19 declared layouts (heic → jpg works); r477 writes ICC/Exif/XMP from `params.blobs` (frame over stream) in spec order with typed size limits; lacks arithmetic / hierarchical via options, Extended XMP |
| **TIFF** (6.0) | ✅ 100% — contract API, every page (r477: one frame per page through the registry, each page's ICC/Exif/XMP/IPTC on its frame record, the first page's on the stream; IPTC tag 33723 new on every surface), native Pal8 / Rgba / Cmyk / Gray / Rgb48 / float, JPEG- and WebP-in-TIFF, §20 colorimetry; lacks Exif/GPS tag semantics (docs ask), a timed-sequence shape for multi-page files | ✅ 100% — contract `encode*`; r477 registry encoder declares 9 layouts with typed refusals, `quality` governs JPEG only (1–99 DCT, 100 = lossless SOF3; it used to switch every save to JPEG), blobs written from both levels, multi-stream muxer writes mixed-geometry pages in order; `registry_encode` fuzz; lacks GeoTIFF/DNG semantics (#408), WhitePoint/PrimaryChromaticities writing |
| **BMP** | ✅ ~98% — contract API, native Pal8/555/565/Bgr24/Bgra, every header generation, RLE4/8, ICC, 8-target fuzz; lacks V4 gamma curve mapping (docs) | ✅ ~98% — all 10 layouts, RLE auto, bitfields / V4 / V5 ICC; r480 9 external PRs: zero-copy `encode_into`/`encode_to`, byte-identity pin; lacks sips interop on V4/V5 |
| **Netpbm** (PBM/PGM/PPM/PNM/PAM) | ✅ ~98% — image-crate contract API (probe/info/decode/decode_all, RGB8/RGBA8 raw); all 9 magics at 1/8/16-bit, 6 PAM TUPLTYPEs, PFM native float (`GrayF32Le`/`RgbF32Le`), fast paths ~45–50 GiB/s; `strict` rejects over-MAXVAL samples | ✅ ~98% — contract `encode*` with flavour/maxval/tupltype `EncodeOptions`; `encode_rgb8` → P6 and `encode_rgba8` → P7 pinned (the r463 1-bit PPM bug cannot recur) |
| **ICO / CUR / ANI** | ✅ ~98% — image-crate contract API (probe/info/decode/decode_all, RGB8/RGBA8 raw); every entry decodes standalone (DIB via bmp, PNG via png), composed `Rgba` output, best-entry selection, CUR hotspots, ANI model + demuxer; fuzzed | ✅ ~98% — contract `encode`/`encode_all` (PNG-above-256 rule, mixed depths), 1/4/8/24/32-bpp DIB + PNG write, AND masks, ANI write; lossless round-trip pinned |
| **JPEG 2000** | ✅ ~98% — contract API, 40 layouts, Part 1 + HTJ2K, registry `.j2k`/`.jp2`; r480 10 external PRs: 4k decode 2–3× faster, −35% memory; lacks multi-threaded Tier-1 | ✅ ~98% — JP2/J2K, 5/3 + 9/7, PCRD layers, HT streams accepted by OpenJPEG; lacks PLM |
| **JPEG XL** | ✅ ~99% — contract API, core optional; Modular + VarDCT, 1–16-bit, animation, byte-exact JPEG reconstruction on every lattice; r477 ICC/Exif/XMP as core blobs on stream + frame (reconstructed JPEGs' APP1/APP2 included), Table F.2 `save_before_ct` presence fixed; lacks float samples | 🟢 ~60% — r477 Modular encoder born: lossless at quality 100, quantised residual ladder below (q0 < q25 < … < q100 strictly), YCoCg RCT, per-channel predictors, ANS entropy streams (rANS), ICC in-codestream + Exif/XMP boxes, animations, 8 declared layouts; lossless sizes between a black-box encoder's efforts 1–3; `jpegxl` muxer splices stills into animations; lacks VarDCT (lossy XYB), WP predictor, Palette/Squeeze |
| **JPEG XS** | ✅ 100% — 65/65 21122-4 codestreams sample-exact, 9 ETS profiles, Nc 1..=8; r480 `jpvi` interlace frame-level semantics + reserved-code tolerance; lacks B>16, 4:2:0 schar (21122-3:2024 not staged) | ✅ 100% (B≤16) — profiles/levels/CBR, RCT/Star-Tetrix, NLT, WGT |
| **AVIF** | ✅ ~99% — image-crate contract API (`probe`/`info` standalone; `decode*` via `registry`; `AvifImage` pixels, `AvifFile` container); 8/10/12-bit all layouts, alpha, grid/overlay/iden/tmap within 1 code, `avis`, ColorSignal; fuzzed; r476 decode takes the execution budget (12 MP tiled 0.34 → 0.07 s on 8 workers, grid tiles in parallel, byte-identical) | ✅ ~99% — contract `encode*`; stills/grid/overlay/iden/layered/alpha/depth, `avis` via heif's SequenceWriter, gain-map authoring, 27 declared input layouts, option schemas; r478 encoder to production: default quality 60 (its lossless default wrote 12 MP as 20 MB), automatic 512-px grid above 4 MP under the thread budget (12 MP q60 8.0 s / 465 MiB → 1.2 s / 162 MiB on 8 workers, +2.8 % bytes, same PSNR), ICC/Exif/XMP from frame/stream blobs through the registry, `prem`, 21 shapes × 5 black-box readers open, fuzz target (2 finds fixed); lacks a shared `av1C` across items (heif writer, readers' 100-box `ipco` cap) and zero-copy planar input |
| **HEIF / HEIC** (ISO/IEC 23008-12 + MIAF) | ✅ 100% — contract API; final Amd 1/Amd 2/MIAF Amd 1 texts, gain maps, `mini`, tiled items, L-HEVC stereo, packed `cfen`; 67/67 producer cells byte-exact, 152-file decode matrix byte-identical on both paths; AV1 items under the thread budget (12 MP AVIF 0.10 s on 8 workers); ICC/Exif/XMP as core blobs, sequence-level Exif/XMP (track `meta` + timed `Exif`/`metx` tracks) on frames / stream blobs / `exif`+`xmp` data streams; r479 `moov`/`udta` and track `cprt` reported as `copyright[:lang]` metadata; lacks `cfen` CMYK composition (#446), L-HEVC enhancement-layer decode | ✅ ~99% — HEVC/AV1/VVC/AVC stills + sequences, 724-cell encoder matrix, framework `quality` on every codec, MIAF / Annex A brands claimed only when earned, re-mux without re-encode, overlays / `iden` chains / depth maps, 36 black-box reader interop rows, `tili` on request, sequence-level Exif/XMP both ways, 81-file production soak at 1/4/8 workers (12 MP grid 0.097 s, RSS ≤ 76 MiB); r479 the `ipco` is a pool: every distinct property box (decoder configurations included) interned once — a 12 MP 256-px grid goes from 196 entries to 5, a 1 024-tile grid from 1 028 to 5, so the readers that refused at their 100-box `ipco` limit now open it (the 1 024-tile file stays behind their 1 000-item `iloc` ceiling), 15-bit `ipma` indices verified both sides of 127; degenerate items refused at write time (empty bodies, 0-row grids, input-less overlays); core 0.1.41 `embedded_blob_kinds` = ICC / Exif / XMP and sequence `set_metadata` → `moov`/`udta` and track `cprt` (the only spec-defined string carriage); lacks AVC 4:0:0, external `tili` tiles |
| **DDS** | ✅ ~99% — image-crate contract API; every D3DFMT/DXGI layout decodes natively (BC1–7, BC6H, ASTC blocks, float, SNORM, packed) to core layouts, mips/cubemaps/arrays/volumes via `decode_all`; UINT/SINT/depth/YUV stay depth-API; fixed volume mip cap + forged-width stride overflow | ✅ ~99% — contract `encode`/`encode_all` (natural or explicit stored layout incl. BC1–7/BC6H/ASTC encoders, generated mip chains, cubemap/array/volume from Frame extras) |
| **OpenEXR** | ✅ 100% — image-crate contract API (`RgbF32Le`/`RgbaF32Le`/`GrayF32Le` view, `decode_all` parts, chromaticities → H.273); scanline + tiled + deep + multi-part, all codecs via compcol zlib; HALF → f32 exact; registry container: one stream per part layout, multi-part byte-level repack, exrinfo-validated | ✅ ~99% — contract `encode`/`encode_all` (pixel type, 10 schemes, luma-chroma, tiled mip/rip, line order); per-channel depth writers kept; lacks per-layer multi-frame emission (core ask) |
| **Farbfeld** | ✅ 100% | ✅ 100% |
| **HDR** (Radiance RGBE) | ✅ ~99% — image-crate contract API (native `RgbF32Le`, `to_rgb8` clamp, strict mode); new/old RLE, 8 orientations, header metadata, derived colorimetry, scene-referred luminance recovery, bit-exact RGBE-quad surface, XYZE photometric fix; fuzz + Criterion | ✅ ~99% — contract `encode*` (`rle`/`line_ending`/`exposure`/`primaries`/`gamma` fields, `encode_rgb8` linear-by-default with `with_input_gamma`); lossless round-trip pinned |
| **QOI** | ✅ 100% | ✅ 100% |
| **TGA** | ✅ 100% | ✅ 100% |
| **ICER** (JPL) | ✅ ~98% — image-crate contract API (Gray8 / Gray16Le 9–16-bit / Yuv444P / Gbrp8 native, ICER-3D bands via `decode_all`, limits → LimitExceeded); §V.C auto segment election; interop container tail blocked on rulings; registry container: cube = one packet, one frame per band; mux == encode_all; memory bounded: planned working set under `max_bytes`, 5 B/sample decode state, hostile 64 MP header 690→69 MB | ✅ ~95% — contract `encode`/`encode_all` (lossless RGB via Gbrp8, 3-D cubes); §V.C closed, digest-pinned wire; mid-packet truncation blocked on the unstaged [13] tables |
| **WBMP** | ✅ 100% | ✅ 100% |
| **PCX** (ZSoft) | ✅ 100% | ✅ 100% |
| **ILBM** (Amiga IFF) | ✅ ~98% — contract API, core optional; ILBM/PBM/ACBM/DEEP/RGB8/RGBN, ANIM via `decode_all`, CAT/LIST children; r477 masking-2 colour key rides as the core palette-alpha record (mask planes still compose `Rgba`), ANNO/AUTH/(c)/NAME/TEXT/DPI/GRAB/DEST + CAMG as `iff-*` blobs on stream + frame, 3 fuzz-found hostile-input fixes (FORM-size underflow, 9 GB forged BMHD, zero-width panic), new daily Fuzz workflow; lacks ANIM op-74, dual-playfield compositing | ✅ ~95% — contract `encode`/`encode_all`, all 8 ANIM ops; r477 registry encoder declares 6 layouts (Gray8 as a grey palette, non-opaque `Rgba` as a mask plane or typed refusal — alpha is no longer dropped silently), alpha record → transparent colour or mask plane by one rule, native chunks written back; lacks HAM ANIM encode |
| **PICT** (Apple QuickDraw) | ✅ ~99% — image-crate contract API (`Rgba` native, `info` from the picture frame, strict mode, 256 MiB raster cap); v1 + v2 vector / raster / text / regions, `$8200` QuickTime images (`jpeg` via mjpeg, `raw `, `cvid`), Palette Manager rule, ImageMagick-emitted PICTs byte-identical; registry container with structural probe (467-fixture no-false-positive sweep); lacks `rle `/`rpza`/`smc `, `raw ` ≤8-bit CLUT (docs gap) | ✅ ~97% — contract `encode*` (version / pack / frame / resolution / clip fields; Packed24 rowBytes overflow fixed), v1 / v2 rasters, patterns, QuickTime wrappers, `PictBuilder` for vectors |
| **SVG** | ✅ ~99% — contract API, core optional; full SVG 1.1 + 2 grid (16 filter primitives, SMIL, CSS3); r480 paint servers bound per shape (objectBoundingBox gradients, tiled `<pattern>`), rsvg-verified; lacks gradient/pattern fill on `<text>` | ✅ ~99% — `write` / `write_svgz` of the document model; `encode*` `Unsupported` by design |
| **PDF** | ✅ ~99.5% — §9.4.3/§9.4.4 text-matrix glyph advancement + bytes → Scene via xref/ObjStm + encryption R=2..6 + signatures + text extraction + Tagged-PDF + §14.6 marked-content + 5 stream filters + annotations + §7.10 multi-input Type 0 (Order-1 + Order-3 cubic-spline)/Type 4 + Type 2/3 functions + Type 3 /FontMatrix glyph-advance scaling + §8.10 Form XObject Do-operator painting + §7.7.3.3/§7.7.3.4 page-tree MediaBox/Resources/Rotate inheritance + §8.6.6.5 DeviceN + Separation tint transforms + §8.7.4.5 all 7 shading types evaluated to geometry/colour (Gouraud/Coons/tensor meshes + axial/radial/function gradients) + CIE colour spaces (CalGray/CalRGB/Lab→XYZ→sRGB) + §8.7.3.3 shading-pattern fills + §8.7.3 tiling-pattern fills (coloured PaintType-1 + uncoloured PaintType-2) + §8.7.4.5 clipped axial/radial sh paint-into-Scene + §8.9.7 content-stream inline images (BI/ID/EI placed with CTM) + §9.6.5 Type 3 font glyphs painted as vector geometry (CharProcs under Tm∘FontMatrix + d0/d1 colour rule) + §12.5.5 annotation appearance streams (reader paint with AS/flags/OC gating + writer generation incl. AcroForm buttons) + gradient write→read round-trip + §11.6 transparency both directions (soft masks reader→IR→writer, groups-as-units, backdrops; q/Q full-state fix) + image scene splice with alpha; read −30% wall | ✅ ~99% — multi-page writer + encryption + signatures + AcroForm + annotation/embedded-file/timestamp writers |

</details>

<details>
<summary><strong>3D scenes & assets</strong> (click to expand)</summary>

> The typed Scene3D / Mesh / Material PBR / Skin / Animation / Camera / Light / AudioEmitter model lives in `oxideav-mesh3d`, with `Mesh3DDecoder` / `Mesh3DEncoder` traits and a `Mesh3DRegistry` that's parallel to `oxideav-core::CodecRegistry`. Per-format crates register into it. `oxideav-meta::populate_mesh3d_registry(&mut Mesh3DRegistry)` walks every enabled format's `register()`. Morph-weight precedence (animation > node > mesh, per-instance `node.weights`) resolves in the hub (r441); typed KHR_texture_transform + sampler surfaces with UV-coverage validation (r444). Lazy bytes flow through `AssetSource` (with a `raw_storage` pass-through hook for archive-backed sources, e.g. ZIP-stored USDZ textures + audio).

| Format | Decode | Encode |
|--------|--------|--------|
| **STL** (ASCII + binary) | ✅ ~99% — both forms + colour attrs + topology + 9-step repair pipeline + validation/lint surface (edge-length/centroid geometry stats + full mass-property triad volume/centroid/inertia-tensor + duplicate-facet culling + non-manifold-edge examples + ASCII solid/endsolid-name lint + Materialise-header inspector) | ✅ ~99% — both formats + attribute pass-through |
| **OBJ** (+ MTL) | ✅ ~99% — full Wavefront grammar + MTL (Phong + PBR) + free-form curves/surfaces with trim-loop re-meshing + ctech/stech cparm resolution-aware tessellation + typed directive accessors + superseded cdc/bzp free-form tessellation + typed obj:superseded accessor + smoothing-group vertex-normal synthesis + tessellation-budget clamp + differential-fuzz fixed-point-proven (4 round-trip/DoS fixes) + fuzz | ✅ ~99% — symmetric + negative-index encoder + byte-faithful 1D/3D vt re-emission + header-comment preservation + vt-dedup index fidelity + state-setting `g` groups (2000-seed property-fuzz-verified) |
| **glTF 2.0** (+ .glb) | ✅ ~98% — JSON + .glb + full PBR + 12+ KHR extensions; mesh3d 0.0.6 fully adopted (targetNames, typed sampled MorphWeights, node weights, texture transform); lacks KHR_audio_emitter (docs gap) | ✅ ~95% — symmetric round-trip incl. XMP + KHR_meshopt_compression write — full v1 bitstream (all control modes) + edge-reuse triangles + two-baseline indices + all 4 Appendix-B forward filters |
| **USDZ** (+ USDA) | ✅ ~97% — typed MaterialExt + UsdSkel + PointInstancer + composition arcs, Crate ceiling 0.12 (Relocates/Splines), cross-package selectors; lacks UsdGeomCamera/UsdLux (unstaged) | ✅ ~92% — USDA + typed USDC writers (usdcat/usdchecker-validated) + opt-in composition-arc preservation + PointInstancer; fixture fixed point byte-identical; lacks Crate array compression |
| **FBX** | 🚧 ~95% — full §7 section set + typed morph targets + sampled MorphWeights + skin Model→Cluster edge (was silently dropped); lacks in-between stations (docs gap), NURBS wire payload | ✅ ~95% — Scene3D→FBX binary+ASCII with verbatim passthrough of every unmodelled record; decode→encode→decode fixed point 7/7 fixtures both forms; lacks KeyAttrFlags semantics (docs ask) |
| **IFC** (BIM, ISO 16739) | ✅ Phase 3 complete / Phase 4 ~90% — STEP/P21 + EXPRESS typing + tessellation: profiles, sweeps, curved advanced Breps, non-convex mesh–mesh Booleans + curved half-spaces, p-curve-bounded + sectioned surfaces, CSG trees, Delaunay-trimmed faces, WHERE rules, georeferencing, fuzzed; lacks torus half-spaces + both-periodic implicit outers | — |
| **Alembic** | 🚧 ~0% — Ogawa wire format docs-gapped per `docs/3d/alembic/GAP-TRACKER.md` | — |

Cross-format integration: `oxideav-cli-convert` exposes a 3D conversion path through `oxideav_meta::populate_mesh3d_registry` — `oxideav convert in.obj out.gltf` (or `--probe` for structural inspection). `crates/oxideav-tests/tests/mesh3d_*.rs` runs the cross-format roundtrip suite. The convert verb carries an ImageMagick-compatible op set (`-resize` / `-thumbnail` / `-extent` / `-monochrome` / `-roll` / `-define` …) plus a 3D→raster renderer (Gouraud + Phong, `-light` / `-camera` / `-projection` / `-fov`, debug render modes, `-aa N`). Black-box oracles cross-validate against Apple `usdzconvert` + Blender + assimp. Still-image side channels (the PDF and 3D renderers' output, icon input) ride the `oxideav-image` gateway, so every registered image format is an output there and `-quality` / `--opt` / `-depth` reach those encoders; the format crates are dev-dependencies only.

</details>

<details>
<summary><strong>Trackers</strong> (decode-only by design) (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **MOD / STM / XM / IT** | ✅ ~97% MOD · ~92% STM · ~98% XM · ~95% IT — r451 XM ref-lockstep 240/240 (FT2 retrig/vol-col/Lxy-envelope ordering fixed, 2 fuzz bombs closed, multi-sample keymaps; STM effect semantics docs-blocked); shared Paula/FT2 mixer + full effect sets + Ultimate SoundTracker 15-sample + Startrekker FLT8 layouts + STM E4x/E7x waveform control + XM fine-slide last-non-zero memory (E1/E2/EA/EB/X1/X2) + note-delay LFO/counter-reset consistency + Kxy=note-97 silence + E6x pattern-loop point reset on pattern transition + typed sample-header accessors + ED0 immediate-trigger fix + n_patterns 0xFF-overflow hardening + hostile-input fuzz + r439 order-flow conformance (Bxx erratum + §5.14 overflow wrap + E6x row-only rewind + OOB-jump diagnostics) + r447 +951 restart-byte wrap semantics (filler-aware) + UST loop-area playback rule + EEx×order-flow pins — order-flow saturated + whole-order-table pattern-scan loader fix + SoundTracker 2.6/IceTracker (MTN\0/IT10) end-to-end | — |
| **STM** (Scream Tracker v1) | ✅ ~85% — structural parse + shared-mixer playback; XM-parity effects (Gxy/Jxy/Bxy/Cxy/Exy/Hxy + 7xy tremolo + volume-slide variants); hard-pan LRRL | — |
| **XM** (FastTracker 2) | ✅ ~98% — structural parse + full playback; 38/38 black-box oracle gates (Amiga table, LFOs, autovibrato, envelope order, FT2 order-flow machine); lacks envelope Scale (docs) | — |
| **IT** (Impulse Tracker) | ✅ ~95% — decode + full player: NNA voices, envelopes, Axx…Zxx + Sxx, 25/25 black-box oracle gates, steered fuzz; lacks 2.14 compressed samples, filters, carry/S9x/Zxx (docs) | — |
| **S3M** | ✅ ~96% — stereo + full ST3 v3.20 effect set + per-channel effect memory + canonical 9-octave ST3 period table + Jxy note-index arpeggio + OPL2/AdLib instrument decode + YM3812 operator core + per-voice latched global volume (Vxx no longer rescales held notes) + per-pattern SBx loop scope (loop start reset at pattern boundary) + effect-memory OOB + truncated-stereo-split fixes + same-row Bxx+Cxx merged jump precedence + DP30ADPCM packed-sample depack + full-pipeline/decoder-API fuzz + r440 AdLib/OPL2 playback LIVE (doc-anchored EG on the staged KSR table, ADSR ±2 samples @44.1 kHz; two-operator FM/additive voices; key-on/off/re-key + Qxy semantics) + r446 Bxx raw-order-list erratum pinned (254/255 sentinels never compacted; divergent-jump diagnostic + real-bytes sentinel probes); lacks vendor EG base values + AM/VIB/KSL tables (docs asks filed) | — |

</details>

<details>
<summary><strong>Windows codec sandbox</strong> (click to expand)</summary>

A pure-Rust 32-bit x86 emulator + PE32 loader + Video for Windows
host that runs legitimately-licensed Windows codec DLLs on **any**
platform — Linux, macOS, FreeBSD, Windows. The codec never executes
on the host CPU; it runs through a software-interpreter sandbox.
Two co-equal end-uses: **rare-codec compatibility** (codecs the
project would otherwise permanently shelve — Indeo, MS-MPEG-4, WMV,
Sorenson, etc.) and **reverse-engineering aid** (every Win32 call,
every memory access, optionally every executed instruction crosses
a Rust boundary; output is JSONL events for downstream analysis).
The sandbox itself lives in
[`KarpelesLab/univdreams`](https://github.com/KarpelesLab/univdreams)
as the `ud-emulator` crate; `oxideav-vfw` is a thin bridge that
adds OS-aware codec discovery (`$XDG_DATA_HOME/oxideav/codecs/` +
cache) and registers ud-emulator-backed `Codec`s into
`oxideav-core::CodecRegistry`. VfW codecs expose both decode
(`ICDecompress*`) and encode (`ICCompress*`, `SandboxedVfwEncoder`)
through the sandbox; DirectShow filters are decode-only. Design contract in
[`docs/winmf/winmf-emulator.md`](https://github.com/OxideAV/docs/blob/master/winmf/winmf-emulator.md).

| Codec | Binary | Test fixture | `ICDecompress` | Notes |
|-------|--------|--------------|----------------|-------|
| Indeo 3 (IV31) | `IR32_32.DLL` | `cubes.mov` 160×120 | ✅ ICERR_OK | Integer ISA only |
| Indeo 5 (IV50) | `IR50_32.DLL` | `cat_attack.avi` 320×240 + 3 more | ✅ ICERR_OK 8/8 frames | MMX kernels active (1.5M-5M dispatches/frame post-r20 FloatingPointProcessor registry probe + EFLAGS.ID / RDTSC / Pentium II CPUID fixes) |
| Indeo 4 (IV41) | `IR41_32.AX` | `crashtest.avi` 240×180 + `indeo41.avi` 320×240 | ✅ ICERR_OK 8/8 frames each | MMX kernels active |
| MSMPEG4 v3 (DIV3) | `mpg4c32.dll` | wmpcdcs8-2001 reference binary | ✅ **DECODE 17/17 frames at 42.9 dB PSNR-RGB + ENCODE externally validated** — full ICCompress lifecycle wired; 176×144 BGR24 → 970-byte MP43 I-frame (78×); self-roundtrip 27.83 dB; AVI 1.0 wrap decodes through ffmpeg + mpv + ffprobe (mean 20.86 dB at q=5000). Covers I/P, skip-MB (~38%), alt-MV-VLC, AC-prediction. | 13 stubs + x87 ISA + DirectShow GUID + `ICINFO_SIZE = 568`; codec rejects non-BI_RGB output 4CC. |
| MSMPEG4 v3 DShow | `mpg4ds32.ax` | winxp | ✅ **Full GOP DirectShow decode + 20/20 across 16 fixture-runs** — covers 6/6 FOURCC variants (MP43/DIV3/DIV4/DVX3/AP41/COL1) routed through MP43 subtype; motion-pan-352×288 + skip-MB + AC-pred fixtures all green. | DirectShow IBaseFilter wrapper: COM scaffolding + ole32 stubs + HostIFilterGraph + HostIPin + HostIMemAllocator + HostIMediaSample + IMediaFilter. CLSID `{82CCD3E0-F71A-11D0-9FE5-00609778EA66}`. |
| WMV1/2 DShow | `wmvds32.ax` | winxp | CLASS_E_CLASSNOTAVAILABLE on default CLSID | Needs the shipped `wmvax.inf` filter CLSID; round-26+ |
| MSADDS audio | `msadds32.ax` | winxp | 🚧 **Pipeline driven through Receive, E_FAIL inside inner-decode (r70)** — PE-load + COM + dual-pin allocator handshake green; ffmpeg-derived extradata flips Receive HRESULT 0x8000FFFF → 0x80004005. r70 pinned actual bail JCC at `0xe282` (`cmp edi, [ebp+0x10]` / `jge → 0xe2bb`), EDI=0x748 = sample-count bound. r69 `0xea3a` hypothesis falsified; r63 helper_addref retired. | Same scaffolding as MP43; `AmtBlueprint::wma_*`; QueryAccept disasm at `docs/codec/msadds32-query-accept-validation.md` |

**Architecture** — the `ud-emulator` engine is a 4 GiB MMU + i386
integer ISA + MMX ISA (~50 opcodes) + x87 FPU (8-deep stack) +
PE32 loader + Win32 stub surface (kernel32 + user32 + msvcrt +
winmm + advapi32 + ole32 + vfw32) + **a COM dispatch layer**
(`Guid` parser + `ComObjectTable` ref-count bookkeeping + vtable
dispatch + class-factory cache covering IUnknown / IClassFactory /
IBaseFilter / IPin / IMemAllocator / IMediaSample / IFilterGraph)
for codecs that ship as DirectShow filters rather than VfW drivers
(`.ax` exposing `DllGetClassObject` instead of `DriverProc`). Both
ud-emulator and oxideav-vfw are `#![forbid(unsafe_code)]` — codec
DLL never runs on the host CPU, and the only `unsafe` boundary
other emulators have (mmap'd executable pages, JIT, longjmp)
doesn't exist here. **Provenance is not clean-room** — Microsoft's
API surface is public by design and explicitly licensable for
interoperability under 17 U.S.C. §117(a)(1) and Article 6 of EU
Directive 2009/24/EC. The codec DLL bytes themselves are
legitimately redistributable (shipped in K-Lite codec packs,
Microsoft WMP redistributables, QuickTime installers, Linux
`vfw_codecs` packages) — not committed to the repo.

**Auto-discovery** — `oxideav_vfw::register(&mut RuntimeContext)`
walks a codec-DLL discovery path, probes each loadable `.dll` /
`.ax` (VfW first via `DRV_LOAD` + `ICOpen` FOURCC sweep, then
DirectShow via `DllGetClassObject` + `EnumPins` on missing
DriverProc), and registers a `Codec` per result at **priority
200** so the pure-Rust SW path (priority 100) and HW path
(priority 10) both win unconditionally — VfW only resolves when
nothing else matches. Default discovery path is
`$XDG_DATA_HOME/oxideav/codecs/` (fallback `~/.local/share/oxideav/codecs/`,
Windows `%LOCALAPPDATA%\oxideav\codecs\`); env var
`OXIDEAV_VFW_CODEC_PATH=/p1:/p2` *replaces* the default when
set. Probe results cache to
`$XDG_CACHE_HOME/oxideav/vfw-discovery.json` keyed by
`(path, mtime, size)` so subsequent registers re-probe only
changed entries. Discovery is gated behind the `auto-discovery`
cargo feature (default-on); `--no-default-features` builds the
sandbox with no FS scan + no `log`/`serde` dep transitive cost.

**Reproducible encode** — `Sandbox::with_rand_seed(u32)` (or `set_rand_seed` at runtime) seeds the sandbox-level `msvcrt!rand` LCG so codec calls that consult `rand`/`srand` are deterministic; default seed is 1 matching MSVC's pre-`srand` initial state. Two sandboxes seeded identically produce byte-identical encoded output. `mpg4c32.dll`'s VfW encode path does not currently consult `rand`, so the API is protection-only on this codec; any future codec that does will inherit deterministic behaviour automatically.

**Trace mode** — disabled by default behind a `trace` Cargo
feature (zero hot-path cost when off). When on, every memory
read/write to a watched range, every Win32 call (with arguments +
return value), and optionally every executed instruction emit
JSONL events. Schema documented in
`docs/winmf/winmf-emulator.md`. The reverse-engineering output is
the input format the project's
specifier→extractor→implementer round procedure consumes when
producing clean-room codec specs from scratch.

### Interactive debugger CLI — now `ud vfw` (univdreams)

The forensic debugger CLI that used to ship as `oxidetracevfw` has
moved to [`KarpelesLab/univdreams`](https://github.com/KarpelesLab/univdreams)
as `ud vfw {probe, decode, encode}`. univdreams' `ud-emulator` crate
is the upstream of this sandbox; `oxideav-vfw` is a thin Rust
adapter that registers ud-emulator-backed codecs into
`oxideav-core::CodecRegistry`. The full debugger surface
(per-instruction trace, memory watchpoints, PC breakpoints, GDB
Remote Serial Protocol server, JSONL trace sink, cascade-loaded
module-stub synthesis) is preserved one repo up. `cargo install
ud-cli` to use it.

</details>

<details>
<summary><strong>Hardware acceleration</strong> (click to expand)</summary>

For codecs the host's GPU / ASIC accelerates natively, oxideav can
delegate decode/encode to an OS hardware engine. The bridges open
the OS framework via `libloading` at first use — **no compile-time
link, no `*-sys` build dep, no header shipped**. The framework
still builds and runs without any of them present; a missing or
older OS framework just unregisters the HW factory at startup so
the pure-Rust path takes the dispatch.

The clean-room workspace policy doesn't apply to these crates —
calling a system OS framework via FFI is the same shape as calling
`libc::malloc`. It's the platform, not a copied algorithm.

| Module | Platform | Decode | Encode | Notes |
|--------|----------|--------|--------|-------|
| **`oxideav-videotoolbox`** | macOS / iOS | 🟢 H.264 + HEVC + ProRes + MJPEG + MPEG-2 + VP9 + MPEG-4 Pt 2 + AV1 (M3+) + VVC — r476 decoder truthfulness: output labels match the planes VT emits (negotiated per session; JPEG 4:2:0 / grey, other samplings fall back to pure Rust), avcC/hvcC + Annex B framing, AU splitting, display-order output (POC / reorder depth), flush at EOF; umbrella H.264 cropping tests 0 → 4/4 and 5/5 byte-exact, 8-codec sweep 12/12 frames each; lacks VVC measurement, MMCO5 | 🚧 H.264 + HEVC + ProRes + MJPEG | Encoder knobs map onto VT session properties (bit rate / quality / profile / data-rate limits); PSNR_Y ~36-61 dB per codec. iOS links the frameworks via build.rs + `dlsym(RTLD_DEFAULT)`; macOS keeps the `dlopen` path; device-specific encoder gaps degrade gracefully via `kVTPropertyNotSupportedErr`; r401 fixed 4 latent FFI bugs (callback ABIs, decoded-frame PTS recovery, session + per-frame leaks) + OSStatus taxonomy + hardware require/enable/disable knob; lacks `avcC` extradata on the encoder output (Annex-B in-band). |
| **`oxideav-audiotoolbox`** | macOS | 🚧 AAC LC + HE-AAC v1/v2 + AAC-LD/ELD + ALAC + iLBC + AMR-NB + AMR-WB + MP3 + FLAC + Opus | 🚧 AAC LC + HE-AAC v1/v2 + AAC-LD/ELD + ALAC + iLBC + FLAC + Opus | MP3 decode bit-exact ≈89.8 dB SNR; FLAC bit-exact 188 416/192 000 i16 @ 48k/2ch; ALAC S32 lossless contract (S16/S32 input, 24-bit output); Opus via `kAudioFormatOpus` (RFC 7845 OpusHead family 0/1/255 + RFC 6716 frame-duration mapping; ~26 dB SNR roundtrip); MP1 + MP2 decode added (sample-exact); typed OSStatus taxonomy + RAII converter + OS-inventory-gated registration; r480 encoder rate/format grids read from the OS. |
| **`oxideav-vaapi`** | Linux (Intel iGPU + AMD Radeon, via libva) | 🚧 H.264 IDR-only on the registry | — stub | Codec id → VAProfile map + `EntrypointMatrix` probe; full-stream H.264/HEVC/VP9/AV1 VLD is in external PR #6, blocked on an `oxideav-vp9` release; lacks libva header docs |
| **`oxideav-vdpau`** | Linux (NVIDIA legacy / Nouveau) | 🚧 H.264 + HEVC + VP9 + MPEG-2 | — stub | Four `CodecInfo` entries (h264/hevc/vp9/mpeg2video) with `CodecCapabilities::video("<codec>_vdpau")` at priority 15 + max_size 8192² + container tags + libvdpau.so.1/libX11.so.6 pre-flight. |
| **`oxideav-nvidia`** | Cross-platform (NVENC + NVDEC) | 🚧 VP9 + AV1 + MPEG-2 | — | `Mpeg2NvDecoder` + MPEG-2 NVDEC factory (cuvidParser + `CudaVideoCodec::Mpeg2`); pre-flight `cuvidGetDecoderCaps` surfaces `Error::Unsupported` early → fallback to oxideav-mpeg12video; registered at priority 5 w/ QT/MP4 fourCC + Matroska codec-id. |
| **`oxideav-vulkan-video`** | Cross-platform (Vulkan VK_KHR_video_*) | 🚧 H.264 + HEVC + AV1 capability queries | — empty | HEVC + AV1 chained capability queries via `vkGetPhysicalDeviceVideoCapabilitiesKHR`; `sys.rs` adds StdVideo H.265 + AV1 type aliases + 4 sType discriminants + profile/anchor-level constants + 4 repr(C) Caps structs; `query_video_decode_h265_capabilities` (H.265 Main 8-bit 4:2:0) + `query_video_decode_av1_capabilities` (AV1 Main 8-bit 4:2:0). |

**Priority + fallback** — every HW factory registers with
`CodecCapabilities::with_priority(10)` (lower numbers win at
resolution time, SW codecs sit at priority 100+). Two fallback
paths to the pure-Rust codec are automatic:

1. **Load failure** (older OS, missing framework, sandboxed
   environment without entitlements) → `register()` logs and
   returns without registering, SW is the only candidate at
   dispatch.
2. **Init failure** (`VTDecompressionSessionCreate` /
   `AudioConverterNew` / equivalent returns non-zero status for
   the requested parameters — stream above device max,
   hardware encoder slot busy, profile not accelerated) →
   factory returns `Err`, and the selection walker
   (`oxideav_pipeline::make_decoder_with` / `make_encoder_with`)
   retries the next-priority impl.

Pipelines that **require** hardware (real-time low-latency
capture where SW can't keep up) opt out of the SW fallback by
setting `CodecPreferences { require_hardware: true, .. }` — the
selection walker then surfaces the OS-level error instead of degrading
silently.

**Opt-out** — `oxideav --no-hwaccel` sets
`CodecPreferences { no_hardware: true }`, which the pipeline
forwards to `make_decoder_with` / `make_encoder_with` so HW
factories are skipped at dispatch. The runtime context still
*registers* every HW backend — `oxideav list` shows the
`*_videotoolbox` / `aac_audiotoolbox` rows regardless of the
flag — only resolution is biased. Useful for byte-deterministic
output or regression bisection.

**Build flags** — disable hardware entirely with `--no-hwaccel`
on the CLI, or build with `oxideav-meta = { default-features =
false, features = ["pure-rust"] }` (= `all` minus `hwaccel`)
for a binary with no FFI to OS HW-engine APIs at all.

</details>

<details>
<summary><strong>Protocols, drivers & integrations</strong> (click to expand)</summary>

Not codecs or containers — these are the I/O surfaces and runtime integrations that surround them.

| Component | Role | Status |
|-----------|------|--------|
| **`oxideav-source`** | URI resolution + file reader + prefetching BufferedSource | ✅ `file://` + `mem://` + `data:` (RFC 2397) + `concat:` (mem://`/`data:`/`slice:` inner schemes) + `slice:<offset>+<length>!<inner>` byte-window + `FileScope` allow-list + `deny_dir` carve-outs + `file://` URI percent-decoding (RFC 3986 §2.1) + r433 symlink-escape hardening (roots re-canonicalised at check time; deny carve-outs no longer fail open) + full-ring prefetch deadlock fix + `concat:` leading-`//` round-trip fix + 3-target fuzz (~75M execs) |
| **`oxideav-http`** | HTTP / HTTPS source driver | ✅ `http://` + `https://` via pure-Rust `ureq` + `rustls` + `webpki-roots`; Range-request seeking; `HttpConfig` policy + RFC 7233 Content-Range/200-fallback/416 handling + RFC 9110 If-Range strong-validator + Content-Length cross-checks + HTTP-date 3 forms (IMF-fixdate/rfc850/asctime) + multipart/byteranges reject + Retry-After surfacing + RFC 7230 §3.2.4 obs-fold normaliser + RFC 9110 §8.4/§12.5.3 content-coding refusal (identity-only negotiation + coded-response rejection) + §12.5.5 Vary content-negotiation stability check + driver-owned §15.4 redirects (5 classes, loop-detect, scheme/host policy, Range/If-Range across hops) + RFC 3986 `uri` module (§5 resolution, all 41 §5.4 examples pinned) + `parse_headers` fuzz; lacks cookies/auth |
| **`oxideav-generator`** | Synthetic media source (`generate://...` URIs) + zero-input filters | ✅ audio synth (sine incl. phase/per-channel phase + chirp/FM/DTMF/multitone/ADSR/ringmod + 5-colour noise + `pwm` + `supersaw` + `tremolo` + dc/impulse trains) + image (xc/gradient/pattern/fractal/plasma/noise/label + Perlin-2001 + Worley + 1–8-bit quantised `ramp`) + video (testsrc/smptebars/fractal_zoom/gradient_animate/zoneplate/`scroll` + `movingbox` exact-MV motion probe + `snow` seeded stateless noise) + catalogue-wide byte-determinism suite + framefill benches + r480 IM7 gradient / radial-gradient (±1 vs magick), seeded `plasma:c1-c2` / fractal, CSS4 + IM colour names; lacks `hald:` |
| **`oxideav-rtmp`** | RTMP ingest + push | ✅ Server + client; AMF0/AMF3 parser/builder; Enhanced-RTMP v1 video + v2 audio + ModEx; pluggable key-verification; `rtmp://` PacketSource; symmetric teardown + client `poll_event` + v2 `MultichannelConfig` (24 SMPTE 22.2 positions) + Multitrack body + §E FLV file writer + `FlvReader<R: Write>` + NetConnection capability negotiation + §7.1.6 Aggregate Message routed end-to-end (`send_aggregate` + `next_packet` + `poll_event`) + ModEx TimestampOffsetNano (ns timebase) + typed `MessageStreamKind` accessor + §5 protocol-control invariant validator + §5.3 Acknowledgement received-byte window + Enhanced-RTMP v2 ReconnectRequest (typed client event + tcUrl resolution) + AMF3 §3.12 externalizable-object decode via `register_externalizable` per-class handlers + typed Enhanced-RTMP VideoFrameType.Command (StartSeek/EndSeek) seek-command frames + Enhanced-RTMP v2 audio silence-message + VideoPacketType.MPEG2TSSequenceStart (av01 descriptor) + SequenceEnd typed on both pipelines + AMF0 complete serializable marker set (§2.15 Unsupported 0x0D / §2.17 XML Document 0x0F / §2.18 Typed Object 0x10 + avmplus 0x11 AMF3 bridge) + play/subscribe direction complete (§4.2.1 PlaySession server + RtmpPlayer pull client + publish→play relay + `rtmp-play://` PacketSource + dynamic playlists/play2 + drain-until-FIN teardown fix) + chunk ext-timestamp/fmt-3 wire-correctness fixes (3 writer desync bugs) + RFC-1982 rollover unwrapping + AMF3 type-17 command path + v2 selector framing + §7.2.1.2 call RPC all surfaces + §5.4.5 peer-bandwidth limit types + Shared Objects (all 11 event types, all surfaces) + @setDataFrame/@clearDataFrame store+replay + HMAC-SHA256 digest handshake (auto-negotiated, dependency-free); + r441 Enhanced-RTMP v2 MULTITRACK end-to-end (per-track demux/mux with spec-invariant validation + typed TrackInfo + tag/multitrack send surfaces on client AND PlaySession + per-track PacketSource streams); lacks RTMPS + RTMPE |
| **`oxideav-sysaudio`** | Native audio output | ✅ Runtime-loaded backends (ALSA, PulseAudio, WASAPI, CoreAudio, OSS); CoreAudio + WASAPI real HAL latency; output-device enumeration; per-device routing API on all backends; `StreamRequest::buffer_frames` honoured; `Driver::preferred_format` introspection on WASAPI/CoreAudio/ALSA; functional OSS `/dev/dsp` via dlopen'd libc (S16_LE negotiation). BT-aware; falls back to software estimate if HAL unavailable; r401 added a CI mock backend, request pre-flight validation, software volume, `Driver::status()` tri-state, callback-panic containment, and fixed CoreAudio HAL enumeration (was empty on every host). |
| **`oxideav-pipeline`** | Pipeline composition (source → transforms → sink) | ✅ JSON transcode-graph executor; pipelined multithreaded runtime drives byte/packet/frame sources natively (spawn/seek/progress/abort on typed sources) + graph-validation hardening (alias-cycle guard, same-kind `all:` fan-out ordinals, key-directed parse — exponential nested-chain fixed) + error-propagation contracts + channel caps + byte ceilings + Progress counters + graph benches + r445 multi-output jobs run in ExecutionContext-clamped parallel waves (document-order error precedence) + seeks fan out to every routed source |
| **`oxideav-scene`** | Time-based scene / composition model | 🚧 data model for PDF pages / RTMP streaming compositor / NLE timelines + per-frame `Sample` + animation-track composition + `RasterRenderer` (bg solid/gradient + Rect/Polygon + `ObjectKind::Vector`) + `ObjectKind::Group` nested + SVG 1.1 path-data (M/L/H/V/C/S/Q/T/Z + relative + A arc) + `ObjectKind::Image(Decoded)` RGBA8 + `Background::DecodedImage(Arc<VideoFrame>)` + audio-cue mixing into `RenderedFrame.audio` + typed PBR metallic-roughness `Material` + `Scene::materials` palette + glTF 2.0 node graph COMPLETE (typed validation + cycle-safe traversal/walk utilities + Mat4 inverse/TRS-decompose + quaternion ops/slerp + keyframe node animation Step/Linear/CubicSpline + Scene-level graph/animation fields) |
| **`oxideav-render`** | Scene3D → pixels rendering backends | ✅ scanline + Whitted raycast + Monte Carlo path tracer (NEE+MIS, Sobol', HDR environment maps, every ratified KHR material extension incl. anisotropy / iridescence / diffuse-transmission / dispersion, 16 spp 256² 119 ms); textures through the `oxideav-image` gateway (every registered format, EXR stays linear); cross-backend parity ±1/255; lacks the four newest extensions on the raster backends |
| **`oxideav-bitstream`** | Codec-header parse/write toolbox | ✅ H.264 + HEVC parameter sets complete (SPS/PPS/VPS incl. VUI/HRD + SEI families + scaling lists + RPS derivation; H.264 + HEVC byte-exact parameter-set writers on lossless parses) + AV1 metadata OBUs + H.266 SPS/PPS/VPS/PH + APS (ALF/LMCS/scaling-list) + full Annex-D SEI set + OPI/DCI, all with byte-exact writers + H.266 complete VPS/SPS/PPS walks + byte-exact writers (r438) + HEVC typed SEI encoders (HRD-coupled BP/PT both ways) + AV1 sequence-header + VP9 keyframe writers (VP9 4-bit misparse found+fixed) + Annex-B ↔ length-prefixed framing — fuzz-hardened; lacks H.266 slice-header walk |
| **`oxideav-audio-filter`** | Audio effects & conversions (streaming) | ✅ ~50 filters: classic + transient/spatial/restoration family + SlewLimiter + LR4 crossover + `true_peak_detector` + `state_variable` Chamberlin SVF + Criterion benchmark harness (7 scenarios) + `crest_factor_meter` + `stereo_correlation_meter` (Pearson coefficient, sliding-window) + `zero_crossing_rate` observer (per-channel sliding-window meter, `sign(0.0) = +1` defends against `f32::signum -0.0` phantom-crossing) + `dither` (TPDF/RPDF requantizer + error-feedback noise shaping) + complete staged EQ-cookbook biquad catalogue (constant-peak BPF + slope shelves) + parallel/New-York compressor (dry/wet blend) + band-limited rational resampler (ratio-scaled anti-alias prototype, ≥40 dB end-to-end alias rejection) + `crossfeed` (headphone ITD + head-shadow) + chunk-size-invariance / hostile-parameter / denormal-flush / analytic-transfer-function contracts + `latency_samples` reporting (pitch_shift NaN-hang + wah/talkbox state-leak fixes) — see crate README for the catalogue |
| **`oxideav-image-filter`** | Single-frame image effects (stateless) | ✅ r478 frame contract on all 66 layouts + daily fuzz; r480 deep 16-bit/float, ImageMagick resize filter set ≡ magick, `-distort` (14 methods, virtual pixels, EWA) ≡ magick, statistics / compare / moments, 37/70 compose ops, fx / sparse-color / connected-components / kmeans / noise / fft; lacks wavelet / segment / sequence ops |
| **`oxideav-image`** | Image gateway (Layer 2 of the image-crate API) | ✅ ~99% — opens all 25 image containers, encode/save by name, blobs + palette alpha both ways, `ExecutionContext` budget; r480 layer ops (coalesce / optimize / deconstruct / flatten…, magick-pinned); lacks per-frame placement from the format crates (core ask) |
| **`oxideav-io`** | One-call opener on the gateway | ✅ r477 image 0.0.4 adoption complete: allow/deny lists ride the gateway (typed `Restricted`), ICC/Exif/XMP blobs + palette alpha on `Image` / `ImageFile` / `RgbaImage` / probe, strip + replace-by-kind on open and save, buffered sources, zero-config savers; 120 tests, 25/25 image containers ping = probe = gateway; lacks the a/v transcode path (`oxideav-pipeline`) |
| **`oxideav-cli-convert`** | `oxideav convert` image + 3D paths | ✅ still-image path on the gateway contract: codec picked by the requested name, schema-gated `-quality`, ICC/Exif/XMP carried by default (`-strip`), palette alpha, `--allow/deny-containers|codecs`, `.heics`/`.avis` from timed sources; r479 workarounds dropped (gateway `ExecutionContext` for the side channels — 12 MP icon from HEIC 0.51 → 0.15 s on 8 workers; the `.avif` quality-60 pin replaced by the encoder's own default), filter planning on declared layouts (each op paired with its typed filter, one conversion only where the filter refuses, 16-bit stays 16-bit through resize, palette kept through pointwise ops), `-set KEY=VALUE` / `-comment` through `Muxer::carries_metadata_key` + `set_metadata`, `.webp` output fixed (was a 0-byte file), writer panics remove the in-flight output; 12 MP HEIC → PNG 0.20 s, AVIF → HEIC 0.70 s on 8 workers; lacks container metadata on the executor / gateway routes |
| **`oxideav-pixfmt`** | Pixel-format conversion + palette + dither | ✅ 82 formats, 6642/6642 pairs — YUV↔RGB matrices, packed 16/32-bit tier, palette + dither, row-band engine (12 MP 4:2:0→RGB 0.7 ms threaded); r480 float colour models (Lab/LCh/Oklab/OkLCh/HSL/HWB/CMYK GCR/UCR/YCbCr…, ±1 LSB at 16 bits) + ICC v2/v4 transforms (`icc` feature, ΔE2000 ≤ 0.04 vs magick on matrix profiles); lacks black-point compensation, HSV/Luv/Jzazbz |

</details>

<details>
<summary><strong>Subtitles</strong> (click to expand)</summary>

All text formats parse to a unified IR (`SubtitleCue` with rich-text
`Segment`s: bold / italic / underline / strike / color / font / voice /
class / karaoke / timestamp / raw) so cross-format conversion preserves
as much styling as each pair can represent. Bitmap-native formats (PGS,
DVB, VobSub) decode directly to `Frame::Video(Rgba)`. All text parsers
tolerate UTF-8 / UTF-16 LE / UTF-16 BE BOMs and CRLF / LF / lone-CR
line endings.

**Text formats** — in `oxideav-subtitle`:

| Format              | Decode | Encode | Notes |
|---------------------|:------:|:------:|-------|
| **SRT** (SubRip)    | ✅ | ✅ | `<b>/<i>/<u>/<s>`, `<font color>` hex + 17 named, `<font face size>` + structural tolerance (PEM preamble + duplicate-index + whitespace-only continuation lines) |
| **WebVTT**          | ✅ | ✅ | Header, STYLE ::cue(.class), REGION, inline b/i/u/c/v/lang/ruby/timestamp + cue-settings round-trip + full REGION block + §4.1 NOTE comment-block round-trip + §3.4 cue identifier round-trip + §4.1/§3.3 strict signature + canonical timestamp enforcement + §6.4 HTML character-reference decoder (decimal / hex / 8 named) + §4.2.2 `&` / `<` / `>` escape on write |
| **MicroDVD**        | ✅ | ✅ | frame-based, `{y:b/i/u/s}`, `{c:$BBGGRR}`, `{f:family}` |
| **MPL2**            | ✅ | ✅ | decisecond timing, `/` italic, `\|` break |
| **MPsub**           | ✅ | ✅ | relative-start timing, `FORMAT=TIME`, `TITLE=`/`AUTHOR=` |
| **VPlayer**         | ✅ | ✅ | `HH:MM:SS:text`, end inferred |
| **PJS**             | ✅ | ✅ | frame-based, quoted body |
| **AQTitle**         | ✅ | ✅ | `-->> N` frame markers |
| **JACOsub**         | ✅ | ✅ | `\B/\I/\U`, `#TITLE`/`#TIMERES` headers |
| **RealText**        | ✅ | ✅ | HTML-like `<time>/<b>/<i>/<u>/<font>/<br/>` |
| **SubViewer 1/2**   | ✅ | ✅ | marker-based v1, `[INFORMATION]` header v2 |
| **TTML**            | ✅ | ✅ | W3C Timed Text, `<tt>/<head>/<styling>/<style>/<p>/<span>/<br/>`, tts:* styling + r171 IMSC 1.2: `<layout>` regions + `tts:textAlign` + 22 IR-unmodelled `tts:*` / `itts:*` style extras + 11 `ttp:*` / `ittp:*` parameter attrs + `HH:MM:SS:FF` / `<n>f` / `<n>t` against `ttp:frameRate` / `ttp:tickRate` + TTML2 §8.1.5 inline `tts:*` on `<p>` (modelled-attr wrap + ttml_p_extra canonical order) + §12.2.4 par/seq timeContainer timing + timed-span progressive reveal + TTML2 §10.2 complete styling-attribute vocabulary (44 tts:* round-trip byte-stable across style/region/inline-p) |
| **SAMI**            | ✅ | ✅ | Microsoft, `<SYNC Start=ms>` + `<STYLE>` CSS classes |
| **EBU STL**         | ✅ | ✅ | ISO/IEC 18041 binary GSI+TTI (text mode only; bitmap + colour variants deferred) |

**Advanced text (own crate)** — `oxideav-ass`:

| Format              | Decode | Encode | Notes |
|---------------------|:------:|:------:|-------|
| **ASS / SSA**       | ✅ | ✅ | Script Info (typed header accessors + WrapStyle→\q bridge) + V4+/V4 styles + full override-tag set rendered (borders / shadows / blur / clips / shear / karaoke / alignment) + typed font-metric/rotation tag family + typed \t animated-transform tag + typed event columns + [Fonts]/[Graphics] attachments + structured-model SSA↔ASS dialect conversion + StyleDef typed accessors + Collisions layout resolver + drawing m/n close + s/p/c B-spline + time-varying override-tag evaluation at time t (\move/\fad/\fade/\t incl. \t(\clip) rect interpolation/\k) + typed \r/\q tags + layer/margin-aware collision resolver + fuzz-hardened fixpoint serialiser (5M+ inputs); re-emit byte-identical |

**Bitmap-native (own crate)** — `oxideav-sub-image`:

| Format              | Decode | Encode | Notes |
|---------------------|:------:|:------:|-------|
| **PGS / HDMV** (`.sup`) | ✅ | ✅ | Blu-ray subtitle stream; PCS/WDS/PDS/ODS + RLE + YCbCr palette → RGBA + RLE codec property + multi-fragment ODS fragmentation on encode + negative sweep + PCS composition_state classified + routed to Packet keyframe flag + independent per-`palette_id` PDS slots within a display set (BD-ROM Part 3 §2.2.1.2.3 "Composition Segments indicate the Palette to be used") with PCS palette_id-driven render selection (fade/colour-change sets) |
| **DVB subtitles**   | ✅ | ✅ | ETSI EN 300 743 segments + §7.2.2 epoch state machine (cross-packet region/CLUT/object retention; normal-case deltas render) + 2/4/8-bit pixel-coded objects + §7.2.4 Y=0 full-transparency + character-coded objects + §7.2.5.1 CLUT-depth map-table application + §7.2.1 Display Definition window clip; encoder: full segment writers + 2/4/8-bit RLE + RGBA display-set encoder (PES-level), roundtrip-pinned + §7.2.5.1 Table-10 2-bit pixel-code conformance fix + spec-vector escape gates |
| **VobSub** (`.idx`+`.sub`) | ✅ | — | DVD SPU with control commands + RLE + 16-colour palette + SP_DCSQ 0x07 CHG_COLCON length-skip + CHG_COLCON application (typed bands + per-pixel replacements during canvas paint) + per-DCSQ STM latching + FSTA_DSP forced-display surfacing |

**Cross-format transforms** (text side): `srt_to_webvtt`,
`webvtt_to_srt` in `oxideav-subtitle`; `srt_to_ass`, `webvtt_to_ass`,
`ass_to_srt`, `ass_to_webvtt` in `oxideav-ass`. Other pairs go through
the unified IR directly (parse → IR → write).

**Text → RGBA rendering** — any decoder producing `Frame::Subtitle` can
be wrapped with `RenderedSubtitleDecoder::make_rendered_decoder(inner,
width, height)` (or `..._with_face(face)` for a TrueType face), which
emits `Frame::Video(Rgba)` at the caller-specified canvas size, one
new frame per visible-state change. Two paths:

- **With face** (default-on `text` cargo feature): shape via
  `oxideav-scribe`, rasterise via `oxideav-raster`. Honours per-run
  colour, supports any TTF/OTF face including CJK + emoji (CBDT colour
  bitmaps land via the bilinear/composer path).
- **Without face** (or with the `text` feature off): falls back to the
  embedded 8×16 bitmap font covering ASCII + Latin-1 supplement, bold
  via smear, italic via shear, 4-offset outline. No TrueType dep, no CJK.

In-container subtitles (MKV / MP4 subtitle tracks) remain a scoped
follow-up.

</details>

### Tags + attached pictures

The `oxideav-id3` crate parses ID3v2.2 / v2.3 / v2.4 tags + typed ID3v1/1.1 and Enhanced TAG+ trailers + CHAP/CTOC chapter frames (read+write symmetric, cycle-safe TOC walkers) (v2.2: complete §4 frame table with typed v2.2-only walkers + §3.1 compression-bit skip since r283; whole-tag
and per-frame unsync, extended header with **CRC-32 [ISO-3309]
verification and emission** since r153, v2.4 data-length indicator,
encrypted/compressed frames recorded as `Unknown` (v2.2 §4.20 CRM encrypted-meta frame now typed decode/encode/round-trip), **r161 v2.4 §3.4
footer emission + strict trailer-validation on read** composable with
whole-tag/per-frame unsync + extended-header CRC) plus the legacy
128-byte ID3v1 trailer. Text frames (T\*, TXXX), URLs (W\*, WXXX),
COMM / USLT, and APIC / PIC picture frames are handled structurally;
less-common frames (SYLT, RGAD/RVA2, PRIV, GEOB, UFID, POPM, MCDI,
…) survive as `Unknown` with their raw bytes available.

The `oxideav-flac` container surfaces the extracted
fields via the standard `Demuxer::metadata()` (Vorbis-comment-style
keys: `title`, `artist`, `album`, `date`, `genre`, `track`,
`composer`, …) and cover art via a new
`Demuxer::attached_pictures()` method returning
`&[AttachedPicture]` (MIME type + one-of-21 picture-type enum +
description + raw image bytes). FLAC's native
`METADATA_BLOCK_PICTURE` is handled natively; FLAC wrapped in ID3
(a few oddball taggers) works via the fallback path.

`oxideav probe file.mp3` prints a `Metadata:` section and an
`Attached pictures:` section with per-picture summary.

### Audio filters

The `oxideav-audio-filter` crate provides:

- **Volume** — gain adjustment with configurable scale factor
- **NoiseGate** — threshold-based gate with attack/hold/release
- **Echo** — delay line with feedback
- **Resample** — polyphase windowed-sinc sample rate conversion
- **Spectrogram** — STFT → image (Viridis/Magma colormaps, RGB + PNG output)

### Pixel formats + conversion

The `oxideav-pixfmt` crate is the shared conversion layer for video
codecs. The `PixelFormat` enum covers ~30 first-tier formats (ffmpeg
equivalent names in parentheses):

- RGB family: `Rgb24`, `Bgr24`, `Rgba`, `Bgra`, `Argb`, `Abgr`, plus
  16-bit-per-channel `Rgb48Le` / `Rgba64Le`.
- YUV planar: `Yuv420P` / `Yuv422P` / `Yuv444P` at 8 / 10 / 12-bit,
  plus JPEG-full-range variants (`YuvJ420P`, `YuvJ422P`, `YuvJ444P`).
- YUV semi-planar: `Nv12`, `Nv21`. YUV packed: `Yuyv422`, `Uyvy422`.
- Grayscale: `Gray8`, `Gray10Le`, `Gray12Le`, `Gray16Le`.
- Alpha-bearing: `Ya8`, `Yuva420P`.
- Palette: `Pal8`. 1-bit: `MonoBlack`, `MonoWhite`.

`oxideav_pixfmt::convert(src, dst_format, &ConvertOptions)` handles
the live conversion matrix (RGB all-to-all swizzles, YUV↔RGB under
BT.601 / BT.709 × limited / full range, NV12/NV21 ↔ Yuv420P, Gray ↔
RGB, Rgb48 ↔ Rgb24, Pal8 ↔ RGB with optional dither). Palette
generation via `generate_palette()` offers MedianCut and Uniform
strategies. Dither options: None, 8×8 ordered Bayer, Floyd-Steinberg.

Codecs declare `accepted_pixel_formats` on their `CodecCapabilities`;
the job graph (below) auto-inserts conversion when the upstream
format doesn't match.

### JSON job graph

The JSON job graph (executed by `oxideav-pipeline` via `oxideav run`;
the former `oxideav-job` crate was folded into the pipeline) is a
declarative way to describe multi-output transcode pipelines. A job is a JSON object: keys are output
filenames (or reserved sinks like `@null` / `@display`), values
describe tracks grouped by `audio` / `video` / `subtitle` / `all`,
and each track carries a recursive input tree of source refs and
filter / convert nodes.

```json
{
  "threads": 8,
  "@in":       {"all": [{"from": "movie.mp4"}]},
  "out.mkv":   {
    "video": [{"from": "@in", "codec": "h264", "codec_params": {"crf": 23}}],
    "audio": [{"from": "@in", "codec": "flac"}]
  },
  "out.png":   {"video": [{"from": "@in", "convert": "rgba"}]}
}
```

The executor has two modes: **serial** (`threads == 1`) runs one
packet at a time; **pipelined** (`threads ≥ 2`, default when
`available_parallelism()` ≥ 2) spawns one worker thread per stage
per track connected by bounded mpsc channels. The mux/sink loop runs
on the caller's thread so `JobSink` implementations don't need to be
`Send` (the SDL2 player sink in oxideplay stays a single-threaded
object). Both modes produce byte-identical output for deterministic
jobs.

`Decoder` / `Encoder` trait hook: `set_execution_context(&ExecutionContext)`
(default no-op) lets codecs opt into slice- / GOP-parallel work later
without trait churn.

Explicit pixel-format conversion nodes (`{"convert": "yuv420p",
"input": ...}`) fit anywhere in the input tree; the resolver also
auto-inserts a `PixConvert` stage between Decode and Encode when a
codec's `accepted_pixel_formats` list excludes the upstream format.

## Input sources

The source layer decouples I/O from container parsing. Container
demuxers receive an already-opened `Box<dyn ReadSeek>` and never touch
the filesystem directly. The `SourceRegistry` resolves URIs to readers:

| Scheme | Driver | Shape | Notes |
|--------|--------|-------|-------|
| bare path / `file://` | built-in | bytes | `std::fs::File` |
| `http://` / `https://` | `oxideav-http` (opt-in) | bytes | `ureq` + `rustls`, Range-request seeking |
| `rtmp://` | `oxideav-rtmp` (opt-in) | packets | Listener accepts one publisher; FLV-shaped tags → `Packet` (time_base 1/1000); skips the demux layer (executor branches via `SourceOutput::Packets`) |
| `generate://...` | `oxideav-generator` (opt-in) | frames | Synthetic audio / image / video; emits decoded `Frame`s directly (executor branches via `SourceOutput::Frames`) |

The HTTP and RTMP drivers are off by default in the library (`http` /
`rtmp` cargo features) and on by default in `oxideav-cli`. `oxideplay`
keeps `http` on; RTMP isn't player-relevant.

`BufferedSource` wraps any `ReadSeek` with a prefetch ring buffer
(64 MiB default in oxideplay, configurable via `--buffer-mib`). A
worker thread fills the ring ahead of the read cursor; seeks inside the
window are free.

```
$ oxideav probe https://download.blender.org/peach/bigbuckbunny_movies/BigBuckBunny_320x180.mp4
Input: https://download.blender.org/peach/bigbuckbunny_movies/BigBuckBunny_320x180.mp4
Format: mp4
Duration: 00:09:56.46
  Stream #0 [Video]  codec=h264  video 320x180
  Stream #1 [Audio]  codec=aac  audio 2ch @ 48000 Hz
```

## Playback

An opt-in binary crate `oxideplay` implements a reference player with
SDL2 (audio + video) and a crossterm TUI. SDL2 is loaded **at runtime
via `libloading`** — `oxideplay` doesn't link against SDL2 at build
time, so the binary builds and ships without requiring SDL2 dev
headers. If SDL2 isn't installed on the target machine, the player
exits cleanly with a "library not found" message instead of failing
to start. The core `oxideav` library and every codec/container/filter
crate stays pure Rust; the only FFI in the framework lives in the
optional HW-engine crates (`oxideav-videotoolbox` / `-audiotoolbox` /
`-vaapi` / `-vdpau` / `-nvidia` / `-vulkan-video`), each also
runtime-loaded via `libloading`.

```
cargo run -p oxideplay -- /path/to/file.mkv
cargo run -p oxideplay -- https://example.com/video.mp4
```

Keybinds: `q` quit, `space` pause, `← / →` seek ±10 s, `↑ / ↓` seek
±1 min (up = forward, down = back), `pgup / pgdn` seek ±10 min, `*`
volume up, `/` volume down. Works from the SDL window (when a video
stream is present) or from the TTY.

When the **winit + wgpu** video output is selected (`--vo winit`),
`oxideplay` ships an **egui on-screen overlay UI** (auto-hide after
~3 s of mouse idle during playback; stays visible while paused).
Mouse-driven controls cover play/pause, draggable seek bar, time
display, volume slider, mute, ±10 s skip, and a toggleable stats
panel. egui (0.34) + egui-wgpu + egui-winit are pure-Rust deps gated
behind the `winit` cargo feature, so SDL2 builds are unaffected.

## CLI

`oxideav` command-line verbs: `list`, `probe`, `remux`, `transcode`,
`run`, `validate`, `dry-run`, `convert`. Inputs can be local paths or
HTTP(S) URLs.

```
$ oxideav list                           # print registered codecs + containers
$ oxideav probe song.flac
$ oxideav transcode song.flac song.wav
$ oxideav remux input.ogg output.mkv
$ oxideav probe https://example.com/video.mp4

# JSON job graph
$ oxideav run job.json
$ oxideav run - < job.json
$ oxideav run --inline '{"out.mkv":{"audio":[{"from":"in.mp3"}]}}'
$ oxideav run --threads 4 job.json        # override thread budget
$ oxideav validate job.json               # check without running
$ oxideav dry-run job.json                # print the resolved DAG

# ImageMagick-style convert (chains filters; accepts generator shorthands)
$ oxideav convert in.png -resize 800x600 out.jpg
$ oxideav convert "xc:red" red.png                      # solid colour
$ oxideav convert "label:Hello world" greeting.png      # text → image
$ oxideav convert "gradient:red-blue" gradient.png

# PDF input + page selectors + Scene-aware fan-out (printf template)
$ oxideav convert -density 300 in.pdf -background white \
                  -alpha remove -alpha off page-%03d.png
$ oxideav convert in.pdf[0] cover.png                   # single-page extraction
$ oxideav convert in.pdf[2-5] excerpt.pdf               # page-range slice (vector preserved)
$ oxideav convert in.pdf      page-%d.svg               # one SVG per page

# 3D scene conversion via oxideav_meta::populate_mesh3d_registry
$ oxideav convert in.obj  out.gltf                      # OBJ → glTF
$ oxideav convert cube.stl cube.obj                     # STL → OBJ
$ oxideav convert scene.gltf scene.glb                  # JSON glTF → binary .glb

# Throughput bench across HW + SW backends (1080p default; --all walks every codec)
$ oxideav bench h264 --duration 3
$ oxideav bench --all --width 1280 --height 720 --side encode
```

Two global flags help diagnose startup or codec issues:

- `--debug` enables debug log output to stderr through the `log` facade.
  Every crate that emits `log::debug!` flows through here.
- `--no-hwaccel` sets `CodecPreferences { no_hardware: true, .. }` on
  the pipeline so the resolution layer skips hardware-accelerated
  factories at dispatch time. The runtime context still registers
  every backend (`oxideav list` shows them all regardless of the flag);
  only the per-route choice is biased toward the pure-Rust path.
  Useful for byte-deterministic output, regression bisection, or when
  the hardware encoder produces a worse stream than the pure-Rust path
  for a specific bitrate target.
- `--debug-output FILE` redirects debug log output to a file instead of
  stderr (implies `--debug`; stderr stays clean).

`oxideplay --job <file>` runs a job where `@display` / `@out` binds
to the SDL2 player sink; other outputs (file paths) write to disk in
the same run.

## Building

> **First clone? Run `./scripts/update-crates.sh` before `cargo build`.**
> The workspace tracks only the integration glue (`oxideav-cli`,
> `oxideplay`, `oxideav-tests`, the `oxideav` facade, the
> `oxideav-meta` aggregator); every per-format codec lives in its
> own `OxideAV/oxideav{,-*}` GitHub repo and must be cloned into
> `crates/` first. `cargo build` on a bare checkout fails with
> `failed to load manifest for workspace member` until you do.

```
git clone https://github.com/OxideAV/oxideav-workspace.git
cd oxideav-workspace

gh auth login                 # one-time: update-crates.sh uses gh API to list siblings
./scripts/update-crates.sh     # populates crates/ with every OxideAV/oxideav{,-*} repo

cargo build --workspace
cargo test --workspace
```

The `oxideav` binary is produced by the `oxideav-cli` crate:

```
cargo run -p oxideav-cli -- --help
```

### Working with the sub-crates

Every per-format codec — plus `oxideav` (facade) and `oxideav-meta` (aggregator) — lives in
its own `OxideAV/oxideav{,-*}` repository. The root `Cargo.toml` globs
`crates/*` as members and points every `[patch.crates-io]` entry at
those local paths, so once the siblings are cloned the workspace
resolves entirely without crates.io round-trips for any `oxideav-*`
dep during local dev or CI.

- `scripts/update-crates.sh` — clones every missing OxideAV sibling AND fast-forwards already-cloned siblings to upstream tip via a single GraphQL call. Skips siblings whose upstream is already an ancestor of local HEAD and refuses to fast-forward when local commits have diverged, so in-progress work is preserved. Idempotent; safe to re-run.

```
./scripts/update-crates.sh    # clone + fast-forward all OxideAV crates
```

CI runs `update-crates.sh` at the top of each job (see
`.github/workflows/ci.yml`), so no crates.io resolution is needed there
either — the workspace builds whether or not a given crate has been
published yet.

`.gitignore` hides the cloned crate working copies so `git status` in
this repo only shows changes to the native members (`oxideav-cli`,
`oxideplay`, `oxideav-tests`). Changes inside a cloned crate are
committed against that crate's own repo, not this one.

## License

MIT — see [`LICENSE`](LICENSE). Copyright © 2026 Karpelès Lab Inc.
