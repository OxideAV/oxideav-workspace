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
  Timestamp / PixelFormat (incl. 16-bit planar YUV + deep Yuva 4:2:2/4:4:4 at 10/12/16-bit; VideoFrame palette + per-plane significant-bits side-channels for mixed-depth streams; r429: deep Yuva 4:2:0 + 8/16-bit planar GBR(A) formats + Ogg BOS-magic codec resolution; r449: 4:4:0 YUV ladder + scene-referred F32 family + plane-geometry/sizing helpers — 70 formats) / ExecutionContext + **DoS framework: `DecoderLimits`
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
  regenerates the call list. (Earlier attempt at a `linkme`-based
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
use oxideav_core::{CodecId, CodecParameters, CodecRegistry, Frame, Packet, TimeBase};

let mut reg = CodecRegistry::new();
oxideav_g711::register(&mut reg);

let mut params = CodecParameters::audio(CodecId::new("pcm_mulaw"));
params.sample_rate = Some(8_000);
params.channels = Some(1);

let mut dec = reg.make_decoder(&params)?;
dec.send_packet(&Packet::new(0, TimeBase::new(1, 8_000), ulaw_bytes))?;
let Frame::Audio(a) = dec.receive_frame()? else { unreachable!() };
// `a.data[0]` is S16 PCM.
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
| Matroska  | ✅ | ✅ | ✅ | MKV/MKA/MKS; Cues read/write-symmetric seek (CueBlockNumber/CueDuration) + SeekHead/Chapters/Attachments + lacing + CRC-32 + typed RFC 9559 element surface (Tags, Colour/HDR mastering, Projection, BlockAdditions read+write, TrackOperation read+write, ContentEncryption signing quartet read+write, TrackTranslate read+write, …) + full BlockGroup semantics (ReferenceBlock/Priority/CodecState/DiscardPadding) + SilentTracks + complete Chapters edition/atom tree incl. ChapterProcess — all read+write; Tags mux (Targets + recursive SimpleTag §5.1.8) symmetric with demux; EBML walker property-fuzzed + typed TrackIdentity (Name/Language/LanguageBCP47/CodecName/Flag{Enabled,Default,Lacing}/AttachmentLink) demux↔mux + Linked-Segment Info (Segment/Prev/Next UUID+Filename, SegmentFamily, ChapterTranslate) + Info Title/DateUTC mux (closes the last Info demux↔mux asymmetry) + OldStereoMode + EBML-header DocTypeExtension/version quartet (full RFC 9559 + RFC 8794 element coverage, demux↔mux symmetric) + reclaimed BlockGroup A.3–A.14 (Slices/TimeSlice/ReferenceFrame) + TrackEntry A.16–A.18/A.25–A.27 legacy elements + AttachedFile A.40–A.42 (FileReferral/UsedStartTime/UsedEndTime) + EncryptedBlock + TagDefaultBogus + modern RFC 9559 element-name aliases (demux+mux) + damage-resilient demux (open_resilient: cluster resync + truncation recovery + DamageEvent ledger) + Cues-less seek fallback + live-streaming layout + §23.2 mid-stream tagging + cluster Position/PrevSize hints + per-track CodecDelay/SeekPreRoll + Segment Duration + BlockAdditionMapping + chapter BCP-47 mux (encoder now symmetric) + fuzz overflow fix + RFC 9559 Table-53 registry census pinned in CI (250/250) + WebM-profile support table + whole-file conformance scanner + strict-WebM mux gating (lenient opt-out) + two-pass Duration finalization + §25.1 Cluster byte budgets + SeekHead-directed late-master recovery + front-Cues layout (§25.3.3) + r434 CELLAR schema validator (262-element table, WrongParent/range/occurrence/CRC placement; zero findings on black-box muxer output) + legacy WebM rows keyed (250-row support table, none Unlisted) + zero-Cluster resilient open + §25.2 SeekHead-expansion Void + r438 TrackOperation APPLIED (3D plane combination + block join, virtual seek via Cues union, re-apply round-trip, schema zero-findings; 2 fuzz-found crashes fixed incl. the daily-Fuzz-red root cause); + r442 the six post-RFC v5 elements CLOSED off the staged doc (EditionDisplay tree, ChapterSkipType + nesting validator + range resolver, Emphasis, TagBlockAddIDValue joint-scope matrix; opt-in mux auto-declares DocTypeVersion 5, WebM-barred) + errata-8615 AttachmentLink list + r445 lie-audited self-referential indices (Cues + SeekHead, typed forensics) with trust-but-verify resilient seek (stale/forged offsets fall back to linear scan; fuzz-found strict-seek overflow saturated) |
| WebM      | ✅ | ✅ | ✅ | First-class: separate fourcc, codec whitelist (VP8/VP9/AV1/Vorbis/Opus); inherits Matroska Cues seek |
| MP4       | ✅ | ✅ | ✅ | mp4/ismv; faststart + iTunes ilst + fragmented demux/mux (DASH/HLS/CMAF) + sidx/mfra + broad typed box-accessor surface + CENC AES-128 CTR/CBC decryption (all 4 schemes) + amve ambient-viewing-environment HDR metadata + btrt bit-rate box (buffer/max/avg on all sample entries) + prft producer-reference-time box (2022-edition NTP-flag annotations) + typed §10 sample-group description entries (roll/prol/rap/tele/sap/alst/rash) + §8.16.4 ssix subsegment-index emission (after each sidx) + §8.7.3 leva level-assignment write symmetry + §8.15.4.2 stvi StereoVideoBox demux+mux + §8.8.16 assp/trep fragmented-mux emission + §8.9.5 csgp compact sample-to-group mux + fragment-local (traf) sgpd/sbgp/csgp demux (CENC seig key-rotation) + §9 hint-track family (RTP/SRTP/RTCP/MPEG-2-TS sample entries + hinf stats) + §8.13 FD item-info + §8.11.7/8 meco/mere + box write-side builders (tref/trgr/kind/cprt/tsel/strk/subs/saiz/saio/pdin/prft; subs/saiz/saio now public read+write) + HEIF/MIAF item-properties graph (iprp/ipco/ipma + ispe/pixi/rloc/auxC/irot/imir/lsel/udes/altt/iscl/rref + grpl entity groups + per-item iloc/iref) read+write + muxer codec entries ×12 (h265/av1/vp9/vp8/h263/opus/alac/ac3/eac3/mp3/G.711) + self-describing CENC packager (per-traf senc/saiz/saio + seig key rotation on write + CencFragmentPackager, all four §10 schemes encrypt→demux→decrypt byte-exact gated; + EveryKeyframe final-sample data-loss fix) + PIFF legacy uuid encryption boxes (senc/tenc/pssh, CENC-bridged) + DASH emsg v0/v1 (demux capture + fragment-mux emission, absolute-time resolver) + full §8.6.6 edit-list timeline (demux mapping incl. discard-flagged pre-roll, typed elst surface, explicit mux + CMAF-priming emission, foreign-file black-box parity) + per-packet stsd-index detection + r438 §8.8 fMP4 completion (mehd/empty-time/mfro read+write, senc-less CENC via saiz/saio) + 5 fuzz OOM classes eliminated + r443 6-target structure-aware fuzz battery (box fixed-points, HEIF graphs, CENC roundtrip, mux identity; 3 hostile-input fixes, daily Fuzz workflow) |
| MOV (QuickTime) | ✅ | ✅ | ✅ | QTFF + ISO BMFF meta + HEIF/HEIC item properties + fragmented-MP4 seek + edit-list mapping (+ muxer edts/elst emission) + `cmov` compressed-movie decompression + §8.14 sub-track groups + §8.7.8/§8.7.9 saiz/saio + §8.6.1.3 ctts composition-offset muxer write + sound sample-description v0/v1 (fixed-ratio + VBR `-2`) + §8.10.1 udta movie/track metadata muxer write + tmcd timecode-track sample-data decode (start_timecode) + §12.3.3 timed-metadata sample entries (metx/mett/urim + txtC + btrt) + §12.4.2 hint-track hmhd + §12.6.3 subtitle sample entries (stpp/sbtt) + §8.9.3 typed sample-group description entries (tele/sap/rash/alst) with per-sample lookups + QuickTime Text + ISO BMFF stxt timed-text sample entries + §8.9.3 sgpd sample-group-description mux (closes the dangling csgp-index gap) + §8.6.1.4 cslg composition-to-decode write + classic run-length sbgp form + muxer write-side tref/tapt/external-dref/timecode-track/chapter-text-track/per-track-language (mdhd+elng) + gmhd/gmin + timed-metadata/subtitle/timed-text/hint track write + stz2 compact sample sizes + write-side per-track auxiliary/grouping atoms (sdtp/stdp/padb/stsh/subs + load/clip/matt/kind/tsel/trgr, parser-symmetric) + r394 edit lists APPLIED to packet timing + edited-timeline seek + ISO AudioSampleEntryV1/srat/chnl both directions + external-dref guard (+3 conformance fixes: video-stsd field offsets both sides, tkhd/mvhd durations from edits, v1-audio 16-byte swallow) + QTFF-2012 SoundDescriptionV2 read+write + lpcm format flags + typed wave/esds/frma extension atoms + hostile-alloc hardened (2 OOM fixes) + FULL edit-list timeline semantics (typed elst surface, discard-flagged never-presented media incl. negative-pts head trims, dwell/rate mux constructors, fragmented init edts, saturating v1 windows, ffprobe parity suite) + mvhd matrix/rotation/preview + tkhd write surface; ffprobe-accepted; r429 muxer: full stbl family + time-ordered interleave + faststart placement + registry dual-API, ffprobe packet-table parity, mux fuzz; r440 external-data movies (set_external_media authoring + sandboxed dref resolution with pluggable opener + rmra-alias chain composition; cmov/stz2 interplay suite); r443 §8.8.7 fragmented external-data BOTH ways (explicit base_data_offset through the dref stream; moof-relative+non-self-dref refused per text) + trun cts + mfra/tfra/mfro + tfdt + mehd write |
| AVI       | ✅ | ✅ | ✅ | AVI 1.0 + OpenDML 2.0; interlaced + VBR audio + LIST INFO + WAVEFORMATEXTENSIBLE + ODML keyframe seek + per-packet keyframe flags + idx1 `rec ` LIST entries round-trip + avih.dwReserved[4] reserved-array accessor + vprp typed VideoFormatToken/VideoStandard accessors + indexed-DIB baseline colour table (RGBQUAD bmiColors[]) + xxpc effective-palette resolution + OpenDML AVIMETAINDEX typed bIndexType (super/std-index) + non-conformant reserved-field diagnostics + OpenDML vprp signal-shape typed accessors + top-level JUNK/DISP read-write symmetry + multi-RIFF movi/AVIX segment surface + nBlockAlign VBR/CBR classification + AVISF_VIDEO_PALCHANGES/AVIF_HASINDEX conformance cross-checks + mux round-trip + r394 OpenDML spec-complete: per-stream indx→ix## targets (spec-correct entries) + in-strl compact std index R+W + txts subtitle streams + bounded hostile allocations (+2 real fixes: strh patch walk, super-index targets); r442 3-target structure-aware fuzz harness from scratch (11.5M execs, nested-LIST 4 GiB allocation + duration overflow fixed, roundtrip identity clean, daily Fuzz workflow) |
| Blu-ray (BD-ROM) | ✅ | — | — | UDF 2.50 + BDMV + `.m2ts` + `bluray://`; playlists / chapters / multi-angle + EP_map keyframe seek + AACS hook + HDMV nav title-engine (index.bdmv→MOBJ; inter-title Jump/Call/Resume, PSR4 seed) + PGS subtitle-segment parser (PCS/WDS/PDS/ODS + RLE) + Display-Set grouping + multi-ODS fragment reassembly + PGS renderer (palette resolution + window compositing) + HDMV navigation-command opcode decode + PSR/GPR register model + HDMV VM execution (Set/Compare/Branch interpreter + Movie-Object runner over Jump/Call/Resume) + CLPI SequenceInfo/AtcSequence/StcSequence + ClipInfo byte/SPN index + ProgramInfo PID lookups + CPI EP-map keyframe seek-index accessors (BD-ROM AV §5.5.4/§5.7) + UO_mask_table/is_repeat_SubPath round-trip fidelity + BDMV fuzz/hostile-input hardening + r443 SubPath/SubPlayItem (typed SubPathType, multi-clip) + typed 64-bit UO-mask (30 operations + reserved-bit forensics) + title-timeline sync windows; lacks SubPath clip playback + IG button-state machine + BD-J |
| DVD-Video | ✅ | — | — | ISO 9660 + UDF 1.02 + IFO/VOB + `dvd://`; navigation VM (incl. PCI NSML_AGLI non-seamless angle jump) + SPU subpictures + RGBA compositor + time seek + VOB → MKV + DTS core frame-header decode + generic audio-substream header (FrmCnt/FirstAccUnit + access-unit offset) + PCI_GI vobu_isrc/c_eltm decode + PCI RECI raw-region capture + 16/20/24-bit LPCM width (bytes_per_sample ratio) + DSI nav-pack typed accessors (VOBU_SRI/SYNCI/SML_PBI) + IFO PGC program-map navigation + PGC_AST_CTL/PGC_SPST_CTL stream-control tables + PGC_SPST display-mode sub-stream resolver + typed StillTime + §6.2 MPEG-2 video elementary-stream header stack (Sequence / Sequence-Extension / Sequence-Display-Extension / GOP / Picture / Picture-Coding-Extension headers) + full navigation engine (domain-transition legality + angle-aware cell walk + PgcRunner pre/cell/post state machine + Type-1 Link resolution + menu D-pad/button bridge + disc-absolute TitlePlan) + playback runtime (stills + NavTimer + audio/SPU stream-select + karaoke routing + VOBU trick-play + backward-SRI index fix) + LPCM frame packing (16/20/24-bit) + synthetic-disc e2e suite; + full 5-band private_stream_1 map (incl. SDDS) + substream census + CGMS-A/APS copy-control decode + r436 second-pass ratification (vobu_cat/CPR_MAI raw-preserve pinned) + full PES header-extension decode + pack classifier + DVD-compliance auditor + 20/24-bit LPCM sample unpacking; lacks CSS auth + odd-channel 20-bit LPCM nibble order |
| MP3       | ✅ | — | ✅ | ID3v2/v1 + Xing/Info VBR + CBR/VBR seek; stereo decode via oxideav-mp3 |
| IFF (EA IFF 85) | ✅ | ✅ | — | `FORM/LIST/CAT` family — Amiga 8SVX + ILBM (EHB/HAM, palette-change chunks) + ANIM op-0/1/2/3/4/5/7/8 + true-colour FORM RGB8/RGBN/DEEP decode + encode (genlock-RLE + TVDC chunky) + FORM ACBM/ABIT plane-contiguous decode+encode+mux + Apple AIFF/AIFF-C + fuzz harness + r449 8SVX voice-structure COMPLETE (typed VHDR + per-octave voice trees + ADSR/SEQN/FADE/PAN read+write; looping-duration + stereo-split demux fixes; svx_decode fuzz target — one forged-ckSize OOM fixed across all FORM readers); lacks 16SV + ANIM op-J (docs-asked) |
| IVF       | ✅ | — | — | VP8 elementary stream container |
| MPEG-TS   | ✅ | ✅ | ✅ | ISO/IEC 13818-1 transport stream — full Table 2-34 stream_type mapping (52 named) + DVB PMT ES descriptors (stream_identifier/teletext/subtitling/AC-3/E-AC-3/DTS) + per-PID 33-bit PTS/DTS unwrap; packet/PSI/descriptor walk (PAT/CAT/PMT/TSDT — all four 13818-1 PSI tables + DVB SDT service_descriptor + DVB EIT (present/following + schedule, EN 300 468 §5.2.4) with short + extended (§6.2.15 tag 0x4E) event descriptors + DVB NIT (network_name_descriptor, EN 300 468 §5.2.1) + DVB BAT (§5.2.2 + bouquet_name_descriptor) + DVB RST running-status (§5.2.7)); Table 2-17 PES header fully decoded incl. PES_extension body (private data, pack_header, packet-sequence counter, P-STD buffer); muxer: multi-program DVB-SI (PAT/PMT/SDT/NIT/EIT present-following) + PSI fragmentation §2.4.4 + PSI repetition intervals + 15-descriptor write side (ISO_639 §2.6.18 + stream_identifier/service/network_name/short_event/service_list/teletext/subtitling/AC-3/E-AC-3/DTS/AAC) + periodic PCR §2.7.2 + PES PTS/DTS timing + mux→demux round-trip harness; + 18 typed DVB SI/PSI descriptors (§6.2 linkage/component/CA-identifier/parental-rating + satellite/cable/terrestrial delivery + AAC/data-broadcast/scrambling) + §6.2.16 extension_descriptor envelope; + write-side §2.4.3.4 adaptation-field builder (Table 2-6 incl. seamless-splice/ltw/piecewise-rate) + write-side Table 2-17 PES header builder + RAI keyframe index + keyframe-accurate seek + DSM trick-mode (Tables 2-20..2-22) + 188/192/204 framing tolerance + TS conformance validator (CC/PCR-cadence/PSI-CRC/rate) + deterministic hostile-input sweeps; r420 keyframe-accurate seek (RAI/data-aligned tiers, wrap-aware PCR bisection, multi-program) + r436 splice signalling (§2.4.3.5 + seamless) + PES splitting + T-STD model + CBR pacing + time-base discontinuities (PCR-only CC bug + 40 ms audio hold-back fixed; conformance-validated both directions) + r444 daily 4-target structure-aware fuzz (mux↔demux identity, T-STD-checked; 8 clause-cited findings fixed incl. pooled-TBsys CBR pacing + PCR ring/backfill + DTS wrap epoch) + r448 PSI/SI EXHAUSTED vs staged specs (ATSC A/65:2013 complete — PSIP family + DCCT/DCCSCT + MSS + Annex-C Huffman + Table 6.25; 13818-1:2000 Table 2-39 closed; write-side CAT/TSDT/BAT/SIT + short-form sections) + mux-side T-STD closed (§2.4.2.3 declared-Rmax video TB pacing, A/B-pinned) + two-generation remux gates; lacks post-2000 13818-1 amendment descriptors + A/52 descriptor bodies (asks filed) |
| AMV       | ✅ | ✅ | — | Chinese MP4-player format — demuxer + muxer + seek + strict-mode validators + symmetric demux→pixels/demux→PCM conveniences + S16-mono audio stream params + §4b IMA/DVI-ADPCM audio decode (1116 blocks → 93.0 s; decode_audio_payload convenience + PCM ffprobe-validated) + §4a in-crate baseline-JPEG video decode to RGB (device-hardcoded quant/Huffman tables) + video rate control + device-profile muxer validation + fuzz harness |
| FLV       | ✅ | ✅ | — | MP3/AAC/H.264 audio + VP6/H.264 video + Enhanced-RTMP extensions (incl. v2 audio-silence discard) + AMF0 metadata + multitrack + HDR colorInfo + fuzz; muxer covers tags / seek-table / cue-points / multitrack join + AMF0 Date (SCRIPTDATADATE) write + AMF3 value encoder + Enhanced-RTMP multichannel-config writer + onMetaData full AMF0 value matrix (nested Object/EcmaArray/StrictArray/Xml/Null) + Enhanced-RTMP-v2 per-track info maps + typed onCuePoint params + ModEx timestamp-offset-nano side-channel + multitrack join_tracks→demux round-trip + FLV-encryption muxer (Annex F.2 |AdditionalHeader + F.3 filtered-tag) + AMF0 Long String (>64 KB onXMPData) + Annex B.1 typed Flash-metadata accessors + Annex B.2 onImageData embedded-image harvest (read+write) + E.4.2.1 Speex/Nellymoser rate+channel pinning + legacy muxer write surface COMPLETE (PCM/ADPCM/G.711/Nellymoser/Speex/MP3-8k/screen-video + audio-silence signal) + fuzz |
| WebP      | ✅ | ✅ | — | RIFF/WEBP (lossy + lossless + animation; ANIM + ANMF emit) + VP8L encoder density push (cost-priced LZ77 DP + entropy-merge clustering + stacked transforms; −13.3% aggregate, byte-smaller than reference max-effort on 9/10 corpus images; max-effort wall −44% via cache-sweep hoists + DP planner) |
| TIFF      | ✅ | ✅ | — | TIFF 6.0 single-image + BigTIFF + PhotometricInterpretation=5/8 CMYK + CIE L*a*b* decode/encode + CCITT T.4 2-D + T.6 (Group 4) fax decode/encode + tiled-image layout + float Predictor=3 (IEEE 16/32/64-bit gray+RGB, strip/tile/planar) + tiled JPEG-in-TIFF (Compression=7) + 4:4:4 YCbCr planar+predictor + planar CMYK coverage |
| PNG / APNG| ✅ | ✅ | — | 8 + 16-bit, all color types, APNG + gAMA/cHRM/zTXt + tRNS round-trip (typed Grayscale/Rgb/Palette; ct=4/6 rejected); region-aware APNG encoder (per-frame offset + delay + dispose/blend, fuzz-hardened) + sRGB linear-light colour management + bKGD §13.15 background compositing + §12.4/§13.12 sample-depth scaling (16→8 + sBIT recovery) + LZW-decode bulk-extend; iCCP + iTXt + §4.3 colour-source precedence resolved (cICP>iCCP>sRGB>cHRM/gAMA) + Table-17 sRGB-contradiction gate; all PNG3 chunks complete + every zlib inflate bomb-bounded (r445) |
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
| **AAC** | 🚧 ~95% — LC/HE-v1/v2/LD/SSR/BSAC decode; lacks the deployed ER-LD TNS filter record | ✅ ~97% — LC + HE-AAC v1 SBR + HE-AAC v2 PS encoders (own-decoder oracle IID 1.5 dB / ICC 0.11, black-box stereo decode clean) + PCE-described layouts + SBR VARVAR grids; lacks ≥3-onset SBR frames, 960/LD SBR |
| **CELT** | ✅ ~98% — reference-lockstep + PLC, fuzzed (RFC 8251 §8 energy-cap fix), ~20× realtime decode | ✅ ~92% — beats the reference listing at every measured rate |
| **Speex** | 🚧 ~85% — r450 crafted-bitstream probe round: fitted laws → measured closed forms (NB 25.6–33.3 dB hp-conformant, WB ≈20–21 dB incl. q10 −7→+20.7 dB, UWB outer band 46.8 dB / speech bands ≤1.1 dB) + measured output high-pass + stereo block phase sample-exact; NB conformance gate previously 10.5–14.4 dB raw / 13.1–19.5 dB high-passed (3-tap pitch-VQ lag-order + short-pitch recursion fixes) + crossover-shaped folded-HB law (WB speech 15.6 dB, HB VQ sub-modes 2/3 first-validated 18.3/18.2 dB) + §10.1 WB high-band per-sub-frame LSP interpolation + QMF synthesis filterbank (two half-bands → 16 kHz PCM, perfect-reconstruction-pinned) + forced open-loop pitch-gain reconstruction into NB decode (modes 1/8) + closed NB decode loop (LSP→LPC + §8.4 e[n]=p[n]+c[n] excitation feedback into the adaptive codebook + synthesis filter → full-frame PCM) + log-domain excitation-gain grid + WB sub-band decode loop (embedded NB+HB §10.4 → both half-band signals) + top-level `SpeexDecoder` packet walk (multi-frame, mixed NB/WB) + UWB framing recursion + LSP base-vector / Q-format pin (NB `.25·i+.25` / HB `.3125·i+.75` rad) + `LSP_MARGIN` min-spacing clamp → LSP set bounded inside `(0,π)` by construction (always-stable filter, validated non-divergent on real q8 fixture) + complete i16 PCM + header-mode-class public surface + UWB 3-layer decode externally validated (fold-source pinned: 19.1 dB/0.994 full 32 kHz, embedded WB layers 21.6 dB) + r446 provenance/08 QMF-recovered mode-4 replication ×2 through crate machinery (mean |ρ| 0.88/0.93) + UWB q10 bit-exact framing gate (76/76 frames incl. packed 80-bit mode-4 fields) + mode-4 gain-base discriminated (backward-adaptive state, not low band; probe-fixture-pinned); lacks exact HB gain-update rule (trace-asked) + bit-exactness + UWB-on-speech (pinned 3-layer divergence) + NB modes 1/7 innovation binding (docs-asked) | 🚧 ~60% — full NB CELP encoder (perceptual weighting + open/closed-loop pitch + innovation VQ + Table 9.1 frame writer; NarrowbandEncoder end-to-end encode→decode round-trip, all 9 modes parse-exact) + WB (SB-CELP) encoder (QMF analysis split + order-8 HB LPC/LSP + Table 10.1 packer + innovation VQ + closed-loop gain grid + packet-level encode_packet, decoder-round-tripped) + UWB 32 kHz encoder+decoder (§2.2 recursion both directions; mode-1 layer pinned via the RFC 5574 rate ladder) + quality→sub-mode ladders (every Table 10.2 rate exact) + VAD/DTX on all three classes + header-driven stream decoder + HB mode-1 fold law fixture-arbitrated ((−1)ⁿ at 1/(2√2); UWB layer-2 no longer silent) + on-wire layer-prefix grammar corrected + externally validated 16.7 dB full / 38.9 dB folded HB, CI-gated + r440 QMF-route arc (mode-3 sign/index wire fix; QMF bank provenance-exact ±0.5 dB; mode-4 state-derived gain + pinned polarity: 4–8 kHz band error 13.1→6.1 dB; exact float OL-gain quantiser law) — ~78%; lacks the exact HB gain law + modes-2/3 gain base (docs asks filed) + Table-11.x UWB VQ geometry + UWB fold-source fixture (docs-gapped) |
| **GSM 06.10** | ✅ — FR bit-exact both directions + 06.11 RX lost-frame legs; HR (06.20) decode chain complete (homing sample-exact; bit-exact blocked on the unstaged 06.06 arithmetic) | ✅ — FR bit-exact + DTX; HR frame-parameter analysis (R0 ~exact); lacks HR excitation analysis |
| **G.711** (μ/A-law) | ✅ 100% | ✅ 100% |
| **G.722** | ✅ ~97% — SB-ADPCM decoder BIT-EXACT vs the ITU conformance corpus (97,536/97,536 octets, all 3 modes) + QMF + auxiliary-data channel + clause-2/2.4 operational conformance (S/D gates, idle-channel limits, group delay ≤4 ms) + pcm16 API (clause-5.2 Note-2 rescaling) + fuzz + robustness totality | ✅ ~95% — SB-ADPCM encoder BIT-EXACT vs the ITU corpus (48,768/48,768; 3 arithmetic bugs fixed: QQ4 row addressing, FILTEP timing, UPPOL1 stability window) + Mode 2/3 round-trip + conformance meters + r435 Appendix II T-series 11/11 BIT-EXACT (enc+dec, all modes) + r439 Appendix IV PLC rebuilt fixed-point on the staged tables + STL operators (erasure-free prefixes bit-exact, SNR ≈13–14 dB, exactness floors raised); r442 FITTED smaller-pitch preference (user-ruled calibration vs staged vectors, labeled fitted): pitch ground truths 17/18 + 7/8 held-out, SNR ≈14–15 dB; residual attributed to the unstaged IV.7 instruction sequences |
| **G.723.1** | ✅ 100% | ✅ ~98% — both 5.3k + 6.3k, ITU-vector-verified wire (2816/2816 byte-identical repack) + near-bit-exact decode (PATHD53 corr 1.0000/+54 dB, OVERD53 0.9993; 3 vector-arbitrated interop corrections: LSP band 0 in the MSB byte, 1-subframe framer delay, unshifted Word16 output rail; 2 LPC→LSP bugs fixed) + §3.6 pitch postfilter; bit-exact OVER/TAME needs the clause-5 overflow protocol (trace ask filed) | 🟢 ~95% — full clause-2 chain with the printed §2.15/§2.16 searches (ACL±1 up to 99%, FCB subframe-exact 16→70%) + §2.3 fixed-point HPF + Annex A VAD/SID encoder (frame types 94%) + dtx fuzz; lacks bit-exact LSP-VQ/taming + CNG RNG |
| **G.728** | ✅ 100% | ✅ 100% |
| **G.729** | 🟢 ~90% — clause-5 fixed-point chain, frame-0 startup byte-exact on every clean vector, Annex B CNG with SID-LSP spectral envelope, corr 0.99+; lacks whole-vector bit-exact (§4.2 residual) | 🟡 ~78% — clause-3 encoder fixed-point through §3.3–§3.9 (locked frame-exact 27.5%, T1 81%; eq (28) is log10) + Annex B DTX/SID encoder leg; lacks bit-exact wire + §B.3 VAD |
| **IMA-ADPCM (AMV)** | ✅ 100% | ✅ 100% |
| **MS-ADPCM / IMA-ADPCM (WAV)** | ✅ 100% | ✅ 100% |
| **G.726** | ✅ 100% | ✅ 100% — ITU Appendix II conformance-proven (112/112 byte-exact), A/µ-law + SYNC, registry law option + r437 G.72x-in-WAV sub-block bit-cell framing (registry framing=wav both directions, stereo lanes, fmt-extension, VBR rate-switching pinned; the long-standing WAV blocker retired) + r442 staged VBR demo reference BIT-EXACT (52 736/52 736 linear-leg words, 3 295 mid-stream switches; 16-sample frame period settled empirically) + r445 IMA reference-compressor interchange mode (Appendix D §6.1/§6.2 ladder, double-entry oracle) + r449 WAV tags 0x0045/0x0064 claimed with container-derived framing defaults (10 WAV tags + ima4 total across the adpcm crate; encoders advertise wire tag + fmt extradata) |
| **OKI / Dialogic VOX** | ✅ 100% | ✅ 100% — mono + Dialogic stereo (encode_packet_multi) + r449 OKIADPCMWAVEFORMAT wPole fmt-extension + WAV tag 0x0017 claimed |
| **8SVX** | ✅ 100% | ✅ 100% |
| **iLBC** (RFC 3951) | ✅ 100% | ✅ 100% |
| **AC-3** (Dolby Digital) | ✅ ~97% — AC-3 + full Annex E decode + E-AC-3 JOC/OAMD object reconstruction with opt-in stereo presentation (community #15, provenance-reviewed) | ✅ ~97% — all Annex E tools + fractional syncframes + TPNP; RD tuning lags the reference encoder |
| **AC-4** (Dolby) | 🚧 ~99% — A-JOC static-dmx A-SPX/A-CPL carrier synthesis into the object path + dialogue enhancement applied on both routes; fuzz-hardened (5 fix rounds) | 🚧 ~95% — framework Encoder + registry options, tight substream sizing, ASF band gate, frame-rate matrix, dialogue-enhancement authoring, ICE + multi-envelope P-frames; lacks 5.X/7.X A-CPL parity, short-frame ASF, rate control |
| **MIDI** (SMF) | ✅ ~99% — SMF 0/1/2 → PCM + soundfonts + MIDI 2.0: full UMP, .midi2 clip playback, MIDI-CI surface, 1.0↔2.0 translation | ✅ ~95% — SMF + .midi2 clip writers + synthesis |
| **NSF** (NES) | 🚧 ~98% — full 6502 + 2A03 APU + six expansion chips + VRC7/OPLL pipeline incl. envelope ladder + rhythm mode + NSF v1/v2/NSFe + NSFe mixe per-device default mix + §8a VRC7/OPLL AM tremolo (silicon-measured 14-level truncated triangle, ≈4.8 dB) + VIB bit-exact (§8b 8×8 PM table) + Namco 163 sum/divide multi-channel mixing + YM2413 rhythm noise generator (23-bit x²³+x⁹+1 LFSR) + §7 EG rate-increment model (eg_shift/eg_select) + §9 phase-gen 10.9 fixed-point (correct VRC7/OPLL pitch) + end-to-end frame-render gate + §7 global-counter EG model wired into live decay/release + §7 global-counter-driven OPLL attack envelope (silicon-measured 12-level sequence) + VRC7 user-patch live-reload fix + 2A03 APU per-sample accuracy (full-rate noise LFSR clock + frame-counter 29830/33254 schedule + post-DAC HP/HP/LP filter chain + pulse bass-note sweep-mute fix) + cycle-exact frame counter (half-cycle events, 3-cycle IRQ window, 5-step cadence fix) + $4015/$4017 contracts + DMC DMA CPU stalls + pre-INIT scrub + all six expansion chips batch-invariant (VRC6 $9003 shift + MMC5 APU-rate pulses + S5B/N163/FDS remainder-carry + MMC5 polarity) + NSFe VRC7 patch-sets + time/fade schedule + r394 dedicated-page sweep muting (unconditional shift-0 adder mute) + load/reload DMC DMA cadence + $4014 OAM-DMA halt (was a no-op); + full NSF/NSF2/NSFe container read+write (lossless v1↔NSFe, well-formedness, UTF-8 strings; fixed a real wrong-starting-track bug) + typed per-track metadata; + r434 NSF2 §2.6 reconciled (forbidden-four typed, NEND advisory, $7C bit-7 semantics locked) + registry glue tested + track durations surfaced + r448 sub-instruction DMA engine (halt-cycle-exact DMC/OAM incl. the documented stop bugs, $4017 reset-delay parity fix, true-write-cycle register deferral, KIL clock semantics); lacks §7a attack recurrence + VRC7 rhythm phases (both docs-gapped; rhythm.md staging asked) + per-access 6502 read timing | — synthesis only |
| **Shorten** (.shn) | ✅ ~95% — fuzzed (5 daily targets; 6 defects fixed incl. a hostile-reservation OOM); lacks the spec-blocked filetype codes | ✅ ~90% — v2/v3 QLPC auto-select, property-tested round trips |
| **TTA** (True Audio) | ✅ ~99% — TTA1 fmt 1/2 + password + trailers + streaming + random-access + §04 decorrelation pinned vs captured reference tape (31-row §7.1 + N>2 cascade + end-to-end pipeline) + §4.3 unseekable-mode (corrupt seek-table CRC → linear-only, random-access refused) + §4.4 empty-stream + fuzz; decode −18% then r386 −17…−25% more (u64 reader + slice-by-8 CRC), byte-identical | ✅ ~97% — bit-exact self-roundtrip, wire-identical to the external encoder (black-box), decode +12% r456, 4 fuzz targets; lacks format=3 float (spec undefined — ruling asked) |
| **Musepack** | ✅ ~98% — SV7+SV8 ±1 LSB vs oracles; seek-table thinning + ID3v2/APEv2 tag pass-through | ✅ ~95% — from-PCM SV7+SV8 at 1-LSB reference parity; SMR quality ladder; dual-generation typed registry options |
| **Cook** (RealMedia) | 🚧 ~70% — r450 §0.2 pre-spectral walk + §2.2 stage-2 refinement vendor-exact (categories 34/34 bit-exact on all traced live frames; synthetic frames → non-silent PCM; multi-frame streaming) — blocked on unstaged envelope/coupling VLC trees (asks filed); flavor/cookie parsers + every extracted DSP table behind typed range-guarded APIs + decode-session orchestrator + per-band quantiser primitives + backend frame-syntax codebook/vector-dim geometry + joint-stereo mirror-index rotation + MSB-first frame bit reader (spec/05) + §1 gain-control envelope (segment-count + √2 ladder) + §0–§3 frame-body decode orchestrator (gain application + §2.1 subband geometry) driven on a real RA Cook stream + §2.2 full per-band quantiser closed form (clip·round·divisor) + §3.1 dequant scale triple + spectral-coefficient assembly + §3.1 per-band symbol/coefficient grouping arithmetic + §3.1 per-band/spectrum reconstruction + §4.2 joint-stereo decouple + frame-body integration (FrameSpectrum mono/stereo) + §2.2 division-free quantiser-index decomposition + §4.1 coupling-control read + full §5 synthesis back end (O(N log N) IMLT → Princen-Bradley window → gain → overlap-add → 16-bit PCM; per-call cadence byte-exact vs validator) + §2.2 category-assignment/bit-allocation loop recovered + wired (validator-exact base pass 25/25 + uniform refinement; real-frame decode blocked on stack-resident v[]/budget/M + §1.2 gain VLC) + §3.2 spectral codebooks vendored (all 1301 symbols round-trip, 7 codebooks) + §3.1 VLC walk + §2.2 cost LUT + entropy→digit bridge + §3.1 band decode (codebook-by-category, magnitude+sign) + level→value expectation rows + recovered §4.3 coupling + N=1024 window synthesis + registry/make_decoder — ~86%; entropy→PCM audible; r433 §2.2 category-assignment constants vendored+bit-locked (last unwired staged table); real-stream PCM gated on the pre-spectral read layout (v[] formation + gain/quant VLC selection; ask filed) | — |
| **WMA** | 🟢 ~90% — vendor v2 decode at 45–60 dB on the closing families (noise-substitution stream unmasked, measured policy); LSP-envelope family docs-gapped | 🟢 ~75% — vendor-wire encoder end-to-end, reference-accepted (corr² 0.98–0.995, gain ≈ 1) |
| **WavPack** | ✅ ~100% decode (post-2026-05-18 orphan) — v4 block/metadata/entropy parse + full §4.2 entropy ladder + multi-block PCM composer + inverse entropy encoder (bit-exact round-trip) + r447 ENCODE ORIGINATION MATRIX CLOSED — every format × channel width, lossless + hybrid pair (stereo-pair member mode search 50.8–61.1%, float/int32 + hybrid multichannel, registry-wired with per-member-chain carries; §4.4/§4.4.1 shaping spec-pinned closing #309; 90 black-box bit-exact checks) + typed-refuses joint-stereo/cross-decorrelation (flag bits 4/5) + §5 running block-CRC compute/verify + §3.1/§3.4/§3.5 decorrelation inverse-prediction weight arithmetic (apply/update_weight primitives) + §3.2/§3.7 decorrelation inverse-prediction loop (all terms, mono+stereo, round-trip-pinned) + mono-lossless decode_samples → reconstructed PCM (§3.7 reverse-storage + multi-pass assembly) + §5 decoded-CRC verify + extension-stream CRC accumulator (crc_x / ExtensionCrc) + §3.2/§3.7 stereo decorrelation prediction loop + joint-stereo undo wired into block decode + §5.6 stream-level CRC mute gate + §1 left-shift final-normalization fixup (sub-byte 12/20-bit depths, CRC folds pre-shift) + §4.1 hybrid correction-fold arithmetic (fold/split + placement selector, block-level) + multichannel grouping decode (member-set channel interleave, bit-exact) + foreign-file decode bit-exact (19/19 reference-encoded fixtures via wp_log2/exp2 + 0x04 seed-prefix rule) + float (0x08, wvx layouts + crc_x erratum found; SHIFT_SAME + EXCEPTIONS sentinel/NaN layouts) + int32 (0x09/0x0C) + sample-rate surface (0x27, seek_seconds) + typed ChannelInfo (0x0D) + hybrid lossy decode end-to-end (§6.5 error_limit/bitrate model, mono/stereo/5.1/24-bit/float, black-box bit-exact) + hybrid-lossless wv+wvc pair decode (0x0B bracket completion + 0x07 shaping; pairs reproduce original encoder input bit-exactly) + false-stereo output fix + joint-stereo/float/int32/multichannel hybrid-lossless wv+wvc pairs bit-exact (54 fixtures / 18 pairs; pair-CRC gate, pair-aware seek, differential pair fuzz + 2 overflow fixes) — every reference-encoder shape decodes bit-exact; lacks DSD (docs-gapped) | ✅ ~97% — lossless + hybrid + float/int32 encode with 0x07 noise-shaping origination (Off/Static/Ramp, .wvc correction pairs) + registry-wired options; 36/36 reference-bit-exact battery incl. r418 hybrid edge probes + ramp rail-saturation pin + r436 hostile-input decode budgets on every eager surface (typed pre-decode refusal, per-packet registry budget, bounded seek, ~1.2M-run differential fuzz clean); lacks DSD + negative-IIR-past-rail (both docs-gapped) |
| **APE** (Monkey's Audio) | ✅ ~95% — 3990 decode byte-exact both directions vs the vendor corpus (levels 1000–5000); lacks pre-3990 archive proof | ✅ ~90% — full 3990 encoder, all levels at 8/16/24-bit, vendor-verified (280/280); lacks pre-3990 + multichannel |
| **DTS** (Core) | ✅ ~98% — Core complete; output level reference-calibrated | 🟢 ~85% — Core encoder: QMF/LFE front end, ADPCM, HF-VQ, transients, Huffman, all rates/layouts, DRC/joint/aux opt-in, oracle-validated; lacks the unprinted SA129–SE129 books (docs ask) |
| **aptX** (classic + HD) | 🚧 stub — NDA-blocked; clean-room QMF + 4-subband quantiser source-of-record purged in the 2026-05-06 audit (trace docs failed clean-room separation). Awaiting a non-contaminated `docs/audio/aptx/` (public-primary-source tables or black-box observer trace) | — |

</details>

<details>
<summary><strong>Video</strong> (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **MJPEG** | ✅ 100% — §K.7.2 hierarchical DCT progression (SOF0/1/2 + SOF5 diff, 1/3-comp RGB+YUV incl. P=12 12-bit YUV/grayscale/RGB + 4-comp CMYK/YCCK) + baseline + progressive + lossless (Huffman + arithmetic, incl. SOF11 subsampled YUV-class) + 12-bit + CMYK/YCCK + RTP/JPEG + DNL (SOF Y=0, T.81 §B.2.5) + Annex J hierarchical spatial-lossless decode (1/3/4-component, EXP ×2 bi-linear upsample, bit-exact) + SOF6 differential-progressive + SOF7/SOF15 DCT-terminating + SOF11/SOF15 arithmetic spatial-lossless progression + SOF9/10/13/14 hierarchical arithmetic DCT progression (every defined SOFn family) + §C Kraft-inequality DHT over-subscription guard + typed APP0/APP14/ICC views + fuzz + golden-pinned perf pass (decode −6..11%) | ✅ 100% — every SOFn family now EMITTED too: full Annex J hierarchical encoder (spatial-lossless + DCT + progressive + arithmetic stage families, lossless-terminated bit-exact pyramids; ecosystem-first — external decoders reject DHP) + baseline + progressive + lossless + arithmetic + CMYK/RGB24/grayscale; SA-progressive AC scans conformance-fixed (band-terminating EOB + §A.4 integer divide), encode −13..15% |
| **FFV1** | ✅ 100% | ✅ 100% |
| **MPEG-1 video** | ✅ ~96% — whole-stream I/P/B/D decode reference-conformant (3-stream corpus ±2/sample) + arbitrary slice structures (mid-row / multi-slice / row-spanning; walker's first-increment rule was a real bug); lacks 11172-2 lower layers under scalability | 🚧 ~90% — dedicated 11172-2 encode path: I/P/B + GOP-structured display-order assembly (per-GOP timecodes, closed_gop) + §2.4.3.2 constrained-parameters check + downloadable quant matrices + full-pel vectors + Tables B.5a–f entropy writers with exhaustive round-trips; 10-stream pinned corpus, strict black-box decode clean + r440 Annex C CBR rate control (exact integer VBV + whole-stream verifier, real vbv_delay, constrained-params preserved) + r443 D-picture encode (§2.4.3.4 type-4, sample-exact both decode paths) — encoder feature-complete |
| **MPEG-2 video** | ✅ ~97% — 12/12 conformance corpus + §7.10 data partitioning + SNR/temporal/spatial two-layer loops (self-made enhancement encoders as oracle) + vertical_size>2800; lacks interlaced spatial weight tables, chroma_simulcast | ✅ ~95% — interlaced encode incl. skips, concealment MVs, 3:2 pulldown, full entropy flags, 4:2:2/4:4:4 on every picture structure, typed runtime encoder options (full assembler surface), scalable enhancement-layer encoders; lacks field pictures in layered loops |
| **MPEG-4 Part 2** | ✅ ~97% — 24-stream corpus + S(GMC)/DP/RVLC/interlaced decode + short-header read side; 3 opt-in ecosystem-compat divergences pinned | 🟢 ~85% — I/P/B/S(GMC) + interlaced I/P/B/S both ways + short header both ways + 2/3-point affine GMC + DC-VLC election/HEC bodies, 20/21 black-box bit-exact pairs, fuzzed; lacks budget-driven dquant/dbquant, interlaced data partitioning |
| **Theora** | ✅ 100% | ✅ ~93% — RDOQ + seeded ME + MVMODE election + two-pass RC (−21% BD-rate r453; chroma quarter-pel fix); lacks lookahead scene-cut |
| **H.263** | ✅ ~99% (post-2026-05-18 orphan) — r450 registry wired (streaming decoder/encoder adapters, dual-API factories, tags + payload magics) + Annex R ISD + Annex V DPS + Annex L/W.6 SEI + §W.5.3 fixed-point IDCT0/FDCT oracle-exact + 2 fuzz DoS fixes (lacks Annex U); r447 UMV+ Table-D.3 nonconformance CLOSED both directions (fixture + black-box proven; UUI "Unlimited" range decode, RRU×UMV pseudo-vectors, AP×UMV+ four-vector emission, §P.2.2 RPRP emulation-prevention fix) + baseline + Annexes D/F/I/J + OBMC + PLUSPTYPE + Annex G PB-frames + Annex K slice-structured + Annex M improved-PB-frames + Annex T modified-quantization (MQ-active picture decode end-to-end on baseline + Annex I AIC paths: §T.2 DQUANT + §T.3 QUANT_C + §T.4 extended-range) + Annex S Alt INTER VLC (§S.2/§S.3) + Annex Q reduced-resolution update (§Q.6 prediction-error upsample + §Q.7 block-boundary filter) + §5.2.2 first-GOB header elision + §5.1.11–§5.1.16 PLUSPTYPE scalability (Annex N/O/P enhancement-layer header) + Annex O scalability macroblock-layer VLCs (Tables O.1–O.4) + Annex O EI + EP enhancement-layer end-to-end reconstruction + Annex N forward-channel RPS to pixels (§N.5 store + §N.4.1 picture/per-GOB/per-slice NEWPRED TRP re-selection) + Annex P RPR (implicit §P.1 + explicit §P.2 reference resampling/warp → pixels) + §5.1.3 decode_sequence PLUSPTYPE picture dispatch (extended-PTYPE end-to-end + custom-PCF accept) + Annex-K slice PEI/SEPB1 ordering fix + Annex T MQ + Annex S AIV threaded through the slice driver + 6 mode-coverage byte-exact conformance fixtures + reference-encoder-fixture baseline decode conformance; + Annex G PB-frames + Annex M Improved-PB streamed through `decode_sequence` + Deblocking-Filter-mode four motion vectors (§5.3.8/Table J.1, no-OBMC INTER4V); + r443 Figure-F.1 predictor fixes + skip-MB OBMC (spec default, compat flag) + Annex Q RRU decode + §N.4.2 BCM parse; lacks EP-picture lower-layer RPRP | 🟡 ~80% — I/P/PB(MVDB) + UMV/AP/AIC/T/K/J/SAC/RRU + two-level rate control, oracle-validated; lacks Annex M emit, AIC-in-P |
| **H.261** | ✅ ~99% — I+P + loop filter + BCH error correction + RTP/RTCP/SDP + Annex A + Annex C conformance + fuzz-hardened (6 targets; packetiser MV-desync panic fixed) | ✅ ~98% — ME + rate control + §3.4 forced-update cyclic INTRA refresh + §4.2.3.1 MBA-stuffing emit/pad (§5.2 HRD buffer regulation) + BCH/RTP framing + §3.1/§4.2.1.2 temporal-reference + picture-rate (TR tracking, picture-rate-driven encode, §4.3.1 freeze clock); 45 dB at 64 kbit/s QCIF |
| **MS-MPEG-4** (v1/v2/v3) | 🚧 ~90% — Microsoft fixtures decode end-to-end (mp43 400/400 MBs, 49/50 frames, ~99.7% Y exact); lacks exact integer IDCT (±1 pel) + DIV3 intra-in-P prediction | ✅ ~90% — v1/v2/v3 I+P decoder-verified, AC-pred + escape ladder mirrored |
| **H.264** | ✅ ~97% — all JVT streams byte-exact; FMO decode + non-flat scaling-matrix dequant fixed r453 | ✅ ~98% — PAFF + MBAFF (CAVLC+CABAC, 8x8), 10/12/14-bit CAVLC+CABAC + lossless bypass (4:2:0/4:2:2/4:4:4; 4:2:2 chroma-DC qP+3 fix), weighted P, multi-ref/MMCO/RPLM, FMO/ASO, trellis RDOQ, SEI writers; lacks B in MBAFF, Intra_NxN at >8-bit |
| **H.265 (HEVC)** | 🚧 ~99% — r451 encoder RC CLOSED: pyramid VBV (per-AU decode-instant enforcement all GOP shapes) + HRD signalling VBR/CBR Annex-C-conformant (exact u128 clock, BP/PT SEI, filler-data CBR schedule) + rate accuracy ±1.6% across the matrix + §D.2.2/.3 SEI parsers; Rext RDPCM + SCC palette decode land (self-built pins; 2 documented reference-decoder deviations, official Rext/SCC bitstreams asked) + 4:4:4 PART_NxN chroma-mode fix + 41-stream tool-axis sweep BYTE-EXACT (B-pyramids / scaling lists / transform-skip / constrained-intra / WPP+slices / 4:2:2+4:4:4; 8 bugs fixed incl. §8.5.3.2.9 listCol + unapplied §8.6.3 scaling lists) + parameter sets + §9.3 CABAC engine with COMPLETE §9.3.2.2 context-init (Tables 9-5..9-42) + §8.7.2 in-loop deblock complete (edge-flag + CU + picture drivers) + full slice header + residual_coding() driver + §D.2 SEI parse (mastering-display + content-light + recovery-point + decoded-picture-hash) + §8.6.2/§8.6.3/§8.6.4 scaling + inverse transform + §7.3.8.9 mvd_coding + §7.3.8.6 merge_flag binarization + §8.4.2/§8.4.3 intra pred-mode derivation + §8.4.4.2 intra sample prediction (substitution/filtering/planar/DC/angular) + §6.4/§6.5 z-scan/tile-scan neighbour availability + §7.3.8.10 transform_unit() + §7.3.8.8 transform_tree() recursion syntax drivers + §7.3.8 slice-data CTU/CU CABAC syntax walk + §8.4 intra sample reconstruction (§8.6.4 transform-orientation fix + §8.6.1 Qp derivation + picture buffer — tiny-i IDR slice decodes to byte-exact pixels) + §8.7.3 SAO apply (edge + band CTB modification + §7.4.9.3 SaoOffsetVal) + Table 8-12 β′/tC′ + §8.7.2.5 deblocking luma/chroma sample-filter kernels + §8.5.3 inter (P/B) PU reconstruction to pixels (§8.5.3.2 MV/chroma-MV resolution + 8-tap luma/4-tap chroma MC interp + default-weighted bi-pred → bit-exact) + §8.7.2.4 deblocking bS derivation + §8.4.2 neighbour-aware intra MPM (IntraModeField + §6.4.1 z-scan reference availability) + reconstruct_intra_picture multi-CTU recon+SAO driver (tiny-i IDR byte-exact) + §8.5.3.2 spatial/temporal merge + AMVP MVP candidate derivation; + §7.3.2.2.2/.3.2 range-extension SPS/PPS + §7.3.2.2.3/.3.3 SCC-extension SPS/PPS/slice bodies decoded in place; + §8.3.1-8.3.5 POC/DPB/RPS/RefPicList/ColPic state machine + §8.5.3.2.8 temporal collocated MV + multi-slice neighbour isolation (SliceAddrRs) + §8.5.3.2.1/§7.3.8.6 per-PU MV-resolution + partition-geometry driver (pu_mv module) + §8.5 per-CU inter reconstruction + picture-level inter driver (mixed intra/inter, §8.7.2 deblock + §8.7.3 SAO in-loop chain) + §8.3 decode_inter_picture DPB reference cycle (IDR→P reconstructs+stores against an in-DPB reference); + whole-bitstream Annex B decode driver + WPP substreams + registry decoder — 16/16 conformance fixtures byte-exact (I/P/B pyramid, Main10, 4:2:2/4:4:4 10-bit, multi-slice, WPP); + explicit weighted prediction + PCM decode + dependent slice segments + per-slice loop-filter flags + hvcC extradata + §8.5.3.2.3 merge-pruning fix + true tiles byte-exact (staged fixtures; multi-tile slice segments both directions, per-tile CABAC reset + entry points); decoder conformance surface complete; + Rext/SCC tail APPLIED: §8.6.6 CCP wired into intra+inter chroma residuals (black-box byte-exact pin), §8.6.8 ACT end-to-end (ACT-qP + a parse-gate fix), IBC current-picture referencing (AMVP/merge integer-MV) — sweep 41→44 with self-built SCC pins | 🟢 ~88% — quadtree encoder + RDOQ (−9.4% BD-rate) / SDH / RQT depth 1..3 + 8x4/4x8 PUs / WP estimation / scaling lists / WPP + tiles (tile-parallel pass 1), r451 pyramid VBV+HRD RC; lacks SCC-tool encoding, WPP-parallel pass 1 |
| **H.266 (VVC)** | ✅ 100% — 56/56 JVET streams byte-exact (subpictures, 4:2:2/4:4:4, IBC + SCIPU dual-tree chroma closed r456); lacks a cargo-fuzz sub-crate (depth mode next) | 🚧 ~95% — LMCS chroma residual scaling + forward CABAC + DCT-II + MTT RDO + P/B + sub-pel MC + weighted bi-pred + affine/AMVR/BCW dispatchers |
| **VP6** | 🚧 ~92% — r450 P-frame MB(0,31) inter MC+recon pixel-exact gate (§11.4 fractional-pel + inter residual on vendor coefficients; §10 mode grammar synthesis-confirmed, §11.1 MV wire fixture-under-determined — extraction asked); §16 IDCT BIT-EXACT (555-block arbitration; r390 rounding erratum corrected) + r447 §13 Table-18 MSB-first extra-bit erratum (fixture-arbitrated) + P-frame beachhead (arithmetic tokens vendor-wire-pinned through the 189-block prefix + first content MB; 31-MB pixel-exact prefix; §10/§11 P-frame wire extraction-asked) + keyframe parse-exact prefix 12→31 MBs via black-box bit-flip probing (chroma DC tree/+128-seed findings) + BoolCoder + DC/AC coefficient decode + MV decode/reconstruction + custom scan + per-block reconstruction + §11.4 FilterVarThresh resolve + §11.5 variance edge-clamp + header-driven FilterConfig + §2/§13/§17 block-to-plane raster frame assembly + §9 output-scaling typed surface + §10 macroblock coding-mode traversal + §10 frame-level MB mode-decode pass (availability-aware prob-row select) + §9 BoolCoder frame-header tail + §7.3 BoolEncoder (decoder-matched range encoder) + §10/§11 per-MB MV resolution (Zero/New/Nearest/Near) + §16 inverse DCT (i64-widened descale) + §17.1 per-MB intra decode loop (I-frame decodes END-TO-END to output pixels) + §17.2/.3 integer-MV inter recon + FourMV macroblock resolution + §17.4 sub-pixel MC predictor (§11.4 bilinear/bicubic) + §11.3 loop filter (BoundaryX/Y round-toward-zero fix) + fused inter (P-frame) per-MB decode driver → END-TO-END pixels + §4 golden-frame ReferenceFrames bookkeeping + top-level per-frame Vp6Decoder assembly driver (§9 header→keyframe/inter dispatch) + registry `Decoder` (codec id vp6, tags VP60/VP61/VP62/vp6f) + §8 Figure-1/Figure-5 coeff-prob-update sub-stream ordering (I+P, real NewNodeProbValue round-trip) + keyframe→P GOP end-to-end + r439 full-frame Huffman coefficient decode (Extractor-03 band selectors + corrected prob banks + §7.2.1 tie-break + carry-forward fill; §13.3.1 Prec-seed, §14 truncation and chroma-seed errata fixed): real vp6f keyframe PIXEL-EXACT end-to-end, 854×480 all 9 720 blocks, CI-gated | 🚧 ~70% — P-frame encode (encode_inter_frame all-CODE_INTER_NO_MV + §10 mode_encode; keyframe→P GOP round-trip) + intra (I-frame) encode: §16-dual forward DCT + quantise + §13 token coding (DC/AC trees + §13.3.3.1 zero-run) + §9/§14 header emit → decoder-reconstructible keyframe (flat-exact; ~44 dB at q=48) + §11.1 MV component encoder (decode_mv inverse) + motion-estimated P-frame encode (two-stage box+¼-pel SAD ME → CODE_INTER_PLUS_MV/NEAREST/NEAR, decoder-reconstructible) + self-describing ME packets + Golden-frame + FourMV encode modes + bit-budget rate control + oxideav-core Encoder registration + §13 Huffman coefficient coder (decode+encode) + §6 MultiStream two-partition transport (every P shape, bit-identical to single-stream) + §13 P-frame prob re-training + cross-frame bank persistence + errata #155 FourMV representative + third-party vp6f fixture: 4 printed-spec errata fixed (MB-unit geometry, partition-1 spans, DC-tree fold, IDCT rounding) + leading-MB pixel-exact real-stream decode, CI-gated; + r439 multistream Huffman keyframe encode with carry-encoded retrained banks (round-tripped); + r442 §9 output scaling BOTH ways (typed placement modes, downsampled encode, letterbox/center gates; §14/Prec-seed errata spec-prose-corroborated); lacks P-frame wire conformance (behavioural trace ask filed) |
| **VP8** | ✅ 100% | ✅ 100% |
| **VP9** | ✅ ~97% — 41/41 corpus byte-exact, tile-parallel decode; registry carries the full 12-format §7.2.2 matrix incl. 4:4:0 (core 0.1.35) | ✅ ~99% — alt-ref pyramid, all SEG_LVL features, tiles, scaled-ref, §8.4 writer-side adaptation + cost-elected forward updates (−7.4% bytes, recon bit-identical), switchable interp filter, two-pass VBV RC (99.9–100% of target), full-format structured/resized entries; lacks sub-8x8/scaled-leaf filter election, structured/HBD two-pass |
| **AV1** | ✅ ~99% — 155-stream corpus byte-exact; decode peak RSS −35% (byte-identical) | 🟢 ~99% — pyramid short-ref + delta-q/lossless tables + inter-frame superres (scaled refs) + inter-layer SVC + S×T operating points + lag-3 grain, corpus 164 byte-exact via dav1d/ffmpeg/aomdec; lacks compound/opener inter-layer, pyramid mid-GOP superres |
| **Dirac / VC-2** | ✅ ~99% — VC-2 LD+HQ + Dirac intra/inter + OBMC + 7 wavelets + 10/12-bit + fragmented pictures + asymmetric transforms + §15.8.10 reference-exact out-of-frame sub-pel edge extension + DEEP COLOUR to 16-bit (>12-bit on Yuv*P16Le, 11/12-bit native P12Le; corpus 12/12 bit-exact incl. 16-bit fragments) + DIRAC_TRACE MV/plane tracing per the docs trace contract | ✅ ~99% — HQ+LD + sub-pel 2-ref bipred + rate control + inter sequence-level rate driver (PerPicture/Cbr/Vbv/VbvHysteresis residue-byte) + §11.3.3 spatial-partition codeblock grid (per-codeblock differential quantiser; bit-exact ≥4×4-sample; 1-ref + bipred; codeblock-aware rate control) + asymmetric transforms + §11.2.6 global-motion encode (P/B/sequence-driver + per-block gmode + pan auto-fit from ME grid; oracle bit-exact cross-decode + 120-case fuzz) + affine/perspective global-model estimation (−31% zoom; estimated-pan oracle bit-exact) + §14 fragment emitter (bit-exact reassembly) + pan_tilt_all zero-matrix fix + §15.8.5 per-component intra-DC fix + 1-ref/bipred inter chains cross-decode bit-exact (literal block params + explicit zero-residue route around two reference-decoder quirks) + 13–16-bit deep-colour intra AND INTER + rate control (u16 P/B incl. codeblock grid + global motion + all four residue-rate drivers; 10/10 roundtrips q0 bit-exact at 10/12/16-bit, 16-bit Cbr within ±2.4% of target; deep inter self-validated — the external validator corrupts >8-bit inter chains it accepts) + r436 closed-loop reference-P chains + I/P/B GOP driver (8–16-bit, 4-variant rate control) + legacy container tags co-claimed (V_DIRAC/drac, 0.95 long-GOP probe); lacks third-party deep-inter ground truth (docs-asked) |
| **VC-2** (standalone `oxideav-vc2`) | ✅ ST 2042-1 intra decode complete — §14 fragment reassembly (bit-identical to unfragmented) + full Annex D (D.1–D.8 incl. asymmetric) + registered `Decoder` (8–16-bit YUV incl. Table-10 presets 7/8 on Yuv*P16Le) + MIXED ≤12-bit custom depths via the core significant-bits side-channel (represent-don't-promote ruling; 12/10 fixture pinned) + FourCC `BBCD` tag claimed + r435 registry-grounded container tags (Matroska V_DIRAC + MP4 drac / OTI 0xA4; weak probe so Dirac wins contested streams) + Table-10 presets 1–8 grounded from the staged transcription + full §11.4 source-parameter retention + ST 2042-2 levels + §12.2 conformance walker + ST 2042-4 MXF mapping data + 8-fixture matrix + truncation/DoS-hardened | ✅ — full ST 2042-1 encoder: LD+HQ, all 7 wavelets, asymmetric, fragments, rate control; 11/14 black-box bit-exact (rest = validator envelope) |
| **AMV video** | ✅ ~92% — r451 both device profiles conformant (2nd fixture 2928/2928 + graded duration cross-check; §4b step-index settled u8@+2) + hostile chunk-size prealloc capped (fuzz find #3) + geometry×fps corpus matrix; typed frame-geometry binding + §4 demuxer 1:1 video:audio interleave cross-check + §4a device-stripped JPEG reconstruction (splices Annex K DQT/DHT + baseline SOF0 4:2:0 → conforming JFIF) + full baseline-JPEG frame decode to RGB (device-hardcoded video tables wired, #127) + r432 selectable triangle chroma upsampling (MAE 0.04/ch vs black-box reference, 30×) + 2 fuzz-found hostile-input crash fixes | 🚧 ~85% — §4a RGB→00dc device-locked baseline-JPEG video encoder + §4b PCM→01wb IMA-ADPCM audio encoder + full decode→encode→`AmvMuxer`→demux→decode round-trip on comedian.amv (1116/1116 chunks, video MAE <3/ch, audio byte-idempotent) + intrinsic codecs registry-wired (amv_video + adpcm_amv as oxideav-core Decoder/Encoder; demuxer declares ids so the pipeline auto-resolves) + native planar YUV420P decode/encode path (skips the lossy RGB hop) + video rate control (measured bitrate targeting, >94% budget utilization) + device-envelope muxer validation + 9× encode speedup (precomputed cosine basis) + fuzz + r432 exact Lagrangian RD coefficient planning (worst sampled MAE 6.87→4.69/ch at identical rate) + λ warm start (255 µs steady-state binding encode); lacks second-device fixture + §4b non-zero initialStepIndex confirmation |
| **ProRes** | ✅ 100% | ✅ 100% |
| **EVC** (MPEG-5) | 🟢 ~90% — Baseline + Main tools, §8.3 RPL bookkeeping, errata #313; 23094-4 gate red on filed entropy asks | 🟢 ~75% — intra/P/B GOPs, EIPD 33-mode intra, exact-bit RD + RDOQ trellis (P/B −51% BD-rate from λ recalibration), one-/two-pass RC (±0.25%), fuzzed; lacks BTT, ATS, ADCC, ALF |
| **HuffYUV** / FFVHuff | ✅ ~97% — HFYU/FFVH + 6 predictors + interlaced + fast-LUT decode + fuzz; r419 profile round: decode wall −13..−23% (173–247 MiB/s), 78 golden pins byte-invariant; r420 two-worker interlaced decode 1.3–1.5× under ExecutionContext (serial default, 78 pins byte-invariant, differential fuzz oracles); r429 serial decode −23..−33% (full pair LUTs; r440 RGB24 phase-flip pairing — every codeword on pair-LUT reads, 2-worker 720p decode 1.35×, serial≡parallel pinned) | ✅ ~97% — v1.x + v2.x symmetric encode across YUY2/RGB24/RGB32; encode 1.9–2.9× faster (ClassicV2 up to ~1 GiB/s), HD bench gates; two-worker interlaced encode 1.7–2.0×; r429 full dual-API (registry Encoder + 78-pin trait-path byte-lock), budget-threaded auto encode 2.15×@3w; r440 lean CustomV2 tables (encode −7..−10%) — perf near-saturated, remaining levers Amdahl/entropy-bounded |
| **Lagarith** | ✅ ~98% — r451 wire-semantics fixes oracle-arbitrated (absorbing top-symbol interval, YV12 first-column Yuv rule, complete YUY2 predictor recovery); all 11 wire types + modern range coder (0x180001050 model normalizer recovered+wired, non-pow2 totals reference-exact) + legacy adaptive-CDF + typed header surface + fuzz + ~3-4% faster (shift-quotient, bit-identical) | ✅ ~95% — r451 THIRD-PARTY-VALIDATED byte-exact (19/26 black-box oracle matrix; 7 skips oracle-side) + NULL emission + PTS passthrough (lacks vendor-fixture parity, operator upload pending); all 11 wire types + all-nine-sub-form encodability proven + byte-exact self-roundtrip across exhaustive matrix (incl. YUY2 odd-width + YV12 odd-dim SPECGAP closure) + 1900-iter fuzz + public encode_frame API + framework Encoder trait registration + dual-API make_encoder/make_decoder factories + NULL-frame framework decode + range-coder reciprocal-multiply LUT machine-checked (floor(2³²/i)) + r432 cost-modeled form/model election (closed-form Fibonacci-prefix + entropy cost, never-larger by construction; −0.2–0.6% size, −22% selector time, −68% random-content time) + full-capacity RLE escapes + encode_frame fuzz loop + absurd-dims hardening; byte-exact-vs-proprietary blocked on missing fixture |
| **Ut Video** | ✅ ~98% — 5 FourCCs × 4 predictors + slice-parallel decode (5.6× at 720p) + opt-in strict-padding conformance decode + panic-free inspector Kraft accessors + 19-fixture reference-golden byte-exact corpus (spec/04 §5.0 median-mode modular-gradient fix); r420 ExecutionContext contract (serial default, budgeted slice-parallel 5.8×@4, 19-fixture invariance-proven) | ✅ ~98% — slice-parallel encode (3.3×) + reference-decoder-verified interop (incl. gradient mode 2) + zero-length-slice interop guard + fuzz oracle + reference-seeded fuzz corpora; budgeted encode 3.5×@4 under the same contract |
| **MagicYUV** | ✅ 100% | ✅ 100% |
| **Cinepak** (CVID) | ✅ ~98% — full CVID intra/inter + Sega FILM demuxer + Saturn/3DO deviants + typed walkers + fuzz + 42-rule spec-cited wire-format conformance linter (frame→strip→chunk→vector, vintage/seek profiles; own encoder held to zero findings); decode 4.4 GiB/s | ✅ ~98% — rolling codebooks + RDO/LBG + rate control + skip-free-aware 0x3000/0x3200 inter vector dispatch + encode-roundtrip fuzz; 34.2 dB PSNR; r430 profile round: encoder hot paths −23…−31% (output-invariant, 15-scenario golden-pinned) |
| **SVQ1/SVQ3** (Sorenson) | 🚧 ~90% — r450 reserved slice-header bits pinned + first-MB grammar characterized (no-neighbour predictor 128; I-frame coefficient tables extractor-asked, blocks 300/300); SVQ1 COMPLETE + 9-target fuzz-hardened (2 wire-reachable arithmetic overflow fixes); SVQ3 intra decode wire-driven (thirdpel binding docs-asked) — byte-exact I/P decode (all 16 wire VLC tables + whole-frame intra + 6-frame P chains + 160×120 overhang decode-and-discard + registered Yuv420P output; 5 fixture sets vs black-box reference) + SVQ3 transform/dequant/intra/interp primitives + chroma DC full-dequant pipeline + SVQ3 intra-4×4 prediction-mode VLC decode end-to-end (slice bits → reconstructed luma pixels, intra-DC dequant) + SVQ3 4×4 coefficient scan-order arrays (normal + alt) + quantiser-driven scan selection + spec/01 Gap-5 Clip1 predicted+residual writeback (reconstruct_4x4) + intra predictor-selection macroblock loop (5 4×4 modes + 16×16 plane/DC + chroma DC, driven across the MB grid) + spec/01 per-block intra reconstruction composition (place→dequant·M·X·Mᵀ→Clip1, Gaps 1–5) + SVQ3 signed-Golomb MV-difference + inter-MB motion-header decode + MC reference path (SVQ3 full-pel fetch/sixths-split/thirdpel-block/inter-predictor + SVQ1 §6.5 half-pel sampler + L3 inter sub-block recon) + SVQ3 picture-plane assembly + whole-picture intra frame-walk (per-MB recon → blit → Yuv420P VideoFrame) + r446 intra access-unit walk (slice envelope + all three intra grammars + quantiser delta + chroma sections) + registry receive_frame → real cropped Yuv420P (SMI-wrapped extradata) + V2 multi-slice continuation — 299/300 MBs of a real 320×240 I-frame pixel-exact (first-MB grammar gap docs-asked); + r439 entropy/transform layer rebuilt from staged spec/03–06 + tables (universal-code VLCs — prior exp-Golomb reader desynced from code number 3 up — staged residual code books + corrected core basis + both secondary transforms + CBP decode + intra-16×16 typing); lacks the I-frame MB-type wire dispatch (docs trace asked + second-oracle 25-frame conformance (reference-window MV clamp pinned, #174 arbitrated; fixture 4MV claim refuted; genuine-4MV #197 census-pinned byte-exact, 348 INTER_4MV MBs vs independent oracle) | ✅ ~92% — full SVQ1 I/P/B encoder + per-frame rate control (λ bisection) + droppable-B cadence (every shape black-box byte-exact) |
| **Indeo 3** (IV31/IV32) | 🚧 ~80% — r451 real IV32 fixtures + settled seed grammar + fixture-arbitrated row-stream executor (10 404/12 480 frame-0 luma pixels byte-exact; lacks cell-sequencing banks, asked); headers + VQ codebooks + MV decode + cell decomposition + MC executor to output pixels (§7.2 fix-up + 4-mode cell copy) + §3.2 mode-byte jump-table dispatch + multi-frame DecodeSession/stateful Indeo3Decoder (INTRA-gate + NULL repeat-previous + bank ping-pong) + §5.5 4:1:0 chroma box-upsampler + spec/07 §5.5 full-res YUV producer (chroma 4×4 box-upsample over §5.7 assembly → assemble_yuv) + spec/07 §6 frame finalisation (saved frame_flags/frame_number + continuity check + return codes) + spec/06 static-table cell-reconstruction executor (mode-byte stream consumer + plane disposition classifier + VQ_NULL copy-cell pixel drive) + whole-plane/whole-frame VQ_NULL reconstruction executor → real strip pixels via §4.3 upshift (deferred VQ_DATA/INTER left black) + oxideav-core Decoder registry integration (IV31/IV32 tag-disambiguation probe + one-shot decode_video_frame); + r433 VQ_NULL prefix-code fix + stateful cross-cell escape-carry executor + arena-generic dyad unpacker + ~12k-case hostile suite (~82% structural); lacks arena values (block-format contradiction, docs ask) + IV31/IV32 fixture | — |
| **Indeo 2/4/5** | 🚧 ~60% — r451 IV50 inverse-Slant kernel + intra reconstruction: flat 240×180 frame FULLY checksum-verified with exact vendor pixels (lacks band_glob_quant dequant scale, asked); IV50 intra entropy→per-band coefficient work list + recovered spec/08 §7 checksum oracle byte-exact on both fixtures (vendor stores-but-never-verifies) + IV50 decode bootstrap (headers + entropy/transform primitives) + spec/07 MV/MC layer (packed MVs + half-pel fold + 4 MC kernels + ref-slot rotation) + full spec/08 output stage (bias/clamp + chroma box-upsample + 5-FOURCC dispatch + planar packing + §6.3/§8 finalisation + whole-frame assemble_frame) + vendored static tables (vlcEnd/synth/dequant-scale) + spec/03 tile/MB layers + spec/05 rv-table mechanism + spec/06 SWAR Slant primitives + whole-frame INTRA driver (first IV50 pixels via assemble_frame) + multi-frame session (INTER structural + MC + NULL) + IV50 registry-wired (`oxideav_core::Decoder` bridge); + fixture-arbitrated entropy layer: both real IV50 fixtures decode end-to-end, all 6 band payloads byte-exact (Kraft anomaly resolved, rv-table semantics cracked, 2 spec/03 errata) — ~55%; pixels gated on scan/dequant/Slant numerics + iv3 codebook banks; Indeo 4/5 also run sandboxed via `oxideav-vfw` | — |

</details>

<details>
<summary><strong>Image</strong> (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **PNG / APNG** | ✅ 100% | ✅ 100% |
| **GIF** | ✅ 100% | ✅ 100% |
| **WebP** (VP8 + VP8L) | ✅ 100% | ✅ 100% |
| **JPEG** (still) | ✅ ~95% — via MJPEG | ✅ ~90% — via MJPEG |
| **TIFF** (6.0) | ✅ ~99% — full JPEG-in-TIFF (12-bit SOF1, lossless SOF3, planar, §22 both layouts) + all baseline features; lacks Exif/GPS tag semantics (docs ask) | ✅ ~97% — chunky/planar/tiled all photometrics + CCITT uncompressed-mode emission + every subsampled-YCbCr layout; lacks JPEG write |
| **BMP** | ✅ ~97% — 1..32-bit + V4/V5 + OS/2 + RLE (delta-skip→index-0 fill) + BITFIELDS (full-width mask) + ICC profiles + 8-target ~19M-exec fuzz campaign (adversarial header-forge/ICC/mask/RLE suites, zero findings) | ✅ ~97% — top-down + palettes + V4-calibrated-RGB/V5/linked-ICC writers + Rgb565/Pal8 |
| **Netpbm** (PBM/PGM/PPM/PNM/PAM) | ✅ ~95% — all 8 magics at 1/8/16-bit + 6 PAM TUPLTYPEs + fast paths (~45-50 GiB/s) + recode fixed-point fuzz + zero-dimension decode/encode asymmetry fix | ✅ ~95% — incl. P7 GRAYSCALE_ALPHA 16-bit + 16-bit ASCII encode |
| **ICO / CUR / ANI** | ✅ ~98% — multi-res + BMP/PNG sub-images + hotspots + ANI playback + per-step frame/hotspot accessors + directory-level `select_*_raw` best-fit + strict validation (incl. anih.bfAttributes reserved-bit reject + anih.cbSize on read/write) + framework Muxer/Demuxer + ANI fuzz target + duplicate-chunk hardening | ✅ ~98% — full 1/4/8/24/32-bpp indexed+direct DIB write (RGBQUAD palette + AND mask + exact-colour quantise) + mixed-depth multi-resolution files + ICO/CUR + symmetric ANI/ACON `write_ani_raw` encoder |
| **JPEG 2000** | ✅ ~98% — Part 1 + HTJ2K MIXED set decode | ✅ ~98% — full Part 1 + HT encode, PCRD layers + PSNR floor, all A.19 styles, PLT/TLM/COM, JP2/JPH writer (40+ shapes black-box exact); lacks PLM |
| **JPEG XL** | ✅ ~99% — multi-pass/multi-preset decode landed (2024 §I.3.1 supersedes FDIS §C.7.1 — erratum #9) + Annex A JPEG recon; fuzzed (8 daily targets, 9 hostile-input fixes) | — retired |
| **JPEG XS** | ✅ 100% — **full ISO/IEC 21122-4 decoder conformance: 65/65 codestreams bit-exact, all 9 Annex-C ETS profiles PASS (incl. both 4444 Nc=4 sets)** — Fs=1 tail-sign fix (Table C.9 NOTE 2) + NL,y≥1 picture-level cascade DWT + Nc 1..=8 + Part-1 conformance/CAP/profile gates + JXS still-image file format (21122-3 Annex A boxes); lacks B>16 (u32 planes; no published vector) | ✅ 100% (B≤16) — Nc 1..=8 + RCT/Star-Tetrix + NLT + per-precinct rate-budget pickers + Annex H content-adaptive WGT weights (4:4:4 RCT + subsampled 4:2:2/4:2:0 H.4–H.11 incl. CFA Star-Tetrix Cpih=3/Sd=1) + Annex E.3 Fq fractional scaling (Bw=20/Fq=8 high-precision lossy) + Table A.8 (Bw,Fq) conformance fix + Table A.12 Rm=1 run mode (both A.12 modes emit + decode, high-bd + subsampled + multi-group) + verified Ppih/Plev/Lcod signalling (all 8 Part-2:2019 profiles emit+claim, declarations decoder-gated) + exact-size CBR on both bit-depth paths + jxpl mirror-consistency + 15 hash-pinned feature-axis streams + r438 CBR×profile one-call composition + 21122-3 media-type registrations + cargo-fuzz (2 encoder bugs fixed); lacks B>16 (no published vector) |
| **AVIF** | 🚧 ~97% — end-to-end HEIF→AV1 decode (grid / alpha / rotation / crop) + §8.11.3 item byte resolution (construction_method 0/1/2 file/idat/item-offset; §8.11.3.3 item_offset follows the 'iloc' iref) across primary / grid-tiles / alpha / metadata) + iovl/iden/tmap/sato/grid derived-image geometry resolution (HEIF §6.3/§6.6.2/§6.6.2.3 overlay-canvas clipping + iden crop-of-original + tmap base-derivation + sato sample-transform + grid tile-placement) + §6.5.4..§6.5.40 item-property surface (incl. tols essential descriptor + §6.5.40 cmin camera-intrinsics) + §8.16.5 prft producer-reference-time + §6.5.36 amve ambient-viewing-environment + gain maps (tmap ISO 21496-1 §6 parse + apply) + unified derivation-graph resolution (nested/diamond dimg walk → decode set) + cm=2 derived-descriptor resolution + §11.2 region items (all 7 RegionGeometry variants + mskC) + §6.10 text/font items (txlo/elng/fnch) + §6.4.7-9 coded-item dependency roles (pred/base/exbl/tbas) + §11.3 derived region items + profile audits; pixel fidelity tracks oxideav-av1 intra | 🚧 container mux — AVIF muxer COMPLETE (ftyp/meta tree hdlr/pitm/iinf/iref/iprp(ipco+ipma)/iloc + item-properties av1C/ispe/pixi/colr/pasp/clap/irot/imir + alpha & depth aux + grid derivation + Exif/XMP + HDR mdcv/clli/amve + MA1B/MA1A profiles; AV1 bitstream black-box) + Encoder trait wired; + r441 pixel→AVIF encode LIVE: 8/10/12-bit × 4:2:0/4:2:2/4:4:4/mono + RGB(A) identity + alpha aux (§4.1 shalls) + clap-padded extents + grid encode; lossless round-trips sample-exact, external black-box tool accepts pixel-exact; registry Encoder live + r444 HBD decode composition (8/10/12-bit grid/alpha/clap/irot/imir + gain-maps at depth, black-box exact BOTH directions); lacks grid-alpha (docs-gapped) + iovl/iden pixel composition |
| **DDS** | ✅ ~99% — header + DXT10 + BC1-7 + BC6H all modes + ASTC LDR decode (Khronos DFS ch.23, all 4×4–12×12 footprints + void-extent + multi-partition + dual-plane, DXGI 133–187) + cubemaps/arrays/volumes + 16-bit/float + packed R11G11B10_FLOAT + R9G9B9E5_SHAREDEXP + R10G10B10A2_UINT + sub-sampled packed R8G8_B8G8/G8R8_G8B8 + 8/16/32-bit plain-integer UINT/SINT HDR uncompressed surfaces + normalised 8/16-bit UNORM/SNORM 1-/2-channel surfaces (normal/height maps) + legacy X8B8G8R8/X1R5G5B5/X4R4G4B4/L16/A4L4 mask layouts + YUV video formats (AYUV/Y410/Y416/YUY2/Y210/Y216/NV12/P010/P016/420_OPAQUE/NV11 → interleaved YUVA) + depth/depth-stencil decode (D16/D32/D24S8/D32S8 + R24G8/R32G8X24 typeless) + legacy G16R16/A2R10G10B10/A8R3G3B2/RGBG/GRGB/UYVY mask+FourCC layouts + signed BC4/BC5 i8 decoders + A4B4G4R4_UNORM (DXGI 191) + R10G10B10_XR_BIAS_A2 (DXGI 89) + encoder dangling-index OOB→InvalidData guard + daily fuzz | ✅ ~99% — uncompressed (2D/DX10 + cubemap/array) + BC1-7 + BC6H + BC-volume (3D) + ASTC LDR encode (single/two/three-subset + dual-plane, all 14 footprints) + mip chains + cubemap/array |
| **OpenEXR** | ✅ ~99% — scanline + tiled + deep + multi-part, all codecs; framework path emits true-HDR F32 (RGBA/RGB/Gray, HALF widened, FLOAT bit-exact); lacks RY/BY + layered-name frame mapping | ✅ ~97% — scanline/tiled/deep/multi-part write; registry encoder accepts the F32 family (HALF|FLOAT, 10 codecs) |
| **Farbfeld** | ✅ 100% | ✅ 100% |
| **HDR** (Radiance RGBE) | ✅ ~99% — new/old RLE + all axis flags + header metadata + derived colorimetry + scene-referred physical luminance (EXPOSURE/COLORCORR recovery) + fuzz + Criterion suite w/ ranked hotspots + bit-exact RGBE-quad round-trip surface (`from`/`to_rgbe_quads`) + 8×8×4 resolution/orientation/mode property matrix + D₄ geometric reorientation (HdrImage::reorient across the 8-orientation matrix, wire-verified) + scene-referred RGB radiance recovery (buffer + in-place EXPOSURE/COLORCORR undo) + XYZE photometric fix (Y verbatim; was 179× overstated) + file-faithful RGBE↔XYZE converters + stop-exact exposure / wire-quad exponent shift | ✅ ~98% — RLE modes + XYZE↔RGB + 8 tonemap ops + RleMode::Smallest per-scanline adaptive + GAMMA= transfer-exponent linearisation (applied on decode; inverse on encode) |
| **QOI** | ✅ 100% | ✅ 100% |
| **TGA** | ✅ 100% | ✅ 100% |
| **ICER** (JPL) | ✅ ~98% — ICER + ICER-3D with §V.C auto segment election; +9% scan throughput; interop container tail blocked on rulings | ✅ ~95% — §V.C closed, digest-pinned wire; mid-packet truncation blocked on the unstaged [13] tables |
| **WBMP** | ✅ 100% | ✅ 100% |
| **PCX** (ZSoft) | ✅ 100% | ✅ 100% |
| **ILBM** (Amiga IFF) | ✅ ~96% — PCHG/CAMG reader policies + HAM6 inference; TVDC transport + DEEP Huffman docs-gapped | ✅ ~92% — all 8 ANIM ops muxer-selectable with timed encoders |
| **PICT** (Apple QuickDraw) | ✅ ~99% — v1 + v2 opcode walkers + rasteriser + indexed PixMap + picture comments + CopyBits/PnMode transfer modes + DirectBits packType 0→§A-3 default packing + QuickDraw text rasterisation (built-in clean-room ASCII face + TxRatio h/v anisotropic glyph scaling + lineJustify intercharacter spacing) + QuickDraw Region rendering (panic-safe right-border-run inversion decoder + FrameRgn/FrameOval/FrameRoundRect/FrameArc + Line family honour pen size/pattern/mode + value-keyed indexed ColorTable, book 3-13) + QuickTime payload capture + fuzz-hardened (raster-budget/overflow guards) + 6 conformance bug fixes; + txFace style synthesis rasterised (bold/italic/underline/outline/shadow/condense/extend, Vol I) + grayishTextOr dimmed-text mode (Vol VI) + text-rasteriser DoS-hardened; lacks CompressedQuickTime $8200 payload (needs Inside Macintosh: QuickTime) | ✅ ~98% — every decodable construct emittable (`PictBuilder` v2 + `PictV1Builder` v1 + text/region/QT emitters), Apple-renderer black-box validated + r435 $8200/$8201 QuickTime payloads typed both directions (FourCC routed via the core resolver, $8201 subopcodes blit, QuickTime-only PICTs decode); lacks a real-world QT-emitted fixture |
| **SVG** | ✅ ~99% — full SVG 1.1 + SVG 2 feature grid (shapes / text / gradients / masks / markers / SMIL / CSS3 selectors + media queries) + all 16 §15 filter primitives rendered + feDropShadow & feComposite (over/arithmetic) + feMerge/feGaussianBlur(edgeModes)/feOffset/feComponentTransfer(§9.7)/feMorphology(§9.17 erode/dilate)/feConvolveMatrix(§9.9)/feDisplacementMap(§9.11)/feTile(§9.20)/feTurbulence(§9.21 Perlin) + feDiffuseLighting(§18)/feSpecularLighting(§19) Sobel-normal lighting pixel evaluation + top-level filter-graph DAG evaluator (in/result chaining + §9.4 subregion clip) + hostile-input hardening (XML-nesting/`<use>`-bomb/`.svgz`-decompression guards + parser fuzz; 2 CSS panics + deep-nest SIGABRT fixed) | ✅ ~97% — r449 read≡write parity push (native shape identity — rect/circle/… no longer flattened to paths — + verbatim `<text>`/tspan/textPath + SMIL parent re-attachment + inline display:none subtrees + a 17/17-doc byte fixed-point conformance gate that flushed 5 writer defects) + round-trips full shape graph + use/defs/symbol + switch + filter/clipPath/mask/marker reference-identity + PreservedExtras + §10.9.2 dominant-baseline + nested-`<svg>` viewport establishment + SVG2 `<symbol>` x/y/refX/refY + preserveAspectRatio `defer`; lacks stylesheet-driven display:none + paint-order-split shape identity |
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
| **IFC** (BIM, ISO 16739) | 🟢 Phase 3–4 ~90% — STEP/P21 + EXPRESS typing + tessellation: profiles, tapered + directrix sweeps, curved advanced Breps (cyl/sphere/torus/B-spline/revolved/extruded faces, watertight), sectioned spines, bounded surfaces, WHERE rules, georeferencing, fuzzed; lacks p-curve-bounded surfaces + non-convex mesh–mesh booleans | — |
| **Alembic** | 🚧 ~0% — Ogawa wire format docs-gapped per `docs/3d/alembic/GAP-TRACKER.md` | — |

Cross-format integration: `oxideav-cli-convert` exposes a 3D conversion path through `oxideav_meta::populate_mesh3d_registry` — `oxideav convert in.obj out.gltf` (or `--probe` for structural inspection). `crates/oxideav-tests/tests/mesh3d_*.rs` runs the cross-format roundtrip suite. The convert verb carries an ImageMagick-compatible op set (`-resize` / `-thumbnail` / `-extent` / `-monochrome` / `-roll` / `-define` …) plus a 3D→raster renderer (Gouraud + Phong, `-light` / `-camera` / `-projection` / `-fov`, debug render modes, `-aa N`). Black-box oracles cross-validate against Apple `usdzconvert` + Blender + assimp.

</details>

<details>
<summary><strong>Trackers</strong> (decode-only by design) (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **MOD / STM / XM / IT** | ✅ ~97% MOD · ~92% STM · ~93% XM · ~90% IT (new r455) — r451 XM ref-lockstep 240/240 (FT2 retrig/vol-col/Lxy-envelope ordering fixed, 2 fuzz bombs closed, multi-sample keymaps; STM effect semantics docs-blocked); shared Paula/FT2 mixer + full effect sets + Ultimate SoundTracker 15-sample + Startrekker FLT8 layouts + STM E4x/E7x waveform control + XM fine-slide last-non-zero memory (E1/E2/EA/EB/X1/X2) + note-delay LFO/counter-reset consistency + Kxy=note-97 silence + E6x pattern-loop point reset on pattern transition + typed sample-header accessors + ED0 immediate-trigger fix + n_patterns 0xFF-overflow hardening + hostile-input fuzz + r439 order-flow conformance (Bxx erratum + §5.14 overflow wrap + E6x row-only rewind + OOB-jump diagnostics) + r447 +951 restart-byte wrap semantics (filler-aware) + UST loop-area playback rule + EEx×order-flow pins — order-flow saturated + whole-order-table pattern-scan loader fix + SoundTracker 2.6/IceTracker (MTN\0/IT10) end-to-end | — |
| **STM** (Scream Tracker v1) | ✅ ~85% — structural parse + shared-mixer playback; XM-parity effects (Gxy/Jxy/Bxy/Cxy/Exy/Hxy + 7xy tremolo + volume-slide variants); hard-pan LRRL | — |
| **XM** (FastTracker 2) | ✅ ~90% — structural parse + full playback; envelopes + fadeout + key-off; vibrato + tone porta + pattern jumps + fine/extra-fine porta + Exy/Kxy subcommands + volume-column slides | — |
| **IT** (Impulse Tracker) | ✅ ~90% — decode + full player: NNA voices, envelopes, Axx…Zxx + Sxx, 25/25 black-box oracle gates, fuzzed; lacks 2.14 compressed samples (docs), filters | — |
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
| **`oxideav-videotoolbox`** | macOS / iOS | 🚧 H.264 + HEVC + ProRes + MJPEG + MPEG-2 + VP9 + MPEG-4 Pt 2 + AV1 (M3+) + VVC | 🚧 H.264 + HEVC + ProRes + MJPEG | Encoder knobs map onto VT session properties (bit rate / quality / profile / data-rate limits); PSNR_Y ~36-61 dB per codec. iOS links the frameworks via build.rs + `dlsym(RTLD_DEFAULT)`; macOS keeps the `dlopen` path; device-specific encoder gaps degrade gracefully via `kVTPropertyNotSupportedErr`; r401 fixed 4 latent FFI bugs (callback ABIs, decoded-frame PTS recovery, session + per-frame leaks) + OSStatus taxonomy + hardware require/enable/disable knob. |
| **`oxideav-audiotoolbox`** | macOS | 🚧 AAC LC + HE-AAC v1/v2 + AAC-LD/ELD + ALAC + iLBC + AMR-NB + AMR-WB + MP3 + FLAC + Opus | 🚧 AAC LC + HE-AAC v1/v2 + AAC-LD/ELD + ALAC + iLBC + FLAC + Opus | MP3 decode bit-exact ≈89.8 dB SNR; FLAC bit-exact 188 416/192 000 i16 @ 48k/2ch; ALAC S32 lossless contract (S16/S32 input, 24-bit output); Opus via `kAudioFormatOpus` (RFC 7845 OpusHead family 0/1/255 + RFC 6716 frame-duration mapping; ~26 dB SNR roundtrip); MP1 + MP2 decode added (sample-exact); typed OSStatus taxonomy + RAII converter + OS-inventory-gated registration. |
| **`oxideav-vaapi`** | Linux (Intel iGPU + AMD Radeon, via libva) | 🚧 H.264 | — stub | Codec id → VAProfile family map; `EntrypointMatrix` snapshot collapses per-device VLD/Enc capability probe FFI ~2×. Planned: HEVC + VP9 + AV1. |
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
   factory returns `Err`, registry retries the next-priority
   impl.

Pipelines that **require** hardware (real-time low-latency
capture where SW can't keep up) opt out of the SW fallback by
setting `CodecPreferences { require_hardware: true, .. }` — the
registry then surfaces the OS-level error instead of degrading
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
| **`oxideav-generator`** | Synthetic media source (`generate://...` URIs) + zero-input filters | ✅ audio synth (sine incl. phase/per-channel phase + chirp/FM/DTMF/multitone/ADSR/ringmod + 5-colour noise + `pwm` + `supersaw` + `tremolo` + dc/impulse trains) + image (xc/gradient/pattern/fractal/plasma/noise/label + Perlin-2001 + Worley + 1–8-bit quantised `ramp`) + video (testsrc/smptebars/fractal_zoom/gradient_animate/zoneplate/`scroll` + `movingbox` exact-MV motion probe + `snow` seeded stateless noise) + catalogue-wide byte-determinism suite + framefill benches |
| **`oxideav-rtmp`** | RTMP ingest + push | ✅ Server + client; AMF0/AMF3 parser/builder; Enhanced-RTMP v1 video + v2 audio + ModEx; pluggable key-verification; `rtmp://` PacketSource; symmetric teardown + client `poll_event` + v2 `MultichannelConfig` (24 SMPTE 22.2 positions) + Multitrack body + §E FLV file writer + `FlvReader<R: Write>` + NetConnection capability negotiation + §7.1.6 Aggregate Message routed end-to-end (`send_aggregate` + `next_packet` + `poll_event`) + ModEx TimestampOffsetNano (ns timebase) + typed `MessageStreamKind` accessor + §5 protocol-control invariant validator + §5.3 Acknowledgement received-byte window + Enhanced-RTMP v2 ReconnectRequest (typed client event + tcUrl resolution) + AMF3 §3.12 externalizable-object decode via `register_externalizable` per-class handlers + typed Enhanced-RTMP VideoFrameType.Command (StartSeek/EndSeek) seek-command frames + Enhanced-RTMP v2 audio silence-message + VideoPacketType.MPEG2TSSequenceStart (av01 descriptor) + SequenceEnd typed on both pipelines + AMF0 complete serializable marker set (§2.15 Unsupported 0x0D / §2.17 XML Document 0x0F / §2.18 Typed Object 0x10 + avmplus 0x11 AMF3 bridge) + play/subscribe direction complete (§4.2.1 PlaySession server + RtmpPlayer pull client + publish→play relay + `rtmp-play://` PacketSource + dynamic playlists/play2 + drain-until-FIN teardown fix) + chunk ext-timestamp/fmt-3 wire-correctness fixes (3 writer desync bugs) + RFC-1982 rollover unwrapping + AMF3 type-17 command path + v2 selector framing + §7.2.1.2 call RPC all surfaces + §5.4.5 peer-bandwidth limit types + Shared Objects (all 11 event types, all surfaces) + @setDataFrame/@clearDataFrame store+replay + HMAC-SHA256 digest handshake (auto-negotiated, dependency-free); + r441 Enhanced-RTMP v2 MULTITRACK end-to-end (per-track demux/mux with spec-invariant validation + typed TrackInfo + tag/multitrack send surfaces on client AND PlaySession + per-track PacketSource streams); lacks RTMPS + RTMPE |
| **`oxideav-sysaudio`** | Native audio output | ✅ Runtime-loaded backends (ALSA, PulseAudio, WASAPI, CoreAudio, OSS); CoreAudio + WASAPI real HAL latency; output-device enumeration; per-device routing API on all backends; `StreamRequest::buffer_frames` honoured; `Driver::preferred_format` introspection on WASAPI/CoreAudio/ALSA; functional OSS `/dev/dsp` via dlopen'd libc (S16_LE negotiation). BT-aware; falls back to software estimate if HAL unavailable; r401 added a CI mock backend, request pre-flight validation, software volume, `Driver::status()` tri-state, callback-panic containment, and fixed CoreAudio HAL enumeration (was empty on every host). |
| **`oxideav-pipeline`** | Pipeline composition (source → transforms → sink) | ✅ JSON transcode-graph executor; pipelined multithreaded runtime drives byte/packet/frame sources natively (spawn/seek/progress/abort on typed sources) + graph-validation hardening (alias-cycle guard, same-kind `all:` fan-out ordinals, key-directed parse — exponential nested-chain fixed) + error-propagation contracts + channel caps + byte ceilings + Progress counters + graph benches + r445 multi-output jobs run in ExecutionContext-clamped parallel waves (document-order error precedence) + seeks fan out to every routed source |
| **`oxideav-scene`** | Time-based scene / composition model | 🚧 data model for PDF pages / RTMP streaming compositor / NLE timelines + per-frame `Sample` + animation-track composition + `RasterRenderer` (bg solid/gradient + Rect/Polygon + `ObjectKind::Vector`) + `ObjectKind::Group` nested + SVG 1.1 path-data (M/L/H/V/C/S/Q/T/Z + relative + A arc) + `ObjectKind::Image(Decoded)` RGBA8 + `Background::DecodedImage(Arc<VideoFrame>)` + audio-cue mixing into `RenderedFrame.audio` + typed PBR metallic-roughness `Material` + `Scene::materials` palette + glTF 2.0 node graph COMPLETE (typed validation + cycle-safe traversal/walk utilities + Mat4 inverse/TRS-decompose + quaternion ops/slerp + keyframe node animation Step/Linear/CubicSpline + Scene-level graph/animation fields) |
| **`oxideav-render`** | Scene3D → pixels rendering backends | 🚧 scanline rasteriser + Whitted raycast (all six shading modes, shadows/reflection/refraction, BVH walk, row-parallel −88%, cross-backend parity ±1/255) + shared camera/math/shade core + criterion benches; PathTrace pending |
| **`oxideav-bitstream`** | Codec-header parse/write toolbox | ✅ H.264 + HEVC parameter sets complete (SPS/PPS/VPS incl. VUI/HRD + SEI families + scaling lists + RPS derivation; H.264 + HEVC byte-exact parameter-set writers on lossless parses) + AV1 metadata OBUs + H.266 SPS/PPS/VPS/PH + APS (ALF/LMCS/scaling-list) + full Annex-D SEI set + OPI/DCI, all with byte-exact writers + H.266 complete VPS/SPS/PPS walks + byte-exact writers (r438) + HEVC typed SEI encoders (HRD-coupled BP/PT both ways) + AV1 sequence-header + VP9 keyframe writers (VP9 4-bit misparse found+fixed) + Annex-B ↔ length-prefixed framing — fuzz-hardened; lacks H.266 slice-header walk |
| **`oxideav-audio-filter`** | Audio effects & conversions (streaming) | ✅ ~50 filters: classic + transient/spatial/restoration family + SlewLimiter + LR4 crossover + `true_peak_detector` + `state_variable` Chamberlin SVF + Criterion benchmark harness (7 scenarios) + `crest_factor_meter` + `stereo_correlation_meter` (Pearson coefficient, sliding-window) + `zero_crossing_rate` observer (per-channel sliding-window meter, `sign(0.0) = +1` defends against `f32::signum -0.0` phantom-crossing) + `dither` (TPDF/RPDF requantizer + error-feedback noise shaping) + complete staged EQ-cookbook biquad catalogue (constant-peak BPF + slope shelves) + parallel/New-York compressor (dry/wet blend) + band-limited rational resampler (ratio-scaled anti-alias prototype, ≥40 dB end-to-end alias rejection) + `crossfeed` (headphone ITD + head-shadow) + chunk-size-invariance / hostile-parameter / denormal-flush / analytic-transfer-function contracts + `latency_samples` reporting (pitch_shift NaN-hang + wah/talkbox state-leak fixes) — see crate README for the catalogue |
| **`oxideav-image-filter`** | Single-frame image effects (stateless) | ✅ 136 filter types / 196 factory names — `VoronoiTransform`/`ProximityFill` (exact nearest-feature) + `SignedDistanceField` (exact signed Euclidean DT) + Gabor + Niblack adaptive local-statistics threshold + `CurveInterpolation::NaturalCubic` + `CentripetalCatmullRom` + `ReinhardExtended` + Drago §4 adaptive-log tone-map (Ld_max cd/m² + exposure-independent log-average pre-scaling) + exact-Euclidean morphology (dilate/erode/open/close/outline) + 7 resize kernels (Lanczos windowed-sinc + Mitchell-Netravali + B-spline anti-alias separable driver) — see crate README for the catalogue |
| **`oxideav-pixfmt`** | Pixel-format conversion + palette + dither | ✅ 70 formats, 4830/4830 pairs closed (1478 direct) — YUV↔RGB matrices (BT.601/709/2020/2100), chroma subsampling incl. 4:4:0, packed 4:2:2, palette + dither, scene-referred F32 family (linear-light, never clamps float↔float); lacks Yuv410P/YuvJ440P core variants |

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
