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
  Timestamp / PixelFormat / ExecutionContext + **DoS framework: `DecoderLimits`
  caps, `arena::ArenaPool` (Rc-based, single-threaded) + `arena::sync::ArenaPool`
  (Arc-based, Send + Sync) refcounted bump-allocator pools, refcounted `Frame`
  whose drop returns the buffer to the pool, `Decoder::receive_arena_frame()`
  trait method with default impl that wraps `receive_frame()` for true zero-copy
  per-decoder opt-in (h261, h263, vp6 ports done)** — Decoder / Encoder /
  Demuxer / Muxer traits + their registries also live here, in
  `oxideav_core::registry::*`), `oxideav-pipeline` (source → transforms → sink
  composition).
- **I/O** — `oxideav-source` (generic SourceRegistry + file driver +
  BufferedSource; openers register as **bytes / packets / frames** and
  `SourceRegistry::open` returns the matching `SourceOutput::{Bytes,
  Packets, Frames}` variant so the executor can branch per shape),
  `oxideav-http` (HTTP/HTTPS bytes driver, opt-in via feature),
  `oxideav-rtmp` (`rtmp://` packet driver — registers via
  `oxideav_rtmp::register(&mut sources)`, default-on in `oxideav-cli`).
- **Effects + conversions** — `oxideav-audio-filter` (Volume / NoiseGate /
  Echo / Resample / Spectrogram), `oxideav-image-filter` (stateless
  single-frame Blur / Edge / Resize), `oxideav-pixfmt` (pixel-format
  conversion matrix + palette generation + dither).
- **Containers** — one crate each for `oxideav-ogg` / `-mkv` / `-mp4` /
  `-avi` / `-iff`. Simple containers (WAV, raw PCM, slin) live inside
  `oxideav-basic`.
- **Codec crates** — one crate per codec family; see the
  [Codecs table](#codecs) below for the per-codec status. Tracker formats
  (`oxideav-mod`, `oxideav-s3m`) are decoder-only by design.
  Recent sibling crates: `oxideav-evc` (MPEG-5 EVC, ISO/IEC 23094-1),
  `oxideav-jpegxs` (JPEG XS, ISO/IEC 21122), `oxideav-midi` (Standard
  MIDI File + soft-synth), `oxideav-pbm` (Netpbm: PBM/PGM/PPM/PNM/PAM),
  `oxideav-nsf` (NES Sound Format — 6502 emu + 2A03 APU); image-format
  bootstrap wave: `oxideav-dds`, `oxideav-openexr`, `oxideav-farbfeld`,
  `oxideav-hdr` (Radiance RGBE), `oxideav-qoi`, `oxideav-tga`,
  `oxideav-icer` (JPL Mars-rover), `oxideav-wbmp`, `oxideav-pcx`,
  `oxideav-pict` (Apple QuickDraw); `oxideav-iff` extended with ILBM.
  AVIF still register-but-refuses while gated on AV1 decoder completeness.
- **Vector graphics + text** — `oxideav-svg` (read+write SVG; rounds 1-3
  ship full shape set + text/filters/masks/clipPath + use/symbol + svgz +
  animate/set@t=0), `oxideav-pdf` (multi-page writer + Scene
  metadata via `/Info` dict; reader: bytes → Scene with xref +
  FlateDecode + content-stream operator parser + r35 inline-image
  extraction (ISO 32000-1 §8.9.7 BI/ID/EI framing)), `oxideav-raster`
  (vector→raster rendering kernel — scanline AA, bilinear/Lanczos2/Lanczos3 + Mitchell/Catmull-Rom/B-spline cubic image resampling,
  trapezoidal coverage, soft masks, patterns, filter primitives, ICC
  pipeline, bitmap cache keyed by `Group::cache_key`), `oxideav-ttf`
  (TrueType parser — cmap 0/4/6/12/14 incl. Variation Sequences, GSUB
  ligatures, GPOS kerning, COLR + CPAL + sbix tables, TTC subfont
  selection, AGL glyph-name→Unicode, full `name`-table accessor API), `oxideav-otf` (CFF / Type 2 charstrings incl. CID-keyed ROS/FDArray/FDSelect + arithmetic/stack/storage/conditional ops + Top-DICT FontMatrix/PaintType/CharstringType/StrokeWidth, ISOAdobe/Expert/ExpertSubset predefined charsets, cubic outlines; r222 GDEF + Coverage + ClassDef common-layout primitives + `GlyphClass` enum),
  `oxideav-scribe` (shaper with vector-first `Shaper::shape_to_paths`
  API — no rasterizer dep; trapezoidal horizontal AA, GPOS mark-to-mark,
  COLR/CBDT colour glyphs via raster bilinear/composer; bidi UAX #9 +
  USE still future work).
- **3D scenes & assets** — typed `oxideav-mesh3d` (Scene3D / Mesh /
  Material PBR / Skin / Animation / Camera / Light / AudioEmitter +
  area-weighted vertex-normal recompute + MikkTSpace-style tangent-space basis (Lengyel 2001) +
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
  model is in place, renderer is a scaffold.

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
| WAV       | ✅ | ✅ | ✅ | LIST/INFO + BWF `bext` (EBU 3285) + smpl/inst/plst Playlist + r193 `fact` chunk per RIFF MCI §3 + r205 `iXML` chunk + r210 §3 `CSET` Character-Set chunk + r221 **RIFF MCI §3 LIST INFO baseline full 23 sub-IDs** (adds 12: `archival_location`/`commissioned`/`cropped`/`dimensions`/`dpi`/`keywords`/`lightness`/`medium`/`palette_setting`/`sharpness`/`source`/`source_form`; pre-r221 aliases preserved; unknown sub-IDs still skipped silently per §3 forward-compat; 64 tests) |
| FLAC      | ✅ | ✅ | ✅ | VORBIS_COMMENT, streaminfo, PICTURE block; SEEKTABLE-based seek; CUESHEET round-trip (read + write per RFC 9639 §8.7); r182 in-place symmetric-pair Levinson-Durbin update + r212 **streaming `Crc8`/`Crc16` validators** (`update_byte(b)` / `update(&[u8])` / `value()` for chunk-fed decoders — Ogg-FLAC demuxers, network-fed, early-fail pre-allocation; buffer-shaped free fns become thin wrappers; `decode_one_frame` rewired through `Crc16`; 100 lib tests) |
| Ogg       | ✅ | ✅ | ✅ | Vorbis/Opus/Theora/Speex pages + comments; page-granule bisection + page-level seek index + chained-link-aware duration + page-loss/hole detection + page-sync recapture + public CRC-32 validation API + r172 Criterion bench harness + r183 streaming CRC + r185 Skeleton 3.0/4.0 + r192 slice-by-4 CRC-32 + branch-free `compute_page_checksum` (3-segment dispatch drops 65535 branches from max-size page; **page/parse/max 493 MiB/s → 1.3 GiB/s = 2.5-3× over r172**) + r196 Skeleton 4.0 `index` packet index-accelerated `seek_to` + r215 Skeleton 4.0 index-validity gating in fast-path `seek_to` + r221 **Skeleton 4.0 multi-stream keyframe-index minimisation in `seek_to`** (per spec "Keyframe indexes for faster seeking" — picks smallest byte offset across every active stream's floor keypoint at target time; r215 fast path consulted only requested stream; `verify_keypoint_landing` runs against winning serial; single-stream byte-identical to r215; 84 tests) |
| Matroska  | ✅ | ✅ | ✅ | MKV/MKA/MKS; Cues seek; SeekHead/Chapters/Attachments/subtitles; opt-in block lacing on write; EBML CRC-32 validation + r186 per-Cluster CRC-32 validated on advance() + Cue-driven seek; typed Tag/TrackOperation/ContentEncodings/chapters() decode; typed Video FlagInterlaced/FieldOrder + geometry quartet + Colour master + SMPTE 2086 MasteringMetadata + StereoMode + r177 Projection + r183 AlphaMode/AspectRatioType/UncompressedFourCC typed decode + r196 §5.1.6 write-side Attachments + r202 §6.2 write-side CRC-32 on Top-Level masters + r208 §5.1.4.1.28 Video FlagInterlaced + FieldOrder write + r214 §5.1.4.1.28.3/§5.1.4.1.28.4 StereoMode + AlphaMode write + r219 **§5.1.4.1.28.8..14 Video geometry quartet write** (`MkvMuxer::set_video_geometry` emits `PixelCrop{Top,Bottom,Left,Right}` + `DisplayWidth`/`DisplayHeight`/`DisplayUnit` per RFC 9559; `MkvVideoGeometry::cropped`/`aspect_ratio` builders; `DisplayUnit::to_raw()` round-trips Table 10 incl. `Other(u64)`; queue-time gates post-header/non-video/zero-display/out-of-range; +15 tests) + r225 **§5.1.4.1.28.15 Video `UncompressedFourCC` write** (`MkvMuxer::set_video_uncompressed_fourcc(stream_index, [u8;4])` emits id `0x2EB524` binary length=4 verbatim inside the per-track `Video` master at `write_header` time after the `PixelCrop*`/`Display*` block; `[u8;4]` array type enforces the §5.1.4.1.28.15 schema-fixed length; queue-time gates post-header/out-of-range/non-video; omitting leaves the element off-disk → `MkvDemuxer::video_uncompressed_fourcc` surfaces `None`) |
| WebM      | ✅ | ✅ | ✅ | First-class: separate fourcc, codec whitelist (VP8/VP9/AV1/Vorbis/Opus); inherits Matroska Cues seek |
| MP4       | ✅ | ✅ | ✅ | mp4/ismv; faststart; iTunes ilst; fragmented demux+mux (DASH/HLS/CMAF) + sidx/mfra/tfra/styp; AC-3/E-AC-3/DTS sample entries; subtitle/timed-text; protected sample-entry unwrap; typed track refs + edts/elst mux + elng + kind + cslg + stsh + sdtp + sample-group sbgp/sgpd + §8.16.5 prft demux + r162 atom-walker robustness + r182 sidx-driven seek fast-path + r189 `read_box_header` largesize overflow reject + r196 ISO/IEC 23001-7 §8 CENC parser + r203 §8.7.8-9 `saiz`/`saio` Sample Auxiliary Information parser + r210 §8.3.4 `trgr` Track Group Box parsing + r216 §8.5.3 `stdp` Degradation Priority Box parsing + r221 **§8.8.2 `mehd` MovieExtendsHeaderBox parsing** (v0 32-bit + v1 64-bit `fragment_duration` widened to `u64`; takes precedence over `mvhd.duration` for `duration_micros` on sealed fragmented files; raw value surfaced on `metadata()` as `mehd_fragment_duration`; unknown-version/truncated mehd silently dropped → mvhd fallback; 289 tests) + r226 **§8.8.13 `leva` Level Assignment Box parser** (typed Level Assignment at `moov/mvex` scope per ISO/IEC 14496-12); lacks AES-CTR/CBC decryption driver |
| MOV (QuickTime) | ✅ | — | ✅ | Apple QTFF + ISO BMFF meta + HEIF/HEIC item-properties + grid/iovl/tmap + symmetric muxer + fragmented-MP4 seek + DASH sidx/styp + stbl + traf saiz/saio sample-aux + r182 ISO 14496-12 §4.2/§11.1 `uuid` User-Type Box parser + r187 largesize overflow reject + r199 §8.3.4 `trgr` Track Group Box + r204 §8.7.3.3 `stz2` Compact Sample Size Box + r210 **ISO 14496-12 §8.5.3 `stdp` Degradation Priority Box** (`SampleTable::stdp: Vec<u16>` sized from stsz/stz2 per §8.5.3.1 with no on-disk count; `SampleTable::sample_degradation_priority(idx)` + `MovDemuxer::sample_degradation_priority(track, sample)` typed accessors; first-wins on duplicate-box, non-zero `flags` rejected per §8.5.3.2 spec-fixes flags=0, body < 2·sample_count bytes rejected; 10 new fixture tests) + r216 QTFF pp.51-53 `imap` Track Input Map parser + r219 **ISO 14496-12 §8.16.4 `ssix` Subsegment Index Box** (`Ssix`/`SsixSubsegment`/`SsixRange` + `parse_ssix` + `subsegment_count`/`total_size_for`/`partial_subsegment_offset` accessors; `MovDemuxer::ssix_for_sidx` O(1) cross-reference honouring §8.16.4.1 "next box after associated sidx" rule; orphan/out-of-order `ssix` surfaces but binds to no `sidx`; +23 tests); ffprobe-accepted |
| AVI       | ✅ | ✅ | ✅ | AVI 1.0 + OpenDML 2.0 demux/mux; AVIX/dmlh/vprp + 2-field interlaced + VBR audio + LIST INFO + typed PaletteChange/TextChunk/AvihFlags/Idx1Flags + r197 OpenDML AVISUPERINDEX `bIndexSubType` surface (`super_index_sub_type` / `super_index_is_2field` / `avi:indx.<n>.sub_type_2field` metadata; AVI_INDEX_SUB_2FIELD == 0x01) + ODML keyframe seek + WAVEFORMATEXTENSIBLE + `strn`/`strd` + CBR-audio validator + dmlh.dwTotalFrames + IDIT/ISMP/rcFrame/wLanguage + dwInitialFrames + r163 typed `dwChannelMask`/`Speaker`/`ChannelLayout` + r182 typed `strh.wPriority` + r203 per-stream `strh.dwStart` + r210 **per-stream `strh.fccHandler` driver-handler FourCC** (`AviDemuxer::stream_handler(idx) -> Option<[u8; 4]>` byte-4 of the 56-byte AVISTREAMHEADER; `AviMuxOptions::with_stream_handler(idx, fourcc)` builder; all-zero → None; `avi:strh.<n>.handler` metadata; video defaults mirror `BITMAPINFOHEADER.biCompression`, audio defaults to all-zero; 13 round-trip tests) + r217 **per-stream `strh.dwSuggestedBufferSize`** (`stream_suggested_buffer_size(idx) -> Option<u32>` + `AviMuxOptions::with_stream_suggested_buffer_size(idx, n)` overrides auto-derived `t.max_chunk_size`; `0` "do not know" sentinel → None; spec-independent from `avih.dwSuggestedBufferSize`; +14 tests) + r222 **per-stream `strh.dwSampleSize` parse + emit** (`stream_sample_size(idx) -> Option<u32>` maps `0` "samples can vary" sentinel → `None`; `AviMuxOptions::with_stream_sample_size(idx, n)` writes the DWORD at offset 44; `avi:strh.<n>.sample_size` metadata; +12 tests) |
| Blu-ray (BD-ROM) | ✅ | — | — | `oxideav-bluray` Phase 2 — UDF 2.50 mount (ECMA-167 3rd ed.) + BDMV walk (`index.bdmv`/`MovieObject.bdmv`/`.mpls`/`.clpi`) + `.m2ts` stream (192→188-byte TP_extra_header strip) + `bluray://` URI handler with auto-detect; r93 typed `Cpi { ep_map: Vec<EpMap { stream_pid, ep_stream_type, entries: Vec<EpEntry { pts_ep_start, spn_ep_start, is_angle_change_point, … }> }> }` CPI EP_map decode per BD-ROM AV §5.7 (coarse + fine two-level table folded into a flat per-PID list a seeker can binary-search); r96 keyframe-aligned `TitleSource::seek_to(pts_90k)` (PTS→clip→I-frame→SPN×192, AACS-unit-aligned); `StreamDecryptor` trait hooks `oxideav-aacs` without hard dep. + r180 multi-angle PlayItem parsing (BD-ROM Part 3 §5.4.4.1) + `open_title_with_angle` / `max_angle` per-angle title open (AV §5.2.3.3) + r188 `Disc::chapters(title)` from PlayListMark entry marks + r200 `Disc::title_streams(title) -> TrackCatalogue` deduplicating per-PlayItem STN_table entries by `(PID, kind)` (AV §5.2.3.3 / Part 3 §5.4.4.4) + mount-time `TitleInfo::languages` from audio/subtitle entries + r211 **cross-PlayItem STC PTS continuity map** (`pts_continuity_segments()` / `map_clip_pts_to_title_pts(byte_pos, pes_pts)` — per-PlayItem `output_byte_*` / `title_pts_*` / `clip_in_pts_90k` / `stc_origin_pts_90k` lifted from CLPI `SequenceInfo::presentation_start_time` §5.5.4.2 picked by `stc_id_ref` §5.4.4.1 + `ConnectionCondition` §5.4.4.2; first PI normalised to NonSeamless; 139 tests) + r222 **mid-stream angle-change-point enumeration** (`TitleSource::angle_change_points()` / `next_angle_change_point(pts_90k)` aggregate `EpEntry::is_angle_change_point` per-PID via the existing EP_map; per-clip ascending PTS; first stepping-stone for AV §5.2.3.3 mid-stream angle switching). Lacks HDMV opcode exec, BD-J, mid-stream angle switching |
| DVD-Video | ✅ | — | — | `oxideav-dvd` Phase 3b — ISO 9660 + UDF 1.02 mount + VIDEO_TS walk + IFO body parser (VMGI/VTSI + TT_SRPT + VTS_PTT_SRPT + PGCI [+ PGC subpicture colour-LUT + pre/post/cell nav command table] + VTS_C_ADT + chapter materialiser) + VOB demux (MPEG-PS pack/PES + Nav-Pack PCI/DSI [+ PCI highlight + DSI typed sections] + DVD substream router for AC-3/DTS/LPCM/subpicture) + VOB → MKV mux (`mkv-output` feature; per-PES PTS preserved + ChapterAtom per `DvdChapter` via RFC 9559 §5.1.7) + `dvd://` URI handler + r172 typed NavInstruction VM disassembler (Phase 3c precursor: full Link family + 13-entry link-subset + Jump/Call SS + Set arithmetic + Type 4..6 classifier). + r179 Sub-Picture Unit (SPU) decoder (SPUH+DCSQT walker, 8 typed commands, 2-bit/four-form PXD RLE, 90 kHz STM-DTS conversion) + r188 SPU RGBA compositor (`composite()`: SET_COLOR/SET_CONTR → PGC palette LUT → BT.601 studio-swing YCbCr→RGB + top/bottom-field PXD interleave) + r200 Phase 3c VM execution (RegisterFile w/ SPRM defaults + RSM call/return stack + `step()/run_list()` honoring Goto/Break/Exit with step-budget; SET-arithmetic + 7 CmpOps + 12 SetOps) + r207 Phase 3c Type 4..6 compound SET+CMP+LNK + r213 Phase 3c SPRM defaults refinement + 6 bitfield-aware accessors + r218 **LPCM audio-pack header decoder** (`LpcmHeader` parses private_stream_1 LPCM 7-byte header — sub_stream_id / frame counters / emphasis / mute / `LpcmQuantisation` 16/20/24-bit / `LpcmSampleFrequency` 48/96 k / 1..=8 channels / X/Y dynamic-range gain; `bitrate_kbps` + `is_within_dvd_video_limit` (6144 kbps cap) + `peel_lpcm_payload` zero-copy split; Phase 3b `mkv_writer` strip step now drops the LPCM header so `pcm_s16be` receives clean samples; 208 lib tests) + r223 **User Operation flag decoder** (`uops` module — `UserOp` 25-variant enum w/ bit-number discriminants + `UopMask` u32 newtype w/ typed `contains` / `is_allowed` / `with` / `without` / `set` / `clear` / `merge_or` three-level OR-merge + `UopLevel::cover()` per-spec applicability matrix (TT_SRPT / PGC / VOBU) + typed accessors on `Pgc` / `PciPacket` / `DvdTitleEntry`; +21 lib tests). Lacks CSS auth (`oxideav-css`) |
| MP3       | ✅ | — | ✅ | demuxer LANDED (ID3v2/ID3v1 skip + Xing/Info VBR + CBR/VBR seek_to); r177 Decoder-trait stereo widening (independent + joint MS + intensity, planar AudioFrame) |
| IFF (EA IFF 85) | ✅ | ✅ | — | One crate for the whole `FORM/LIST/CAT` family — Amiga `8SVX` audio + `ILBM` images (1..8-plane indexed + 24-bit literal-RGB true-colour, EHB/HAM6/HAM8, ByteRun1, HasMask, GRAB, SHAM, PCHG; CRNG/CCRT/DRNG `cycle_step`) + `ANIM` (op-0 literal + op-5 vertical-delta encode/decode + r192 op-7 Short/Long Vertical Delta decode) + Apple `AIFF / AIFF-C` (FORM/COMM/SSND walker, 80-bit IEEE-extended sample-rate decode, NONE/twos/sowt/raw/fl32/FL32/fl64/FL64 PCM, codec-bearing FourCCs ima4/ulaw/alaw routed to sibling crates) + r198 §6.0 AIFF MARK chunk parsing + r203 §9 AIFF `INST` (Instrument) chunk parsing (`InstrumentChunk { baseNote / detune / low+highNote / low+highVelocity / gain / sustainLoop / releaseLoop }` + `PlayMode { NoLooping / ForwardLooping / ForwardBackwardLooping }` + `resolve_sustain_loop`/`resolve_release_loop` join against MARK with begin<end ordering guard; MIDI 0..=127 validation) + r209 **ANIM op-7 encode + AIFF COMT/AESD/APPL surfacing + MARK/INST write-side** (`anim::encode_op7_body`/`encode_anim_op7` greedy Skip/Same/Uniq per column + 64-byte pointer table + 8 op/data-lists; `Form::comments`/`aesd`/`applications` dup-rejecting accessors; `write_marker_chunk`/`write_instrument_chunk`/`write_comments_chunk`/`write_aesd_chunk`/`write_appl_chunk` complete the round-trip; +21 tests) + r215 AIFF §10.0 MIDI Data chunk + r220 **AIFF §13.0 text chunks** (typed `NAME`/`AUTH`/`(c) `/`ANNO` via `TextChunk`+`TextKind`; `Form::name`/`author`/`copyright` singletons + `annotations: Vec<TextChunk>` per-spec "any number"; `as_str` + `as_string_lossy`; canonical `(c) ` ckID only; `write_text_chunk`; 363 tests); lacks ANIM op-8 + DEEP/TVPP/RGB8/RGBN true-colour (#1368) + AIFF SAXEL §8.0 (draft) |
| IVF       | ✅ | — | — | VP8 elementary stream container |
| AMV       | ✅ | ✅ | — | Chinese MP4 player format (RIFF-like) — r191 clean-room demuxer + r197 `AmvMuxer` + r203 `seek_to` + r208 lazy `build_chunk_index` O(log N) cache + r213 trailer-recovery for truncated files + r218 **§2/§3 strict-mode sentinel validation** (`AmvDemuxer::open_strict()` + `AmvHeader::validate_sentinels()` cross-check §2 `dwMicroSecPerFrame == 1e6/fps` + `flag_one == 1` + `reserved_30 == 0` and §3 four all-zero `strh`/`strf` bodies, naming the file offset on any non-zero byte; permissive `open` path unchanged; real `comedian.amv` cross-checked; 73 lib tests) + r223 **§3b audio WAVEFORMATEX device-profile sentinels** (`AmvWaveFormat::validate_sentinels()` cross-checks the §3b six fixed-value constants — `wFormatTag==1`, `nChannels==1`, `nAvgBytesPerSec==nSamplesPerSec*2`, `nBlockAlign==2`, `wBitsPerSample==16`, `cbSize==0`; `nSamplesPerSec` free; +11 unit tests) |
| FLV       | ✅ | ✅ | — | Flash Video — MP3/AAC/H.264 audio + VP6f/VP6a/H.264 video + Enhanced RTMP ExVideoTagHeader + AMF0 onMetaData/onXMPData/onCuePoint + Annex F encryption + E-FLV ModEx walk + multitrack body splitter + HDR colorInfo metadata + r161 injection-robustness suite + 16 MB OOM-lever guard + r182 onMetaData catch-all preserves Date/Null/StrictArray/AMF3-nested + r186 unknown-script-name argument preservation + r196 first muxer slice (audio-only) + r202 §E.4.3 / §E.4.3.1 video-tag muxer slice (write_h263_tag + write_vp6_tag + write_vp6a_tag w/ AlphaOffset + write_avc_sequence_header + write_avc_nalu_tag w/ SI24 CompositionTime + write_avc_end_of_sequence + VideoTagHeader↔byte round-trips) + r209 Enhanced-RTMP ExVideo + ExAudio muxer slice + r215 `onMetaData.keyframes` seek-table writer + r221 **ExVideo/ExAudio ModEx prefix emission** (shared `mod_ex::emit::<7>` writer chains size + raw payload + `(subtype<<4 \| next_pt)` trailer; lead-byte low nibble `7`; resolved packet_type rides on final trailer; validates all parser invariants at emit time incl. TimestampOffsetNano UI24 BE + size 1..=65536; multitrack-bearing video rides through as `ExPacketType::Multitrack`; fixes latent size-encoder off-by-one at payload length 256; 317 tests); lacks ExAudio multitrack emit |
| WebP      | ✅ | ✅ | — | RIFF/WEBP (lossy + lossless + animation; ANIM + ANMF emit) + r210 **§4.4 `inverse_color_indexing` per-bundle hoist** (`width_bits ∈ {1,2,3}` bundled path rebuilt: row bases + `(x % count)·bits` field-selector hoisted out of x-loop; green byte loaded once per bundle, `count = 1<<width_bits` outputs walked with stepping shift; palette-2 40.7→31.6 µs = **−22.4 %**; correctness checked against verbatim pre-r210 reference across 4 bundling levels including trailing-partial-bundle / single-column / single-row / OOB-index; 526 tests) |
| TIFF      | ✅ | ✅ | — | TIFF 6.0 single-image + r177 BigTIFF write (magic 43 / 8-byte offsets / LONG8 strip+tile arrays) + r183 PhotometricInterpretation=8 1976 CIE L*a*b* decode + r185 CCITT T.4 2-D + T.6 (Group 4) fax decode (READ algorithm; tiffcp-oracle pixel-exact) + r206 §23 CIE L*a*b* encode + r221 **CCITT T.4 2-D (Modified READ) + T.6/MMR/Group 4 ENCODERS** (`TiffCompression::CcittT4TwoD{eol_byte_aligned}` + `CcittT6`; `encode_two_d_row` walks coding line emitting Pass/Horizontal(M+M)/V(0..±3) mode codes per T.4 §4.2.1.3; T4TwoD: row 0 1-D tagged, rest 2-D, T4Options bit 0 set in IFD; T6: every row 2-D against previous, no EOL framing, T6Options=0; 286 tests; closes r79 decoder-only asymmetry) |
| PNG / APNG| ✅ | ✅ | — | 8 + 16-bit, all color types, APNG animation + r188 gAMA/cHRM + r202 §4.2.10 zTXt + r219 **§4.2.9 tRNS round-trip** (`PngMetadata::trns` typed `Grayscale(u16)` ct=0 / `Rgb(u16,u16,u16)` ct=2 / `Palette(Vec<u8>)` ct=3; ct=4/6 rejected per spec; PLTE→tRNS→IDAT ordering enforced; samples bounded by `(1<<bit_depth)-1`; encoder-side duplicate-chunk reject; 412 tests); metadata lacks only iCCP/iTXt |
| GIF       | ✅ | ✅ | — | GIF87a/GIF89a, LZW, animation + NETSCAPE2.0 loop + multi-frame compositor (§23 disposal-method state machine, 4 modes) + r181 `GifImage::frames_with_palette` §21 active-table iterator + r188 §23 `has_transparency()` / `requires_user_input()` stream-level GCE flag queries — clean-room rebuilt from CompuServe spec |
| JPEG      | ✅ | ✅ | — | Still-image wrapper around the MJPEG codec |
| BMP       | ✅ | ✅ | — | Windows bitmap — DIB headers BITMAPINFOHEADER / V4 / V5, 1/4/8/16/24/32-bit + r182 BI_ALPHABITFIELDS (compression=6, V3 four-mask alpha variant); also exposes the DIB helpers used by ICO / CUR sub-images |
| Netpbm    | ✅ | ✅ | — | All seven PNM magics + PAM (P1-P7); 1/8/16-bit; comment-tolerant ASCII + binary; .pbm/.pgm/.ppm/.pnm/.pam + r183 user-defined PAM TUPLTYPE + r189 ASCII (P1/P2/P3) hot-path rewrite (stack-buffer digit writer + u8-direct emitters + checked u32 accumulator: encode P1 7.3→139 MiB/s ×19, P2 60→322 MiB/s ×5.4, P3 58→295 MiB/s ×5.1) + r222 **P7 PAM `GRAYSCALE` 16-bit row-level LE→BE byte-swap** (`encode_p7_gray16` mirrors the existing decoder swap so the depth-1 16-bit PAM path is byte-symmetric host-endian; round-trips bit-exact against decode) |
| ICO / CUR | ✅ | ✅ | — | Windows icon + cursor — multi-resolution, BMP and PNG sub-images; r178 body-dim `(0,256]` reject + r184 CUR hotspot body-derived bound (closes fuzz hotspot probe-vs-render panic) |
| slin      | ✅ | ✅ | — | Asterisk raw-PCM: .sln/.slin/.sln8..192 |
| MOD / S3M / STM | ✅ | — | — | Tracker modules (decode-only by design; r186 XM vol-col panning-slide + r192 XM instrument auto-vibrato `vibrato_type` byte selector + `+4` "don't retrigger" flag) + r220 **STM `9xx` set-sample-offset effect** (per `mem_sample_offset` PT-shape memory register; `xx * 0x100` byte cursor on note-trigger; `900` reuses latched; out-of-range silences but still latches pitch metadata; closes most prominent STM effect-column gap; +5 tests) + r222 **S3M `Oxy` loop-aware sample-offset wrap** (per ScreamTracker v3.20 §Oxy — `Oxy` past loop-end wraps within `loop_start..loop_end` instead of silencing; new `resolved_sample_offset(off, loop_start, loop_end, len, looped)` helper; non-looped samples retain silence-on-overflow; SDx-deferred path unchanged; +5 tests) |

Cross-container remux works for any pair whose codecs don't require
rewriting (FLAC ↔ MKV, Ogg ↔ MKV, MP4 ↔ MOV, etc.).

### Content protection

| Layer | Status | Notes |
|-------|:-------|-------|
| AACS  | ✅ Common 0.953 + BD-Prerecorded 0.953 | `oxideav-aacs` clean-room — KEYDB.cfg parser, `MKB_RO.inf` / `Unit_Key_RO.inf` parsers, Subset-Difference tree walk, Device-Key → Processing-Key → Media-Key → VUK derivation, AES-128-CBC Aligned Unit decryption, Title Key unwrap + Phase B SCSI MMC drive-command wire layer (REPORT_KEY / SEND_KEY / READ_DISC_STRUCTURE typed CDBs + AGID / Drive-Cert-Challenge / Drive-Key / Host-Cert-Challenge / Host-Key / Volume-ID sub-payload codecs + `DriveCommand` trait + `MockDrive` synthetic-fixture impl) + Phase C Drive-Host AKE (clean-room ECDSA over the AACS 160-bit curve + FIPS 180-2 SHA-1 + AES-128-CMAC; `host_authenticate` §4.3 state machine + `DriveAuthState` wired into `MockDrive`; Bus Key = lsb_128 of shared ECDH x-coord; §4.4 Volume-ID transfer w/ CMAC verify). + r177 READ_DISC_STRUCTURE Format 0x81 / 0x82 / 0x83 typed sub-payloads (PMSN, Media-ID, MKB-pack body up to 32 KiB; CMAC verify per §4.5/§4.6/§4.14.3.4; MockDrive serves Format 0x81/0x82). + r183 MKB ECDSA verify §3.2.5.1.2/.3/.8 (host/drive revocation list + end-of-block signature; caller-supplied AACS LA pubkey) + r188 BD-Prerecorded §2.3 Content Hash Table + r200 `KEYDB::parse_with_report` structured `ParseReport` (1-based `line_number` + UTF-8-boundary-safe 80-byte `snippet` + `Display`-formatted `AacsError` `reason` per skipped line; `KeyDb::parse` unchanged) + 27-case fuzz/robustness suite + r211 **Phase C AKE/EC self-check entry points** (`curve_self_check` / `aacs_la_pub_self_check` / `ake_ecdh_self_check` / `ake_full_self_check` / `all_self_checks` cascade; G-on-curve + n·G=O + scalar distributivity + `Hk·Dv == Dk·Hv` shared-x + full `host_authenticate` round-trip against synthetic-LA-rooted `MockDrive`; `AacsError::SelfCheckFailed { what: &'static str }`; 199 tests) + r222 **signed Content Certificate parse + verify** (`content_certificate` module — generic PVB §2.4 Table 2-1 (header + Format_Specific_Section + N·8-byte CHT digests + 40-byte signature) + BD-Prerecorded §2.4-§2.6 Format_Specific_Section (`Hash_Value_of_MC_Manifest_File`(20) + `Hash_Value_of_BDJ_Root_Cert`(20) + `Num_of_CPS_Unit`(2) + per-unit `Hash_Value_of_CPS_Unit_Usage_File`(20)); Content Certificate ID = `Applicant_ID || Content_Sequence_Number`; §2.6 AACS_Verify over CC minus signature + `CHT_d = [SHA-1(CHT)]_lsb_64` digest; 16-test unit + 6 integration; closes the long-standing "lacks signed Content Certificate Table 2-1 verify" tail). Lacks AACS 2.0 (UHD-BD) |

</details>

### Codecs

> Each row below is a current-state summary. For round-by-round history, design notes, and per-feature trade-offs, see the per-crate `README.md` and `CHANGELOG.md` in `crates/oxideav-<codec>/`.

<details>
<summary><strong>Audio</strong> (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **PCM** (s8/16/24/32/f32/f64) | ✅ 100% | ✅ 100% |
| **slin** (Asterisk raw PCM) | ✅ 100% | ✅ 100% |
| **FLAC** | ✅ 100% — bit-exact vs RFC 9639 + CUESHEET → Chapter API + r163 RFC 9639 §8.8 typed PICTURE accessor (parse + write; 92 tests) | ✅ 100% — bit-exact roundtrip + LPC order/window/precision search + closed-form Rice estimate + flamegraphs + §8.6 PADDING writer + composable block-header serialiser + opt-in PADDING reservation + r186 partitioned-Rice search O(1)-per-partition prefix-sum + raw-bits table (~13-20% encoder speedup on s24/multi-ch scenarios; bit-identical) |
| **Vorbis** | 🚧 r9 (post-2026-05-20 orphan) — identification + comment + §3.2.1 codebook + Huffman tree + full §4.2.4 setup-header walker + §3.2.1/§3.3 VQ vector unpack + §8.6 residue decode (formats 0/1/2) + §7.2.3/§7.2.4 floor type 1 + §6.2.2/§6.2.3 floor type 0 LSP + §1.3.2/§4.3.1 Vorbis window + §4.3.5 inverse channel coupling + §4.3.3 nonzero-vector propagate + §4.3.6 floor×residue + §4.3.1–§4.3.8 audio-packet driver + r180 §4.3.7 IMDCT + r186 streaming overlap-add | 🚧 r2 — r195 §4.2.1+§4.2.2 identification-header WRITE + §5.2.1 comment-header WRITE + r201 §3.2.1 codebook WRITE + §9.2.2 `float32_pack` (bit-exact roundtrip across 3 length encodings × 3 lookup types; auto-picks densest sparse/ordered/dense) + r206 §7.2.2 floor type 1 WRITE + r212 §6.2.1 floor type 0 header WRITE + r218 **§8.6.1 residue header WRITE** (`encoder::write_residue_header` bit-exact inverse of r5 parser across formats 0/1/2 incl. low_bits/high_bits 5-bit-elision rule + 8 `WriteResidueError` variants gated on §8.6.1 invariants; 452 lib tests) + r223 **§4.2.4 mapping header WRITE** (`encoder::write_mapping_header` bit-exact inverse of r5 parser; densest defaults `submaps_flag=0` when `submaps==1` + `square_polar_flag=0` when coupling empty; 11 `WriteMappingError` variants gated on §4.2.4 invariants; 488 lib tests); lacks audio-packet WRITE + mode WRITE + setup-header splice |
| **Opus** | 🚧 ~33% — RFC 6716 range decoder + full SILK pipeline + §4.3 Table 56 CELT pre-band header + §3.1/§4.2 framing dispatch + r195 §4.5.2 SILK+CELT state-reset policy + r200 §4.5.1.4 redundant-frame decode params + r204 §4.3.2.1 CELT coarse-energy Laplace `E_PROB_MODEL` 336 Q8 bytes + r217 **§4.3.3 `CACHE_CAPS50` per-band cap table + `init_caps()` convert helpers** (336 Q8 bytes via `cache_caps_offset()` / `cache_caps_value()` / `cache_caps_row()` + `CacheCapsStereo::{Mono,Stereo}` selector + `((value+64)*channels*N)/4` Eq. closes round-24 allocator table-dependency pair; 560 tests) + r223 **§4.3.3 alloc-trim PDF + signalling gate** (`celt_alloc_trim` module — RFC 6716 Table 58 PDF `{2,2,5,10,22,46,22,10,5,2,2}/128` + derived iCDF `[126,124,119,109,87,41,19,9,4,2,0]`; trim ∈ [0,10] with default 5; `alloc_trim_is_signalled` saturating gate (48 eighth-bits cost vs 64-per-byte); `decode_alloc_trim` composite wrapper; +33 lib tests = 593 lib + 20 integration); per-LM inter-mode `(α,β)` deferred | 🚧 scaffold |
| **MP1 / MP2** | ✅ Layer I + Layer II decode to PCM + §2.4.3.1 CRC-16 verify + mp2 frame-level decode loop + r191 Annex D Phase-2 psy + r203 Annex D Phase-3 Step 3 LTq offset + Model 2 spreading + r211 §2.4.3.1 free-format frame-length probe + r216 **ISO 13818-3 Annex B Table B.1 LSF Layer II allocation table wired** (closes long-standing alias to MPEG-1 Table 3-B.2b — Table B.1 has strictly narrower per-subband ladder Σ nbal=75 vs B.2b's 94, with row-shapes `{4,4,4,4, 3·7, 2·19, 0,0}` per spec §2.4.3.1; `tables_layer2::layer2_bit_allocation_table` now routes LSF Fs∈{16,22.05,24} kHz to `TABLE_LSF`; 259 lib tests); allocator still pending D.1/D.3/D.4 PNG→text #1262 | 🚧 ~82% — Layer I encoder + Layer II §C.1.5.2.7 bit-allocation + r192 §C.1.3 polyphase analysis filterbank + r197 §C.1.5.1.4 Layer II per-part scalefactor extraction; + r222 **§2.4.1.6 / §C.1.5.2 top-level Layer II frame encoder `encode_layer2_frame`** (single call wires §C.1.5.1.4 scalefactor selector + §C.1.5.2.7 bit allocator + §2.4.1.6 region writers behind `quantize_layer2_sample` exact-inverse of the decoder requantizer; optional §2.4.1.4 CRC patch; shared-allocation invariant enforced; +7 lib tests); still pending `Mp1Encoder` Layer II switch + Table C.4 SCFSI selection (PNG→text gap) |
| **MP2** | 🚧 ~45% (post-2026-05-24 orphan) — §2.4.1.3/§2.4.2.3 Layer II header parser + §2.4.3.1 frame sizing + Annex B tables + joint-stereo + scfsi + §2.4.3.3.4 sample requantizer + §2.4.3.2 polyphase synthesis filterbank + frame-level decode loop + r185 full LSF Layer II + r202 §C.1.5.2.5/§C.1.5.2.6 SCFSI Table C.4 encoder-side selection + r208 §2.4.1.6 `write_audio_data` encoder bit-for-bit inverse of parser + r214 §C.1.5.2.7 iterative encoder bit-allocator + r220 §2.4.3.3.4 encoder sample quantizer + r227 **§2.4 / Annex C top-level frame-level encode orchestrator** (`encode_frame(header, pcm, smr_db, banc) -> Result<Vec<u8>, EncodeError>` pulls §C.1.3 analysis filterbank → §C.1.5.2.6 scalefactor extraction → §C.1.5.2.7 bit allocation → §C.1.5.2.5 Table C.4 SCFSI selection → §2.4.1.6 audio-data writer → §2.4.3.3.4 sample codewords → §2.4.3.1 CRC-16 patch into one call; `encode_frame_with` + `EncodeFrameState` persists the §C.1.3 X ring buffer across frames; round-trip identity via `frame::decode_frame` for stereo / single-channel / joint-stereo / no-CRC headers + `CrcMismatch` on bit-flip + joint-stereo above-`bound` allocation balance invariant; +15 lib tests = 230 lib + 14 integration); lacks Annex D §D.1 / §D.2 psychoacoustic SMR model (caller-supplied `SmrTable` for now) | 🚧 scaffold |
| **MP3** | ✅ ~100% — bit-exact decode + ID3v2/Xing seek + MPEG-2.5 framing; 634 tests | 🚧 ~95% — Phase-2 + r194 long + r197 pure-short + r204 mixed-block per-band threshold-in-quiet path + r207 trait-API one-shot Annex D threshold-in-quiet factory + r213 §D.1 Step 3 caller-supplied dB offset path + r219 **§D.1 Step 6 `vf` masking-function + Step 7 `LTg` global masker** (`psy::masking_index_tonal(z)` / `masking_index_non_tonal(z)` av formulas; `masking_function_vf(dz,X)` 4-branch piecewise on `dz∈[-3,8)`; `individual_masking_threshold_db(masker,z_i)` composes `LT=X+av+vf`; `global_masking_threshold_db` Σ-power-sum w/ LTq; typed `Masker{kind,z_bark,spl_db}` + `MaskerKind::{Tonal,NonTonal}`; 587 tests) + r224 **§D.1 Step 4 critical-band-boundary Tables D.2a-f** (six Layer I/II × 32/44.1/48 kHz tables transcribed from staged text extract dc78918; `CriticalBandBoundary { no, index_fcb, frequency_hz, z_bark }` carrier + `critical_band_boundaries(layer, fs)` dispatch + `band_of_fft_line(boundaries, idx)` mapper feeding the future tonal-vs-non-tonal classifier; typed `AnnexDSamplingRate`; D.2e row 17 `z_bark` PDF-illegible prefix `16.11` adopted verbatim); lacks FFT/SPL/tonality classifier + Model 2 + intensity-stereo |
| **AAC** | 🚧 Phase 1 — ADTS + raw_data_block walker + AudioSpecificConfig + program_config_element + r177 §4.4.1 GASpecificConfig extensionFlag + Table 1.15 epConfig + r192 §1.6.5 Table 1.15 trailing `syncExtensionType=0x2b7` implicit-SBR/PS/ER-BSAC probe (`AudioSpecificConfig.trailing_sbr_probe`; ext-AOT 5 reads sbrPresentFlag + optional 4-bit ext-sfi w/ 24-bit escape + secondary `0x548` sync gating psPresentFlag; ext-AOT 22 reads sbrPresentFlag + mandatory 4-bit ext-channel-config; `parse_bits_bounded` for LATM/esds carrier-bounded callers) + r194 §4.5.4.1 SWB offset tables + §4.6.13 `apply_pulse_data` reconstruction + r200 §4.6.9.4 TNS_MAX_ORDER/BANDS clamp surface + r207 §4.4.6 Table 4.50 `ics_body` walker + r213 §4.6.3.1 Table 4.95 spectral codebook + §4.6.3.3 idx→tuple translation + r219 **Table 4.A.2 Spectrum Huffman Codebook 1 wire** (`HCOD1_*` + `hcod1_encode/decode/write`; signed 4-tuple LAV=1; 81 entries; max codeword length 11 bits; zero-tuple `0` 1-bit codeword; Kraft equality `Σ 2^(11-L)=2048` complete-prefix verified; foundational for §4.6.3 spectrum walker; 580 tests) + r226 **Table 4.A.3 Spectrum Huffman Codebook 2 wire** (HCOD2 — second of eleven §4.6.3.1 spectrum codebooks; signed 4-tuple LAV=1; 81 entries; max codeword length 9 bits; zero-tuple `0b000` 3-bit codeword; shortest non-zero 4-bit at index 67; Kraft equality `Σ 2^(9-L)=512` complete-prefix verified); decoder body still pending Huffman codebooks 3-11 + raw_data_block→ics_body wiring | 🚧 scaffold — Phase-2 writers: section_data + ics_info + pulse_data + tns_data + scale_factor_data + DPCM + r160 raw_data_block + r165 Pce::write + r183 gain_control_data SSR + r187 §4.4.2.7 extension_payload; SBR types pending QMF |
| **CELT** | 🚧 r13 (post-2026-05-20 orphan) — RFC 6716 §4.1 range decoder + §4.3 prefix + §4.3.2.1 coarse-energy scaffold + §4.3.3 bit-allocation fields + §4.3.4 tf_change/tf_select + r181 §4.3.4.3 spread + r187 §4.3.7.1 post-filter + §4.3.7.2 de-emphasis + r195 §4.3.4.5 Walsh-Hadamard primitives + r200 §4.3.3 `cache_caps50` + dynamic-band-boost decode loop + r207 §4.3.3 initial-reservations budget walk + r213 §4.3.3 §2.6 per-band minimums + trim_offsets + Table 55 + r220 **§4.3.3 Table 57 static-allocation matrix** (`STATIC_ALLOC: [[u8; 11]; 21]` verbatim from RFC body lines 6234-6286 in 1/32 bit/MDCT-bin units + `interp_alloc_1_32nd` 1/64-step linear interp + `band_static_alloc_1_8th` `channels*N*alloc<<LM>>2` + `window_static_alloc_1_8th` coded-band-window sum for the static-allocation search driver; 229 tests) + r226 **§4.3.3 static-allocation search** (RFC 6716 binary-search dispatch over r220 Table 57 + r213 trim/min adjustments — closes pure-static branch of bit-allocation); blocked on docs #936 (Laplace) | 🚧 scaffold |
| **Speex** | 🚧 ~28% — Ogg stream-header + NB + WB high-band + §5.5 in-band signalling + r179 `BitWriter` + r187 encoder-side `write` + r191 22 CELP companion-table accessors + r194 NB LSP-VQ → Q10 LSP reconstruction + r200 §9.1 per-sub-frame LSP linear interpolation + r208 **NB 3-tap pitch-gain VQ reconstruction** (Manual Eq. 9.1 / CELP companion §2.2: `PitchGainTaps { taps: [i16; 3] }` β taps `(g0, g1, g2)` of LTP convolution `ea[n] = g0·e[n−T−1] + g1·e[n−T] + g2·e[n−T+1]`; resolves 5-bit/7-bit pitch-gain VQ; +32 codebook bias applied in-module; column 3 `search_aid` dropped; `NarrowbandSubFrameIndices::pitch_gain_taps(submode)` lookup; 205 tests) + r214 WB-HB 2-stage LSP MSVQ Q10 reconstruction + r220 **NB fixed-codebook (innovation) sub-vector lookup + per-mode dispatcher** (`InnovationCodebook` enum 6 shapes / `sub_vector(cb,idx)` static-row return / `InnovationMapping::for_mode(submode)` `Silence`/`Documented`/`Undocumented` 3-way / `decode_subframe(submode, idx) -> [i16; 40]` MSB-first chunked decode; modes 6/8 documented from staged docs, modes 1-5+7 surface as `InnovationError::Undocumented` docs-gap; 224 tests); lacks §9.1 LSP→LPC + synthesis + UWB framing + per-mode codebook binding for 7 unbound modes | 🚧 scaffold |
| **GSM 06.10** | 🚧 ~30% — r185 clean-room §5.3 RPE-LTP decoder + r200 §4.4 in-band homing + §5.1 `norm`/`div` primitives | 🚧 r207 §5.2.0..§5.2.3 pre-processing + r213 **§5.2.4..§5.2.6 LPC-analysis front-end** (`encoder::analysis::{autocorrelation, reflection_coefficients, reflection_to_lar, analyse_frame}`; §5.2.4 dynamic input scaling `scalauto`/`mult_r`; §5.2.5 Schur with `L_ACF[0]==0` + `P[0]<|P[1]|` aborts; §5.2.6 3-segment piecewise breakpoints 22118/31130 + r218 **§5.2.7 LAR quantisation + coding** (`analysis::quantise_lar` per-index Table 5.1 A/B/MIC/MAC arithmetic with full saturation + LARc 3/4/5/6-bit packing; `analyse_and_quantise_frame` end-to-end §5.2.4→§5.2.5→§5.2.6→§5.2.7 wrapper; 93 tests) + r225 **§5.2.8..§5.2.10 short-term analysis filter** (`analysis::Analyzer::analyse_frame(s) -> (LARc[1..=8], d[0..160])` runs §5.2.7→§5.2.8→§5.2.9.1→§5.2.9.2→§5.2.10 end-to-end across the four sub-blocks (k=0..=12 / 13..=26 / 27..=39 / 40..=159) persisting `LARpp(j-1)` + `u[0..=7]` per §4.5 Table 4.2; `short_term_analysis_filter(s, rp, u, k_start, k_end)` 8-stage lattice w/ verbatim spec inner-loop sequence); `make_encoder` still `Unsupported` until §5.2.11..§5.2.18 + §1.7 packer |
| **G.711** (μ/A-law) | ✅ 100% | ✅ 100% |
| **G.722** | 🚧 r185 clean-room SB-ADPCM decoder bring-up + r200 BLOCK1/QMF predictor split + r207 Table 19 RIL=11111 sign-anomaly fix + r212 Figure 1/G.722 auxiliary-data channel + r218 **clause 2 transmission characteristics** (`transmission` module — typed normative-limits constants per clause/page; dBm0 ↔ uniform-PCM bridge; `IdleNoiseReport`/`measure_idle_noise` end-to-end clause-2 wideband −60 dBm0 check per mode; 77 tests) | 🚧 r200 SB-ADPCM encoder bring-up + r207 Mode-2/Mode-3 silence envelope round-trip + r225 **Appendix-II test-sequence harness + QMF-bypass entry points** (`Encoder::encode_subband_pair(x_l, x_h)` + `Decoder::decode_subband_pair(i_lr, i_h)` drive the sub-band ADPCM encoders/decoders directly per clause II.2.1/II.2.2 "QMFs by-passed"; new `test_harness` module exposes the four normative sub-blocks of clause II.2.3 translating between 16-bit `X#`/`I#`/`RL#`/`RH#` test-sequence words and the codec's per-sample inputs); lacks clause 2.5.2 reconstructing-filter mask |
| **G.723.1** | ✅ 100% | ✅ 100% — both 5.3k + 6.3k |
| **G.728** | 🚧 ~16% — clean-room decoder front-end: Annex A/B/C/D tables + block-50 Levinson-Durbin + blocks 29/31/32 + r195 blocks 30/33 backward synthesis-filter + vector-gain adapters + r201 blocks 73-77 postfilter AGC tail + r207 block 72 short-term (spectral) postfilter + r213 §4.6 block 71 long-term (pitch) postfilter `LongTermPostfilter` + r220 **§4.7 block 81 LPC inverse filter** (`PitchInverseFilter` eq. 4-6 `Ã(z) = 1 − Σ ã_i·z^-i` with `ã_i = -a_i` sign-flip from synthesis byproduct; 10-tap FIR memory carry-over; refresh per adaptation cycle; `Decoder::pitch_inverse_filter()` accessor; bit-for-bit cold-start passthrough preserved; 125 tests) + r223 **§4.7 block 82 PitchSearch** (`pitch_search` module — 240-sample `d(−139..100)` LPC-residual buffer + 60-sample `d̄(−34..25)` decimated buffer fed by Annex D third-order elliptic 1 kHz lowpass + 4:1 decimator; coarse `ρ(i)` over decimated lags 5..=35 (eq. 4-7); full-resolution `C(i)` refinement clamped into `[KPMIN=20, KPMAX=140]` (eq. 4-8); fundamental-vs-multiple resolution via `β0`/`β1` clamped to `[0,1]` + `TAPTH=0.4` selector (eqs. 4-9..4-11); 148 tests); residual gap scoped to §4.7 blocks 83/84 + encoder | 🚧 scaffold |
| **G.729** | 🚧 ~10% — clean-room from staged trace #859: r173-r195 tables + serial parser + LSP-quantiser codebooks + corpus harness + r201 §3.2.4 MA-predictor `fg` family + r207 §3.2.4 LSP-frame reconstruction + r213 §3.2.5 per-subframe LSP interpolation + r218 **§3.2.6 LSP→LP conversion** (`lsp_to_lp` eqs (13)/(14) F1/F2 polynomial recursion + (25) `(1±z^-1)` factor restoration + (26) `A(z)=(F'_1+F'_2)/2` recombination; `lsf_to_lp` ω-domain wrapper; full §3.2.4→§3.2.5→§3.2.6 chain wired; 41 tests) + r225 **§4.1 / Table-8 parameter unpacker** (`parameters` module splits round-191 serial 80-bit payload into the 15 typed codeword indices `Parameters { L0, L1, L2, L3, P1, P0, P2, C1, S1, C2, S2, GA1, GB1, GA2, GB2 }`; `unpack_parameters(FrameKind)` / `unpack_bit_array(&[bool;80])` read codewords MSB-first per Table-8 NOTE with `BITS_PER_FRAME=80` statically asserted; `Parameters::pitch_parity_ok()` per §3.7.2/§4.1.2 odd-parity convention; 9 new lib tests = 50); lacks §4.1.1 bit-extraction glue + gain GA/GB + postfilter + Annex B DTX | 🚧 scaffold |
| **IMA-ADPCM (AMV)** | ✅ 100% | ✅ 100% |
| **MS-ADPCM / IMA-ADPCM (WAV)** | ✅ 100% | ✅ 100% — block-aligned WAV encoder for both nibble layouts |
| **OKI / Dialogic VOX** | ✅ 100% — r186 clean-room from Dialogic app note 00-1366-001 (1988); HiFirst (VOX/MSM6295) + LoFirst (MSM6258) nibble orders, Native12 + Wide16 output | ✅ 100% — symmetric §3 closed-form encode; mono-only via registry (Dialogic hardware constraint) |
| **8SVX** | ✅ 100% | ✅ 100% |
| **iLBC** (RFC 3951) | ✅ 100% — NB 20/30 ms + r219 **§3.8 uneven-level-protection wire layout** (3-pass class-1/2/3 pack/unpack on Appendix A.41 `ULP_20msTbl`/`ULP_30msTbl`; bit-IO-agnostic `pack_emit_list`/`unpack_logical` driver; FFmpeg-fixture PSNR silence 75→95 dB / step-impulse 34→39 dB / voice-like +1..+2.4 dB; 212 tests) + r226 **RFC 3952 §4.2 outbound SDP fmtp builders** (mode-NB-20ms/30ms `a=fmtp:<pt> mode=<n>` line emitters for SIP offer/answer) | ✅ 100% |
| **AC-3 / AC-4** (Dolby Digital / Dolby AC-4) | ✅ ~97% — AC-3 full decode + E-AC-3 SPX + TPNP + AHT + §7.8.2 LtRt downmix + r126 Annex D mix-level + WAVE_FORMAT_EXTENSIBLE + r172 SPX-attenuation border + r182 §7.10.1 CRC verifier + r187 §7.10.1 augmented crc2 + r193 typed `BitStreamMode` accessor for Table 5.7 + r196 §E.2.3.1.8 E-AC-3 `chanmap` routing + r202 §7.7.2.2 typed `CompressionGain` + r208 typed xbsi2 / informational-metadata Dolby Surround EX/Dolby Headphone/A/D-converter type (`dsurexmod`/`dheadphonmod`/`adconvtyp` per Tables D2.7/D2.8/D2.9) + r214 typed `AudioProductionInfo` for §5.4.2.13-15 + r219 **typed `TimeCode1`/`TimeCode2`/`TimeCodePresence` for §5.4.2.26-28 + Table 5.13** (`Bsi::timecod1` 5+6+3 bits → `hours`/`minutes`/`eight_second_increments`+`seconds_in_day`; `Bsi::timecod2` 3+5+6 bits → `seconds`/`frames`/`frame_fractions`; Table 5.13 4-state enum + `from_flags`; Annex D `bsid==6` short-circuit; 234 tests) + r226 **typed `CopyrightInfo` surface for `copyrightb`/`origbs`** (§5.4.2.24-25 / §E.2.3.1.62 — 2-bit composite carrier on `Bsi`, byte-stable Annex D `bsid==6` short-circuit) | 🚧 AC-3 ~95% + AC-4 IMS r215 **5_X SIMPLE/ASPX_ACPL_3 real γ₁/γ₂/γ₃/γ₄** per-band extraction (closes r208 zero-delta deferral via 2×2 normal equations on carrier+surround MDCT spectra; analytic γ₁/γ₂ resolves from `(L + Ls/√2)` Pseudocode 118 cancellation; `extract_gamma_{1_2,3_4}_q_per_band_surround_least_squares` + new `_real_alpha_beta_full_gamma` writer + 5_0/5_1 PCM entry; 822 tests) + r226 **`write_aspx_data_{1,2}ch_real_envelope()` builders** (per-channel ASPX envelope encoder wrapper around r215 real-α/β/γ extraction) |
<!-- ac3 decode r129: E-AC-3 mixmdata mix-levels (ltrt/loro c/sur) now surfaced + routed through §7.8 downmix in process_eac3_frame -->
| **AC-4** (Dolby) | 🚧 ~98% — A-SPX + DRC + 60+ ETSI codebooks + 5_X/7_X ACPL_1/2/3 + cfg0/1/2/3 + LFE + SSF/SNF + SAP + Pseudocode 121 companding + IMS bitstream_version≥2 walker + r181 §5.7.7.7 Pseudocode 121 + r190 Table 126 `aspx_int_class = FIXFIX` writer width fix; lacks ETSI fixture RMS audit, object/a-joc | 🚧 IMS ~72% — v0/v2 TOC + mono/stereo/joint M/S + 5.0/5.1/7.1 SIMPLE Cfg3Five + 5_X SIMPLE/ASPX_ACPL_1/2 + ASPX_ACPL_3 + r132/r135/r139/r144 real per-band α+β for ACPL_1/2 + r193 real per-band β1/β2 for 5_X ASPX_ACPL_3 + r196 real per-band α1/α2 for 5_X ASPX_ACPL_3 + r202 real per-parameter-band α + β for 7.0/7.1 SIMPLE/ASPX_ACPL_2 + r208 5_X SIMPLE/ASPX_ACPL_3 real per-band γ5/γ6 extraction + r219 **ASPX envelope value-emitting helpers** (`aspx_sig_hcb_arrays()` 12-entry `(LEN,CW,cb_off)` + `aspx_noise_hcb_arrays()` 6-entry; SIGNAL F0/DF/DT dispatched on `(quant_mode, stereo_mode)` per Annex A.2 Tables A.16..A.33; NOISE F0/DF/DT dispatched on `stereo_mode`; mirrors `acpl_hcb_arrays()` pattern; 834 tests); lacks γ1..γ4 (need 5.1+Ls+Rs PCM input layout) + 7_X ACPL_3 β + Table-181 SAP residual + back-pair Lb/Rb |
| **MIDI** (SMF) | ✅ ~99% — SMF Type 0/1/2 → PCM via 32-voice mixer + SF2/SFZ/DLS + FF 01..07 text-meta iterator family (10 helpers) + r208 `smpte_offsets()` FF 54 + `FrameRate` enum + r213 `channel_snapshot_at`/`channel_snapshots_at` channel-state seek primitive + r219 **`sequencer_specifics()` FF 7F iteration helper** (yields manufacturer-id-tagged proprietary blob extents per SMF spec, alongside r208 `smpte_offsets()` / r213 snapshot family) + r224 **`sequence_numbers()` FF 00 02 ssss iteration helper** (`SequenceNumberEvent { tick, track, number }` big-endian u16 decode per absolute tick on parent track; lifts meta-event iterator family to 13 total; surfaces every occurrence rather than enforcing SMF 1.0 first-event placement; stable cross-track merge; +8 tests = 415); r172 cargo-fuzz (30M+ panic-free) | — synthesis only |
| **NSF** (NES) | 🚧 ~96% — full 6502 + IRQ/NMI + 5/5 2A03 APU + DMC DMA + six expansion chips + NSF v1/v2/NSFe + Dendy region + r154 Namco 163 + r185 VRC7 OPLL pipeline + r199 VRC7 register semantics + r204 **VRC7 KSR (Key Scale of RATE)** per YM2413 §III-1-2 Table III-2 (`Envelope::update_rks(block, fnum_msb)` cached RKS: KSR=0 → `block >> 1`; KSR=1 → `(block << 1) \| fnum_msb`; 4-bit per-stage R widens to 6-bit RATE = 4·R + RKS via `Envelope::effective_rate(r)` with explicit R=0→RATE=0 halt carve-out; pitch-only `$1X`/`$2X` writes trigger mid-note `refresh_rks` glide; 213 tests). + r209 **§4 KSL (Key Scale of LEVEL) formula scaffold** (`ksl_attenuation_env_levels`/`ksl_base_attenuation` + `KSL_BASE_BYTE_TABLE: [[u32; 16]; 8]` exposing `(base) >> (3 - KSL)`; block 0 bit-exact; blocks 1..=7 zero scaffold awaiting #1363; `OpllChannel.{mod_ksl,car_ksl}` capture on `load_patch`; trip-wire test; 202 tests; Rule-E scrub of `src/opll.rs:23-43` resolves #1339) + r215 **MMC5 PCM Mode/IRQ register + `$8000..=$BFFF` read-mode write-by-read** (`$5010` write decodes bit 7 PCM-IRQ-enable alongside bit 0 mode-select; `$5010` read = `(irqTrip AND irqEnable)` with ack-clear; `$5011` write-mode `value==0 → irqTrip=1, DAC unchanged`; `Mmc5::observe_prg_read` + 4-way `irq_line` OR; 218 tests) + r223 **VRC6 sawtooth 14-step cycle correction + E-clear accumulator zero** (§"Sawtooth Channel" example A=$08 bit-correct walk `0,0,8,8,16,16,24,24,32,32,40,40,48,48` via `step % 14` replacing malformed `step & 0x0D` mask; §note E-clear `$B002` falling-edge zeros accumulator + step while preserving freq divider; +12 lib tests = 230). Lacks §4 byte base table rows 1..=7 (#1363) + §7 per-rate env tables + rhythm mode | — synthesis only |
| **Shorten** (.shn) | 🚧 r13 (post-2026-05-18 orphan) — `ajkg` magic + v2/v3 ulong + svar(n) + per-block function dispatch + VERBATIM/QUIT + DIFF0..3 + Rice residual + per-channel carry + spec/05 §2.5 running mean + QLPC predictor + r7 `decode_stream` + r145 `Decoder` trait + r181 block-by-block + r187 streaming `Decoder` + r191 envelope encoder surface + r197 **`write_diff0_block` predictor encoder** (full `<fn=0> <energy> <residual>×bs` command per spec/03 §3.1 + spec/05 §3.1; `min_energy_for_diff0` selector; encode→decode round-trips byte-exact through `decode_stream` across DIFF0+VERBATIM splice, silent block, ±100 max-natural residuals; +15 tests = 203)+ r209 **`write_diff1_block` order-1 polynomial-difference predictor encoder** (per spec/03 §3.2 + spec/05 §1.1 + §3.1; seeds `s(t−1)` from `carry.at(0)`, writes `e₁(t) = s(t) − s(t−1)` under `svar(energy_encoded + 1)`; mean-invariant per spec/05 §2; `min_energy_for_diff1` natural-energy selector; byte-exact roundtrips via `decode_stream`; 224 tests) + r215 **`write_diff2_block` order-2 polynomial-difference predictor encoder** (per spec/03 §3.3 + spec/05 §1 + §3.1; seeds `s(t−1)`/`s(t−2)` from `carry.at(0)`/`carry.at(1)`, writes `e₂(t) = s(t) − (2·s(t−1) − s(t−2))` under `svar(energy_encoded + 1)`; mean-invariant per spec/05 §2; shared `min_energy_for_diff2` selector; 244 tests) + r223 **`write_diff3_block` order-3 polynomial-difference predictor encoder** (per spec/03 §3.2 eq. 6 + spec/05 §1.1 + §3.1; seeds `s(t−1)`/`s(t−2)`/`s(t−3)` from `carry.at(0..=2)`, writes `e₃(t) = s(t) − (3·s(t−1) − 3·s(t−2) + s(t−3))` under `svar(energy_encoded + 1)`; mean-invariant per spec/05 §2; `min_energy_for_diff3` selector + `FN_DIFF3=3`; 264 tests); lacks QLPC predictor encoder + sequencer + #1267 | 🚧 scaffold |
| **TTA** (True Audio) | ✅ ~98% — TTA1 fmt=1/2 + password + ID3v1/APEv2 trailer + r187 streaming + random-access decode API + r198 streaming bench parameter-cube + r204 **`Decoder::new_with_password` brings streaming + random-access onto format=2 streams** (ECMA-182 CRC-64 digest from `spec/07` §3.2 + Stage-A `qm[0..7]` priming at every per-channel frame init per §3.5–§3.6; format=1 transparent alias via clear_priming) + r209 **`Decoder::decode_from_sample(sample_index)` + `frame_iter_from_sample(sample_index)` random-access player-API sugar** (eager + lazy `SampleSkipIter` suffix of `decode_all` from per-channel sample boundary; both reuse `seek_to_sample`'s spec/01 §4.1 arithmetic; cover format=1 + format=2; pre-existing libtta citation in `src/roundtrip_tests.rs:20-21` paraphrased per Rule E in same commit, resolves #1338; 101 lib tests) + r215 **duration-keyed player-API quartet** on `Decoder` (`total_duration` + `seek_to_time(d)` + `frame_iter_from_time(d)` + `decode_from_time(d)`; floor(time_ns × sample_rate / 1e9) widened to u128 — monotone non-decreasing across full envelope, overflow-free; pure sugar over r209 sample-keyed primitives so format=2 password-protected reach propagates unchanged; 123 lib + 9 integration tests) + r226 **`sample_range` cargo-fuzz target** (drives r209/r215/r219 player-API surface — `seek_to_sample`/`seek_to_time` + iterator + decode-from-sample) | ✅ ~96% — TTA1 fmt=1/2 + password; bit-exact self-roundtrip |
| **APE** (Monkey's Audio) | 🚧 r190 Phase 1 + r206 polish — 8-byte `MAC ` magic + decimal-coded version + 5 compression-level enum prefix parser + `Display` for `CompressionLevel`/`HeaderPrefix` (surfaces `version_raw` so unknown encoder values stay distinguishable) + 2040-input single-byte mutation harness asserting every result is `Ok` or a documented `Error` variant (18 unit + 8 integration + 1 doctest); per-version header tail + IIR coefficients + residual `k` recurrence + range-decoder bounds + channel decorrelation all DOCS-GAP | 🚧 scaffold |
| **Musepack** | 🚧 r197 — SV7 §2.5/§2.6 requantiser constants + SV7/SV8 stream-magic recognisers + SV8 packet outer-frame walker + r197 SV7 mpc_huffman tables + CNS PRNG + r201 SV7 §2.5 per-band sample-decode dispatcher (`BandDecodeCase` classifier covers all 18 spec cases; Cns=−1 / Empty=0 / HuffmanPerSample=3..=7 / PcmEscape=8..=17 live; Grouped1/2 + SV8 canonical-Huffman walk surface as DOCS-GAP via `Error::UnsupportedBandType(i8)` per #1323) + r206 **SV7 §2.6 reconstruction primitives** (`centre_pcm_level`/`centre_pcm_band` PCM-escape centring for band_types 8..=17, `dequantise_sample` covering CNS band -1 + normal 0..=17 via `centred * C / 65536`, `dequantise_band`/`dequantise_huffman_band`/`dequantise_cns_band` convenience wrappers, `pcm_escape_d` + `DEQUANT_DIVISOR=65536.0`; cross-module bit-reader→PCM-escape→centring→dequant integration test; 85 tests) + r214 **SV7 §2.4 SCF coding-method decoder** (`scf` module on top of round-197 SCFI/DSCF Huffman; `ScfCodingMethod` + `GranuleSchedule::{deltas_to_read,granule_to_slot}` + 4 granule schedules transcribed verbatim + `BandScf` + `reconstruct_scf_from_deltas(reader, base, schedule)` + `decode_band_scf(reader, base)`; `SCF_GRANULES_PER_BAND=3` / `SCF_MAX_DISTINCT=3`; new `Error::InvalidScfCodingMethod(i8)`; 101 tests) + r223 **SV7 §2.3 band-type header loop walker** (`sv7_band_header` module on top of round-197 `SV7_BANDTYPE_HEADER_TABLE`; `RawBandTypeVlc(i8)` typed wrapper preserves §2.3-VLC → §2.5-dispatcher-case remap honesty; `BandHeader { band_type: [_;2], ms_flag }`; `decode_band_header(reader, nch)` + `decode_header_loop(reader, max_band, nch)` walks `0..=max_band`; `SV7_SUBBAND_COUNT=32` + `SV7_MAX_BAND_INCLUSIVE=31`; new `Error::{MaxBandOutOfRange, ChannelCountInvalid}`; 120 tests); lacks SV7 fixed-header field map + §2.3-VLC→§2.5-case remap + SV8 canonical-Huffman + 32-band synthesis | 🚧 scaffold |
| **Cook** (RealMedia) | 🚧 r4 — flavor table + cookie parser + 8 DSP parameter tables + r194 open-time `DecodeConfig` (cookie ↔ flavor cross-check + sub-packet accounting) + r197 wire-level real-stream integration test + r203 cookie→flavor multi-match API + r217 **selector-family classification + typed per-family GAP errors** (`SelectorFamily::classify(selector)` + `is_parser_supported(selector)` + `CookCookie::family()` faithful to spec/01 §3.1 backend factory switch; new `Error::{NonExtendedSelectorNotSupported, MultichannelSelectorNotSupported}` typed-GAP variants; `UnsupportedSelector` reserved for `0x80040005` factory-reject values; 43 tests) + r224 **`CallSession` stateful RADecode call-counter + PCM-cursor over wired `DecodeConfig`/`SubPacketLayout`** (`next_call_expected_input_len` / `next_call_pcm_bytes` warmup vs steady-state / `next_call_pcm_byte_range` / `advance_one_call(input_len, output_len)` with typed `Error::CallInputLengthMismatch`/`CallOutputLengthMismatch`; closes the third pipeline structural layer above backend frame-decode); lacks bitstream decode | — |
| **WMA** | 🚧 r4 — patent-disclosed primitives (r197 mid-side stereo + run/level walker) + r203 quantization-matrix differential coding + entropy-mode selector + r216 **§6 codebook grid + escape disposition** (US6,223,162 Claims 4-10: `CodebookGrid` 2-D `(R,L)` probability grid bounded by `(Rm,Ln)` w/ threshold; `Disposition::{InCodebook,Escape}` + `probability_of` + `is_in_codebook`/`is_escape` + `in_codebook_pairs()`; 6-variant `InvalidGrid` error; 163 tests); lacks codeword Huffman tables / exponent partition / LSP codebook / sign-bit layout / escape coding (`[GAP]` per docs) | — |
| **WavPack** | 🚧 ~85% (post-2026-05-18 orphan) — v4 block/metadata/decorrelation/entropy parse + LSB bit-reader + Golomb (base,add) interval + r186 `parse_block` aggregate + r191 `AdaptiveMedians` §3.2 + r194 **first PCM-producing API** `decode_packed_samples_mono` + r199 stereo per-sample decode loop + r201 `EntropyInfo→AdaptiveMedians` bridges + `from_entropy` wrappers + r206 **`WavPackBlock::decode_samples()` one-call PCM composer** chaining `parse_block` + `0x05` entropy expander + `0x0A` typed view finder + `decode_packed_samples_*_from_entropy`; mono/stereo dispatch via new `Flags::is_block_data_mono` (union of bit 2 `mono` and bit 30 `false_stereo`); returns `Vec<i32>` of `block_samples` mono OR `block_samples*2` interleaved stereo; new `UnsupportedBlockFeature` enum (Hybrid/FloatData/Int32Mode/MultichannelMember/Decorrelation/LowLatencyBlock/RobustBlock) + 3 structural errors; 295 tests) + r214 block-level discovery / accessor sweep + r219 **multi-block stream iteration** (`BlockIter<'a>` `Clone`+`FusedIterator`-compliant lazy walker yielding `Result<WavPackBlock>`, fuses on first error; `iter_blocks`/`parse_blocks`/`block_count`/`total_block_samples` (u64-widened); preserves r13 `CkSizeExceedsBuffer`/`Truncated` error split; 337 tests) + r224 **multi-block stream → PCM composer** (`decode_stream(bytes) -> Result<Vec<i32>>` eager walker concatenating per-block PCM in on-disk order + `StreamDecodeIter<'a>` `Clone`+`FusedIterator` lazy iterator + `iter_decoded_blocks` free-fn twin; fuses on first error via r219 fuse + r206 refusal taxonomy; metadata-only blocks (`block_samples==0`) skipped silently; mixed mono+stereo inputs reflect per-block shape; 358 tests); lacks hybrid 0x0B+0x0C / float / multichannel / CRC / decorrelation prediction-loop consumer / encoder | 🚧 scaffold |
| **APE** (Monkey's Audio) | 🚧 r190 Phase 1 + r206 polish — 8-byte `MAC ` magic + decimal-coded version u16 + 5 compression-level enum (1000/2000/3000/4000/5000) prefix parser + `Display` impls (label + raw u16) + 2040-input mutation harness asserting only documented `Error` variants leak from `parse()`; 18 unit + 8 integration tests + standalone-build OK + r225 **`CompressionLevel::ALL`/`iter()` + `From`/`TryFrom`/`FromStr` standard-trait conversions** (`const [CompressionLevel; 5]` in documented order; `Error::UnknownCompressionLevel(raw)` reused for reverse conversion; case-insensitive `FromStr` over the five narrative labels `fast|normal|high|extra high|insane`); per-version header tail (sound params/frame count/seek table/embedded WAV) + IIR coefficients + residual `k` recurrence + range-decoder bounds + channel decorrelation reconstruction all DOCS-GAP | 🚧 scaffold |
| **DTS** (Core) | 🚧 ~40% — frame-sync header + 14↔16-bit pack/unpack + r192 `iter_frames_14bit` + r195 §5.4.1 ABITS/SCALES side-info + Annex D §D.5.6 12-level BHUFF + §D.5.3/§D.5.4 small-Huffman + §D.1.1 RMS_6BIT + §D.1.2 RMS_7BIT + r202 §5.3 SFREQ/AMODE/PCMR typed resolvers + r208 **§C.2.5 `PreCalCosMod()` 544-entry `raCosMod` cosine-modulation matrix** (4-block layout: Block 1 `cos((2i+1)(2k+1)π/64)` 16×16, Block 2 `cos(i(2k+1)π/32)` 16×16, Block 3 `+0.25/(2·cos((2k+1)π/128))` 16, Block 4 `−0.25/(2·sin((2k+1)π/128))` 16; per-block start constants `COS_MOD_BLOCK{1..4}_START` = 0/256/512/528; 232 lib + 217 standalone tests) + r214 **§C.2.4 Sum/Difference Decoding** (front-pair `SUMF || AMODE==3`, surround-pair `SUMS`; single-pair `sum_difference_decode_{i32,f64}` `(L', R') = (L+R, L−R)` with read-old/write-new ordering + wrapping integer arithmetic; subband-pair `*_subband_pair_*` outer-loop variants; dispatch predicates `front_sum_difference_required` / `surround_sum_difference_required`; new `Error::SumDiffLengthMismatch`; 256 lib + 241 standalone tests) + r223 **§C.2.3 Joint Subband Coding copy+scale** (`joint_subband` module — `joint_subband_decode_range_{i32,f64}` for high-end-subband copy + per-subband scale per §C.2.3 inner loop; `joint_source_channel(joinx)` one-based→zero-based resolver (`None` for `JOINX[ch]==0` disabled case); `joint_subband_required(joinx)` dispatch predicate; new `Error::JointSubbandShapeMismatch`; i32 path uses `wrapping_mul` per spec's C `int` semantics; 276 lib + 8 doctests); lacks subframe walker + §5.4 polyphase synthesis (blocked on §D.8 raCoeff* taps #1357) + DIALNORM | — |
| **aptX** (classic + HD) | 🚧 ~70% — 4-band QMF + ADPCM; bit-exact NDA-blocked + r189 RFC 2361 §A.24 `WAVE_FORMAT_TAG_APTX = 0x0025` IANA tag + `CODEC_ID_STR = "aptx"` registry (lets RIFF containers route 0x0025 → clean NotImplemented) | — |

</details>

<details>
<summary><strong>Video</strong> (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **MJPEG** | ✅ ~97% — baseline + progressive 4:2:0/4:2:2/4:4:4/grey + 12-bit YUV (baseline + r183 SOF2 P=12 progressive) + SOF9 arithmetic + lossless SOF3 + RFC 2435 RTP/JPEG depacketization + r190 §G.1.1 SOF2 4-component CMYK / YCCK progressive at P=8 + r197 8th cargo-fuzz target `arith_decode` for SOF9 Q-coder per-iteration coverage + r215 RFC 2435 §3.1.7 opt-in restart-interval-aligned RTP/JPEG packetization + r221 **4-component lossless SOF3 P=8 (Adobe APP14 CMYK) decode** end-to-end (predictor 1..=7 bit-exact, Adobe APP14 transform applied like lossy CMYK path, DRI/RSTn + Pt=2 quantisation supported, P≠8 rejected; 193 tests) | ✅ ~96% — baseline + progressive + lossless SOF3 grey/RGB + DRI/RSTn + non-zero point transform Pt 0..15 + r193 public 4-component CMYK encoder + r221 **4-component lossless SOF3 P=8 ENCODE** (`encode_lossless_jpeg_cmyk` + `_with_opts` w/ Adobe APP14 + DRI/RSTn + Pt) |
| **FFV1** | 🚧 ~83% — RFC 9043 decoder + demux + decode_frame driver (**YCbCr v3-default 4:2:0 bit-exact end-to-end** against expected.raw all 18 432 Samples; RGB line-major still open) + r214 **§4.6.6 per-slot state-buffer fix** (Cr divergence closed: state keyed by §4.6.6 slot not by resolved Set; Cb+Cr share chroma slot's persistent per-context state, only §3.8.2.2.1 run-mode resets per Plane; new `reconstruct_plane_with_state`/`encode_plane_with_state` pub(crate) APIs; 404 tests) + r179 `coder_type==2` alternative state-transition table wired through decode + encode + r208 Golomb-Rice chroma-planes decode_frame cursor fix + r220 **§4.6.6 per-slot state-buffer rule extended to RGB driver** (`colorspace_type == 1`, coder_type ∈ {1,2}; mirrors r214 YCbCr fix; `RangePlaneState` lifted out of `PlaneLineState` into per-slot `Vec<Option<RangePlaneState>>` lazily filled on first slot touch; first end-to-end bit-exact RGB-fixture decode milestone on `v3-rgb-bgr0` slice 0; 405 integration tests) | 🚧 ~96% — Slice Footer + Slice Header + Golomb-Rice primitives + frame-level Golomb-Rice + YCbCr encoder + r164 range-coded SliceContent encoder + r190/r193 §4.7 RGB + RCT encode + r196 unified `encode_frame` dispatch helper + r202 §4.2 Parameters + §4.1 Quantization Table Set cascade encoder; lacks §4.2.14 tail (#904) |
| **MPEG-1 video** | 🚧 ~42% — sequence/GOP/picture/slice + macroblock walk + intra-DC + §2.4.3.7 dct_coeff walker + §2.4.4 dequantiser + r185 §A 8×8 IDCT + IEEE P1180/D2 conformance + r194 §7.3 `mpeg2_inverse_scan` + r202 §6.2.6 MPEG-2 `block(i)` driver (`mpeg2_decode_block` chains §7.2.1 DC prelude → §7.2.2 residual VLC walker w/ FIRST/NEXT alternation → §7.3 inverse scan → §7.4 inverse-quant → §A 8×8 IDCT into one bitstream→`f[y][x]` entry; 677 tests) | 🚧 scaffold |
| **MPEG-2 video** | 🚧 ~65% — §6.2.x sequence/GOP/picture/slice + macroblock walk + §7.6.3.x PMV + §7.6.4-8 forming-predictions/combine/add-saturate + r165 §7.6 driver + r179 §7.4 inverse-quantisation + r185 §A 8×8 IDCT + r192 §7.2.2 residual VLC walker + r194 §7.3 `mpeg2_inverse_scan` + r199 §7.2.1 **intra-block DC prelude** + r206 §6.2.5/§6.2.6 macroblock-block driver + r213 §6.2.4 slice-level macroblock-header walker `walk_slice` + r221 **§7.6.6 skipped-macroblock specification** (`SkippedMacroblockContext`/`SkippedMacroblock`/`SkippedMotionVector` + `describe_skipped_macroblock`; P field/§7.6.6.1, P frame/§7.6.6.2, B field/§7.6.6.3, B frame/§7.6.6.4; same-parity reference, direction inheritance, MV source, `reset_pmv` per §7.6.3.4; I-picture rejection; B-picture rejection when previous_direction is Skipped; 638 lib tests) + r223 **§6.2.5.1 macroblock_modes() tail wired into slice walker** (frame_motion_type Table 6-17 / field_motion_type Table 6-18 / dct_type read between `macroblock_type` and `quantiser_scale_code` per §6.2.5 syntax order; `SliceWalkContext` gains `picture_structure` + `frame_pred_frame_dct`; `MacroblockRecord` gains `motion_type` + `dct_type`; fixes latent ordering bug; 652 lib tests); next: §6.2.5 motion_vectors(s) wiring | 🚧 scaffold |
| **MPEG-4 Part 2** | 🚧 ~64% — I-VOP intra + inter texture + §6.2.5 video_packet_header + §7.8.7.3 GMC + r182 §7.6.2.1 half-sample bilinear + r190 §7.6.2.2 quarter-sample + Table 7-13 chroma MV reduction + r193 §7.6.9.5.2 B-VOP direct-mode MV derivation + r195 §7.6.9.5.3 B-VOP luminance prediction-block + r201 §7.6.5 chroma MV derivation `MVDCHR` + r206 §7.6.1.6 vector padding + r212 §7.6.9.5.3 + §7.6.9.4 B-VOP chrominance motion-compensation plane + r217 **§7.6.5 / Figure 7-34 spatial MV-predictor candidate grid** (`MvGrid`/`gather_mv_predictor_candidates` + `MbMv ∈ {Absent, OneMv, FourMv}` + `MbMvRecord` 4-element transparency mask; VOP/video-packet/GOB boundary handling; spec worked example pinned `MV1=(-2,3) MV2=(1,5) MV3=(-1,7) → (-1, 5)`; 587 tests) + r222 **§7.3 VOP reconstruction `p[y][x] + f[y][x]` w/ step-3 [0,255] clip** (per-pixel reconstruction module closing the decoder loop between texture decode and the picture buffer; saturating clip matches §7.3 step 3); lacks §6.2.6.2 MV-body parser wiring + MC driver + encoder | 🚧 scaffold |
| **Theora** | 🚧 ~48% — §6.1–§6.4 setup-header + Appendix B.2/B.3 VP3-default tables + §6.4.x quant + DCT-token Huffman + §7.1–§7.5 frame walk + r160 §7.5 motion vectors + r179 §7.7.1 EOB Token decode + r185 §6.4.1 LFLIMS + r191 §7.7.2 Coefficient Token Decode + r195 §7.7.3 DCT Coefficient Decode driver + r201 §7.8.1 DC predictor compute + r206 §7.8.2 Inverting DC Prediction driver + r212 §7.9.2 Dequantization + r218 **§7.9.1 Predictors** (`compute_intra_predictor` §7.9.1.1 constant-128 fill; `compute_whole_pixel_predictor` §7.9.1.2 clamped reference-plane copy; `compute_half_pixel_predictor` §7.9.1.3 two-sample averaged `>>1`; typed `ReferencePlane` view + 3 reject variants + `split_half_pixel_motion_vector` §7.9.4 truncate-pair helper; 366 tests) + r223 **§7.9.3 Inverse DCT (1D + 2D)** (`inverse_dct_1d` §7.9.3.1 55-step procedure verbatim with Table 7.65 constants `C3/S3=54491`, `C4=46341`, `C6/S6=25080`, `C7/S7=12785`; `inverse_dct_2d` §7.9.3.2 row/column driver with `(X + 8) >> 4` final rounding; 379 tests); lacks §7.9.4 reconstruction driver + §7.10 loop filter | 🚧 scaffold |
| **H.263** | 🚧 ~89% (post-2026-05-18 orphan) — §5.1-§5.4 baseline + §6 IDCT/MV/half-pel/INTER + Annex J §J.3 deblock + Annex I AIC + Annex D UMV + Annex F 4-MV + OBMC + §5.1.4 PLUSPTYPE + Annex K §K.2 SS + r187/r192 §I.3 AIC reconstruction pipeline + r196 §I.2/§I.3 AIC MB-grid driver wiring + r202 `decode_picture_layer` PLUSPTYPE entry-point + r208 **§5.1.4.4/§5.1.4.5 PLUSPTYPE inherited-state stream driver** (`decode_picture_layer_with_inherited(data, ref, options, inherited) -> DecodePictureOutcome { frame, inherited }` retains §5.1.4.4 OPPTYPE snapshot across pictures so a UFEP=000 PLUSPTYPE header is decodable; `InheritedExtendedState` grown to full snapshot `source_format/umv/advanced_prediction/advanced_intra/deblocking`; `from_opptype` captures from parsed `Opptype` (refused-mode bits dropped); §5.1.4.5 rule 1 UMV/AP forced off in I-pictures after inheritance; rule 3 baseline-PTYPE resets snapshot; legacy `decode_picture_layer` is thin wrapper; 385 tests) + r214 §4.2.1/§5.1.5 custom-source-format GOB-layout driver + r220 **§K.2 `SliceHeaderContext::from_picture_layout(layout, sss, cpm, rru)` shape adapter** (plugs the missing wiring between canonical `PictureLayout` and §K.2 slice-layer parser; `SliceStructuredSubmode::rectangular` gates §K.2.8 SWI; `cpm` drives §K.2.4 SSBI + §K.2.6 SEPB2 threshold; `rru` selects Tables K.2/K.3 right-hand columns; 401 tests) + r226 **§K.2.1 SSTUF slice-header stuffing skipper** (Annex K SS slice-header byte-stuffing consume rule); lacks Annex K driver + PB-frames | 🚧 scaffold |
| **H.261** | ✅ ~98% — I+P QCIF/CIF + integer-pel + loop filter + BCH FEC + Annex B HRD + RFC 4587 RTP + RFC 3550 RTCP + r189 §6.2.1 SDP offer/answer + r198 3rd cargo-fuzz target + r204 4th target `parse_rtp_payload` + r211 **Annex A IDCT-accuracy conformance test** (§A.1 LCG-driven 10000 8×8 pel blocks × all three `(L=256,H=255)/(L=H=5)/(L=H=300)` datasets; f64 forward §A.2 + reference §A.4 IDCT coded from §3.2.4 equation; §A.3 clip to [-2048,+2047]; §A.7 peak/per-pel MSE/overall MSE/per-pel|mean|/overall|mean| asserted against thresholds — measured 2-5 orders of magnitude inside; §A.8 all-zero + §A.9 sign-flipped rerun) + r216 **5th cargo-fuzz target `parse_sdp_fmtp`** (drives `parse_rtpmap`/`parse_fmtp`/`H261FmtpParams::parse_value`/`negotiate_answer`; 5-mode driver + 10-seed corpus per RFC 4587 §6.2 + RFC 2032 fallback + non-H.261 reject; non-UTF-8 → U+FFFD lossy decode; +19 stable-CI mirror tests) + r222 **Annex D §D.2/§D.3 still-image sub-image transform helpers** (encoder-side hooks for low-frame-rate transmission per Annex D; matches §D.2 still-image hint flag wiring + §D.3 sub-image-update model; spec-shape only — caller still drives rate-control) | ✅ ~98% — spiral+diamond ME + GQUANT-from-bitrate + BCH framing + RTP wrap + RTCP compound build/parse; 45 dB at 64 kbit/s QCIF |
| **MS-MPEG-4** (v1/v2/v3) | 🚧 ~44% — clean-room scaffold + r202 `Macroblock4MvDecoder` 4-MV-per-MB bitstream tests (4 integration tests pin picture-corner rule-4 + within-MB candidate chaining + four-zero-MVD rigid-motion + parallel-reader cross-check against `predict_block_mv`; 80 integration tests) + r181 `GFamily` accessors + r185 Figure 7-34 MV-predictor walk + r191 1-MV predictor routed through `predict_block_mv(Block::TopLeft, …)` + r196 §7.6.5 4-MV-per-MB batch predictor + r208 **4-MV neighbour-MB bordering-cell picker** (`bordering_block_of_neighbour` + `pick_neighbour_mv_from_4mv` const-fns + `NeighbourDirection` enum close long-standing "caller picks right cell from neighbouring MB" gap in `MacroblockCandidates`; (current-block, direction) → bordering-block table from Figure 7-34's 4 sub-diagrams: TL takes all 3, TR takes Above+AboveRight, BL takes Left, BR takes nothing; 309 tests) + r214 4-MV neighbour-state resolver + r221 **`Macroblock4MvDecoderNeighbours`** (`NeighbourSet`-driven stateful analogue of r196's `Macroblock4MvDecoder`; predict/commit/finalise shape but routes `predictor_for(block)` through `resolve_block_candidates`, so a 4-MV-coded neighbour's bordering 8×8 cell is picked per current-MB block per Figure 7-34; closes r196's "collapse-to-one-Option<Mv>-per-direction" assumption for 4-MV neighbours; 331 tests). Still lacks G0..G3 primary canonical-Huffman bit-length array + alt-MV VLC + 4-MV MCBPC | — |
| **H.264** | 🚧 ~80% — I/P/B + 4:2:0/4:2:2/4:4:4 + CAVLC + CABAC + DPB + 44 SEI types + fuzz-hardened + r183 SEI type 46 + r187 §8.2.1 POC i64-staged + r192 §5.2.4.1.1 strict avcC parser + High-family extension trailer + r194 §7.3.5.3.1 CAVLC call-contract guards + r200 Annex G MVC SEI types 39+43 + r207 Annex G MVC SEI type 41 + r213 §G.13.1.5/§G.13.2.5 Annex G MVC SEI type 40 `multiview_acquisition_info` + r219 **§7.3.1 NAL extension header parsing** (typed `NalUnitHeaderMvcExtension` §G.7.3.1.1 / `NalUnitHeaderSvcExtension` §F.7.3.1.1 / `NalUnitHeader3dAvcExtension` §I.7.3.1.1; `parse_nal_unit` dispatches `nal_unit_type ∈ {14,20,21}` on `svc_extension_flag`/`avc_3d_extension_flag`; new `TruncatedExtensionHeader` variant; type-21 → MVC fall-through honoured; 1108 tests) + r226 **Annex H 3D-AVC SEI type 51 `three_dimensional_reference_displays_info`** (§H.13.1.10 / §H.13.2.10 — typed `ThreeDimensionalReferenceDisplaysInfo` payload + parser); lacks MBAFF, SVC/3D/MVC body | 🚧 ~83% — I+P (1MV/4MV, ¼-pel) + B + CABAC at all chroma layouts + Trellis-quant RDOQ-lite (1227 tests); ffmpeg PSNR_Y 44.20 dB |
| **H.265 (HEVC)** | 🚧 ~54% — VPS+SPS+PPS bodies + scaling-list + scan + §9.3 CABAC engine + slice header through §7.3.6.3 pred_weight_table + r182 §7.3.6.2 ref_pic_lists_modification() + r190 §7.4.8 inter-RPS-prediction typed builder + r193 §7.3.2.3.1 `PpsExtensionFlags` + r195 §9.3.4.2 binarization scaffold + r200 §9.3.4.2.4 `coded_sub_block_flag` ctxInc + §9.3.4.2.2 Table 9-49 `split_cu_flag`/`cu_skip_flag` ctxInc + r207 six §9.3.4.2/Table 9-48 closed-form ctxInc + r212 §7.3.2.2.1 SPS extension-flag typed decode + r218 **§7.3.4 `sao()` per-CTU binarization shapes** (Tables 9-43/9-48 — `sao_merge_flag_ctx_inc`, `sao_type_idx_ctx_inc` w/ bypass bin-1, `sao_offset_abs_tr_cmax(bit_depth)` `(1<<min(bd,10)−5)−1`, FL `sao_offset_sign`/`sao_band_position`/`sao_eo_class` cmax+nbits; 280 tests) + r223 **§9.3.4.2.6 + §9.3.4.2.7 coeff_abs_level_greater{1,2}_flag ctxInc state machine** (`Greater1State` per-TB walker carries `(ctx_set, greater1_ctx, seen_any_subblock, has_last_greater1_flag)` across §7.3.8.11 sub-block loop; eq. 9-56/9-57 init / 9-58 ladder bump / 9-59/9-60 ctxInc; eq. 9-61/9-62 greater2 reads same sub-block's ctxSet; `COEFF_ABS_LEVEL_GREATER_X_FL_CMAX=1` Table 9-43 shape; +14 tests = 294); lacks sig_coeff_flag (blocked on Table 9-50 i=15 cell #1414) + coeff_abs_level_remaining + coeff_sign_flag + residual/IDCT | 🚧 scaffold |
| **H.266 (VVC)** | 🚧 ~70% — 4:2:0 IDR intra + ALF/SAO/CC-ALF + P/B merge+skip + HMVP + MMVD + CIIP + BCW + BDOF + GPM + AMVR + HBD + DMVR + affine + PROF + AMVP + SbTMVP + r181 VPS + r193 §7.3.10.10 `amvr_flag`/`amvr_precision_idx` CABAC reader; 1106 lib tests | 🚧 ~90% — forward CABAC + DCT-II + SAO/ALF/cu_qp_delta + MTT BT+TT RDO + P+B + sub-pel MC + multi-ref DPB + weighted bi-pred + r190 §7.3.11.7 `encode_non_merge_inter_pre_residual` + r195 encoder-side §7.3.10.10 `amvr_enc` + r201 §7.3.10.5 `bcw_idx_enc` encoder mirror + r207 §7.3.10.5 multi-CP-MV affine MVD encoder dispatcher + r213 **encoder-side §7.3.11.7 non-merge inter pre-residual affine+AMVR + affine+AMVR+BCW composite dispatchers** (`encode_non_merge_inter_pre_residual_affine_with_amvr` chains r207 per-CP affine 1-9 + r195 §7.3.10.10 AMVR 10; `_with_bcw` adds r201 §7.3.10.5 BCW 11; byte-for-byte degenerate to translational siblings under `MotionModel::Translational`; 1142 tests) + r224 **reader-side `_with_amvr` + `_with_amvr_and_bcw` composite walkers** (`read_non_merge_inter_pre_residual_with_amvr` chains r219 reader cascade with r193 `read_amvr_inter_gated` returning `(NonMergeInterPreResidualDecision, AmvrDecision)`; `_with_amvr_and_bcw` further chains r126 `read_bcw_idx_gated` returning `(decision, amvr, bcw_idx)`; debug-asserts the same caller-conformance contracts the encoder dispatchers enforce; 11 new lib tests pin encoder→decoder symmetry across both arms; 1153 tests) |
| **VP6** | 🚧 r18 — §13 static tables + §3 RawBitReader + §7.3 BoolCoder + r198 §13.2.1 DC arithmetic + r204 §13.3.1 AC coefficient arithmetic decoder + r211 `fetch_prediction_block_clamped` §11.5-derived edge-clamped MC fetch + r217 **§13.3.3.1 `decode_ac_zero_run` BoolCoder traversal** (Figure 16 / Table 38 eight-node ZRL walk `>4`/`>2`/`>1`/`>3`/`>8`/`>6`/`>5`/`>7` reading one `B(prob)` per node; `>8` escape reads six LSB-first extrabits as `(RunLength − 9)`; output range `1..=72`; third BoolCoder layer after r198 DC + r204 AC; 389 tests) + r226 **§11.1 motion-vector component arithmetic decoder** (BoolCoder MV-component walker — magnitude/sign reader for inter-MB displacement) | 🚧 scaffold |
| **VP8** | ✅ 100% | ✅ 100% |
| **VP9** | 🚧 ~44% — §6.2 walk + §9.2 Bool decoder + §6.3 compressed-header primitives chain complete + §6.4.24 coeff + §8.6 dequant + §8.7 inverse transforms + §8.5.1 intra pred + r199 §6.3.12 `frame_reference_mode` + r205 §6.3.16 **`mv_probs()` compressed-header outer sweep** (65/69-cell walk over 9 `mv_*_prob[]` arrays in three unconditional phases + conditional HP tail gated on `allow_high_precision_mv`; threads §6.3.17 `update_mv_prob` per-cell primitive; new `MvProbs` aggregate + `defaults()` ctor; 9 §10.5 default tables + 5 §3 MV constants verbatim-transcribed; 415 lib tests; §6.3.1→§6.3.18 primitives chain complete) + r215 **§6.4.1 `get_tile_offset` + §6.4.2 `decode_tile` superblock-row driver** (`get_tile_offset(tile_num, mis, tile_sz_log2) -> u32` per spec L2335-2338; `decode_tile(...)` outer `r ∈ [Start,End)` step-8 w/ `clear_left_context()` per row + inner `c ∈ [Start,End)` step-8 → existing `decode_partition(r,c,BLOCK_64X64)`; 440 lib + 460 total tests) + r225 **§6.3 inter-arm outer dispatch — `parse_compressed_header_inter` entry point** (composes §6.3.9 `read_inter_mode_probs` → §6.3.10 `read_interp_filter_probs` (SWITCHABLE-gated) → §6.3.11 `read_is_inter_probs` → §6.3.12 `frame_reference_mode` firing §6.3.18 `setup_compound_reference_mode` on non-SingleReference arms → §6.3.13 `frame_reference_mode_probs` → §6.3.14 `read_y_mode_probs` → §6.3.15 `read_partition_probs` → §6.3.16 `mv_probs` driving §6.3.17 `update_mv_prob` per cell + HP tail; intra-shared prefix §6.3.1/§6.3.2/§6.3.7/§6.3.8 factored into a shared helper); lacks §6.4 outer `decode_tiles( sz )` + §6.2.5 inter-frame branch + §6.4.4 `decode_block_apply` leaf wire-up + §8.4 loop filter | 🚧 scaffold |
| **AV1** | 🚧 ~94% — decoder feature-complete + **standalone `decode_av1` public entry** + r203 §6.7.2 Y-only (monochrome) on the dyn pixel driver + r207 multi-SB dyn-Y dispatch up to 128×128 (1528 tests + integration roundtrips) | 🚧 ~32% encoder — pixel-space YUV→IVF driver + 14-mode intra picker + §7.13.3 forward 2D dispatcher + WHT lossless + forward quantize + r194 §7.11.5.3 UV_CFL_PRED + r196 `base_q_idx > 0` lossy quant + r197 rectangular extents + r203 monochrome encoder dyn driver + r207 **multi-super-block tiling on monochrome dyn driver** (lifts Y-only extent ceiling 64→128 via §5.11.1 `for r/c += sbSize4` walk with `sbSize=BLOCK_64X64`; each SB origin a fresh `BLOCK_64X64`-rooted `EncodeNode` tree; edge SBs swallow OOB quadrants via r234 `EncodeNode::dummy_oob` + §5.11.4 line-1 short-circuit; new `MonoYFrameMultiSb` + `EncodedFrameDynYMultiSb` + `encode_intra_frame_y_dyn_multi_sb{,_with_q}` + `MAX_DIM_Y_MULTI_SB=128`; bit-exact across 10 extents incl. partial-coverage edges 72×64/104×72 + 2×2 grids up to 128×128 + lossy q∈{1,32,200}) + r214 **4:2:0 YUV multi-super-block dyn driver** (`Yuv420FrameMultiSb` / `EncodedFrameDynYuvMultiSb` types + `encode_intra_frame_yuv_dyn_multi_sb{,_with_q}` entries + `MAX_DIM_YUV_MULTI_SB=128`; decoder `decode_frame_dyn` lifts cap + dispatches per-SB `decode_partition_node(... BLOCK_64X64)` when `w>64 || h>64`; ≤64² byte-identical to single-root path; 1537 lib + 67 integration tests) + r223 **§8.2.6 post-renormalisation invariant probes** (both `SymbolDecoder::renormalize` and `SymbolWriter::write_symbol` now enforce `32768 ≤ SymbolRange ≤ 65535` and `SymbolValue < SymbolRange` via a typed `Error::SymbolStateInvariantBroken` rather than leaving the codec in an undefined state; +6 lib tests = 1543; no-op on conformant input). Lacks rectangular **TX_SIZE family** (TX_4X8/8X4/8X16/16X8) + §5.11.18 inter mode_info + RD picker |
| **Dirac / VC-2** | ✅ ~95% — VC-2 LD+HQ intra + Dirac core-syntax intra/inter + OBMC + 7 wavelets + 10/12-bit + bit-exact intra fixtures + r165 fuzz oracle + r190 Criterion bench + r195 row-major slice driving + r201 §12.4.4 `extended_transform_parameters` parser (345 tests) | 🚧 ~97% — HQ+LD intra + Dirac core-syntax + adaptive sub-pel + 2-ref bipred + post-OBMC refinement + rate-control + r193 inter-encoder fuzz + r206 §12.4.4 VC-2 v3 symmetric-default sequence-header roundtrip + r212 **VC-2 v3 asymmetric `extended_transform_parameters` emission + decoder-rejection tests** (`ExtendedTransformOverride { wavelet_index_ho, dwt_depth_ho }` + `with_extended_transform_override()` builders on `EncoderParams` + `LdEncoderParams`; emits `asym_transform_index_flag` per §12.4.4.2 + `asym_transform_flag` per §12.4.4.3 then gated `wavelet_index_ho`/`dwt_depth_ho`; bitstream spec-conformant for future asymmetric IDWT; current decoder surfaces `PictureError::AsymmetricTransformUnsupported`; 4 tests pin override vs symmetric default; closes the r201→r206→r212 §12.4.4 lineage) + r223 **§14.2 fragment-header parser** (`src/fragment.rs` setup-fragment 8-byte (`fragment_slice_count==0`, transform parameters follow) + data-fragment 12-byte (`(x_offset, y_offset)` carry) parser; `ParseInfo::is_fragment_parse_code()` §10.5.2 Table 5 bit predicate `(parse_code & 0x0C) == 0x0C`; `FragmentError::Truncated { needed, available }`; 369 tests) |
| **AMV video** | 🚧 scaffold (orphan rebuild post-audit 2026-05-18; clean-room re-implementation pending) | 🚧 scaffold (orphan rebuild post-audit 2026-05-18; clean-room re-implementation pending) |
| **ProRes** | ✅ ~96% — RDD 36 entropy + 8/10/12-bit + 4:4:4:4 alpha + interlaced + RAW refused; ffmpeg interop 60-68 dB + cargo-fuzz + r185 `idct8x8_dc_only` fast path + r195+r201 SHA-256 lockstep pin on 9 fixtures + r206 SHA-256 lockstep on 128×128 interlaced apcn + FIPS 180-4 §B.1/§B.2 self-check (263 tests) | ✅ ~97% — RDD 36 all 6 profiles + interlaced + alpha + perceptual quant matrices + r193 ffmpeg cross-decode + r212 encoder-output SHA-256 lockstep pin across every public encoder entry point + r218 **SHA-256 pin extended onto 10/12-bit encoder paths** (RDD 36 §7.5.1 forward level-shift; `encode_frame_with_depth` `apcn`/`ap4h` × 10/12-bit pins on depth-distinct gradient inputs with round-trip + `assert_ne!` depth-flip guards) |
| **EVC** (MPEG-5) | 🚧 ~90% — NAL + SPS/PPS/APS + §9.3 CABAC + §8 intra + DCT-II + P/B inter + RPL + HMVP + DPB + ALF + DRA + IBC §8.6 + r187 §8.9.7 `DraChromaDerived` + r193 §8.9.8 `DraJoinedScaleFlag=1` joined-chroma-scale + r195 §7.4.3.1 SPS-signalled `ChromaQpTable` + r201 `chroma_qp_table_for_sps` three-way SPS adapter + r207 `derive_dra_chroma_state_for_sps` SPS→§8.9.6 chroma-chain adapter + r213 §8.5 AMVR helper trio + r218 **§7.4.7 MMVD distance / sign / offset derivation + §9.3.4 ctxInc** + r223 **§8.5.2.3.9 bipred MMVD offset distribution (eqs. 591-616)** (`inter::mmvd_dist_scale_factor` eq. 599/604 POC scaling `(\|num\| << 5) / \|den\|`; `inter::mmvd_apply_bipred_offset` eqs. 591-616 three-way magnitude split — `Abs(L0)==/>/<Abs(L1)`, opposite-POC-sign `mMvdL1` negation per eqs. 607-610, one-list-active "Otherwise" branch per eqs. 611-612, accumulation under shared `wrap16`; 482 tests); lacks Main-profile toolset (BTT/ADMVP/EIPD/ATS/affine + thread `sps_amvr_flag`/parsed `amvr_idx` into helpers) + #1278 §8.9.8 eq 1398-1409 tableNum==0 branch ambiguity | — |
| **HuffYUV** / FFVHuff | ✅ ~97% — HFYU + FFVH FourCCs + 6 predictors + 8-bit only + interlaced field-stride=2 + fast-LUT decoder + SWAR 8-byte gradient post-pass + r181 YUY2 LEFT macropixel-step branch-free decoder + r196 cargo-fuzz `encode_roundtrip` target + r202 YUY2 Median tail-loop dead-branch strip + r208 **LEFT-helper dedup vs `predict::*`** (decoder-local `inverse_left_per_channel` was byte-for-byte duplicate of `predict::inverse_left_row`; `inverse_yuy2_left{,_range}` were thin pass-throughs into `predict::inverse_yuy2_left_macropixel` since r181 macropixel-step rewrite; YUY2 + RGB24 + RGB32 decode paths now re-pointed at predict-side helpers; 164 tests) + r214 macropixel-step YUY2 Huffman-decode body | ✅ ~96% — full encoder symmetry × YUY2/RGB24/RGB32 + v1.x + v2.x ClassicV2/CustomV2 + r181 YUY2 LEFT forward + r186 `forward_rgb_left_subtract_linear` + r202 encoder-side dead-branch parity + r221 **macropixel-step YUY2 Huffman-ENCODE body** (encoder mirror of r214 decoder rewrite; per-byte `match byte_idx & 3` hoisted to compile-time 4-byte macropixel-step `lookup_code(slot1)→write→slot2→write→slot1→write→slot3→write`; 176 tests) |
| **Lagarith** | ✅ ~95% — all 11 wire types + modern range coder + legacy adaptive-CDF + Fibonacci-Zeckendorf prefix + JPEG-LS Median + G-pivot decorr + zero-run RLE + pair-packed 513-entry CDF + modern RGB(A) first-column predictor Rule B + r198 deeper channel-body fuzz + r211 lazy alpha-plane decode in `decode_arith_rgba` for `Bgr24` host + early `PixelFormatMismatch` reject on `Yv12`/`Yuy2` for RGB-family frames + r216 **packed-RGB(A) pack-loop branch hoist** (per-pixel `match pixel_kind` lifted out of hot loop in `decode_arith_rgba`/`decode_arith_rgb`/`decode_legacy_rgb`/`decode_solid`; `decode_solid` swaps per-pixel push for `vec!`+chunked-write; output bytes unchanged; 221 tests + 7 ffmpeg pins) + r222 **frame-level type-1 (uncompressed) encoder size-guard wrappers** (4 public wrappers around the modern arithmetic encoders that pre-budget the output buffer against the type-1 fallback ceiling, surfacing a typed overflow error before the codec body runs) | 🚧 ~76% — encoder for SOLID/RGB/RGBA/YV12/YUY2/legacy-RGB + spec/02 §5 Step-A + Step-B + Step-C `freqs[]` cache + r135/r138/r141 modern + per-channel header-form selection; byte-exact vs proprietary encoder Auditor-blocked |
| **Ut Video** | ✅ ~97% — 5 native FourCCs × 4 predictors + RGB inter-plane decorrelation + LUT-accelerated canonical Huffman + slice-parallel decode (5.63× at 720p) + criterion baseline + r186 `Decoder` trait factory + r196 Gradient/Median per-row branch-hoist + r203 **row-strided None + Left predictor refactor** (single shared stride-aware row driver replaces two near-duplicate per-predictor inner loops; tests/round16_predictor_row_stride.rs covers contiguous + odd-stride + tail-partial-row equivalence vs r186 baseline; +468 test LOC; observable byte-identical) + r215 **content-adaptive trait-path predictor heuristic** (`choose_predictor(plane, width, height) -> Predictor` samples `HEURISTIC_SAMPLE_ROWS=8` leading rows under each candidate, picks lowest `Σ count[s]·log2(N/count[s])` per spec/05 §2.2; `UtVideoEncoder.predictor_override: Option<Predictor>` + `set_predictor`/`predictor_override`; direct API unchanged; 258 tests) | ✅ ~96% — slice-parallel encode (3.28×) + content-fixture corpus + r161 cargo-fuzz oracle |
| **MagicYUV** | ✅ 100% — r212 **batched raw-mode bit unpacker `bitreader::unpack_raw_bits_to_u16`** (per-pixel→per-`floor(56/bits)`-pixel refill; 640×480 10-bit raw slice drops 300k→55k refill branches; bit-stream equivalent vs per-pixel reference across 200/4096-sample sweeps; 86 tests) | ✅ 100% — r206 `examples/profile_magicyuv.rs` samply-friendly flat profile driver + r224 **per-plane scratch reuse in `build_slice_residuals_u{8,16}_into`** (Fixed: src→tail of residuals + in-place predictor, per-slice mallocs 1→0; Dynamic: trial_a/trial_b/loser-slot walk + winner extend-copy, per-slice mallocs 3→0; 1280×720 M8RG/dynamic frame drops Vec<u8> malloc count 90→2; +5 tests pinning fresh-Vec-per-call reference oracle) |
| **Cinepak** (CVID) | ✅ ~98% — frame header + multi-strip + V1/V4 codebooks + intra/inter + grayscale + Sega FILM demuxer + Saturn/3DO deviant + r181 codebook_chunk_apply + r192 `decode_vector_chunk` cargo-fuzz target + criterion benches + r196 `decode_multi_frame` cargo-fuzz target + r202 named seed-corpora for `codebook_chunk_apply` / `decode_vector_chunk` / `decode_deviant_frame` (27 deterministic seeds + in-memory verification) | ✅ ~98% — stateful encoder + rolling codebooks + RDO + LBG + 3-axis grid picker + bitrate-target rate-control + keyframe-interval (34.18 dB PSNR; decode 4.4 GiB/s) + r215 **`EncoderOptions::vintage_compat`** (vintage Win+Mac player compat: rejects `strip_count>3`; inter chunk-omission falls back to header-only chunks `chunk_size=0x0004` keeping wire stream V4-then-V1 order each strip; +9 tests) |
| **SVQ1/SVQ3** (Sorenson) | 🚧 r11 — SVQ1 framework + r194 L=0..L=3 codebook payload + r197 L=4/L=5 ABSENCE + r203 **SVQ1 saturating-clip + bit-mask helper LUTs** (`build.rs` extension stages `clip_lut.csv` 769-row table + `MANIFEST-02.sha256` integrity; `svq1_helper_luts.rs` typed-LUT accessors for `saturating_clip` + `mask_bits`; +237 LOC LUT module + +175 LOC build extension; `tables/clip_lut.meta` binary-disassembly-tier provenance YAML only) + r224 **SVQ3 sub-pixel thirdpel interpolation arithmetic** (`svq3_mc` module — `thirdpel_interpolate_1d(a,b) = ((2A+B+1)*0x2AB)>>11` + `thirdpel_interpolate_2d(a,b,c,d) = ((4A+3B+3C+2D+6)*0xAAB)>>15` const fns; `THIRDPEL_2D_WEIGHTS = [[4,3],[3,2]]` 2×2 verbatim; `stored_sixths_base(precision)` 6/3/2 Fullpel/Halfpel/Thirdpel resolver + `is_aligned_to_precision_base` predicate; rounding direction left unspecified pending wiki-spec clarification); lacks intra-vs-inter ordering + stage interleave + SVQ3 MV-VLC + #1256 svq3.c attribution scrub | — |
| **Indeo 3** (IV31/IV32) | 🚧 r14 — clean-room codec-frame header + bitstream header + spec/02 picture-layer + spec/03 macroblock-layer + spec/04 VQ codebook + spec/06 byte-level entropy + spec/07 output-reconstruction + four cell-shape kernels + spec/02 strip-context array + spec/03 per-cell sub-array wiring + r181 spec/05 §1 mc_table + r186 spec/05 §2.2/§2.3/§3.3/§3.4 packed-MV bit-layout + r196 spec/05 §5.4 cell-position decoding + r202 spec/05 §4.2 ping-pong bank-selection + r208 **spec/05 §4.1 strip pixel-buffer arena geometry typing** (`MC_ARENA_LEN = 0x8020` + `MC_ARENA_ROW_STRIDE = 0xb0` cross-check; `STRIP_PIXEL_BUFFER_ALIAS_COUNT = 6` + `StripPixelBufferAlias::{Base0..Base5}` enum + `from_index`/`as_index`/`slot_relative_byte_offset`; `strip_region_bytes(plane_height_pixels)`; `StripArenaCapacity::for_plane_height` (boundary `186`; §4.1 example height 240 flagged not-fitting); `base_pointer_aliases_equal` invariant probe; 341 tests) + r214 **spec/05 §4.3 source-pointer plumbing** (per-plane decoder→cell-state dispatcher stack-frame hand-off pinned at `IR32_32.DLL!0x10006638..0x10006641`; 6 named-byte-offset constants `DECODER_ARG_{SRC,DST}_SLOT_OFFSET` = 0x54/0x58, `DISPATCHER_SCRATCH_{SRC_DATA,DST_DATA,EXTRA}_OFFSET` = 0x24/0x28/0x38, `STRIP_CTX_ARRAY_ELEMENT_SHIFT=2` + adjacency cross-checks; typed `DecoderStackArg`/`DispatcherScratch` enums; `SourcePlumbingPair::for_role` + `is_self_copy_degenerate` predicate cross-validated against `McBankAssignment::is_self_copy`; 374 tests) + r225 **spec/05 §5.5 chroma-plane scaling surface** (`indeo3::mc_chroma`: `LUMA_PIXEL_PER_CHROMA_PIXEL=4` YVU9 4:1 third-bullet ratio cross-checked against the `160 == 40*4` strip-width split; `CHROMA_PACKED_MV_FACTOR_IS_BUFFER_STRIDE=true` fourth-bullet disposition pinning the §3.3 `176`-factor as buffer-allocated row-stride; `MC_KERNEL_GEOMETRY_IS_PLANE_ROLE_INVARIANT=true` first-bullet disposition; typed `MvPixelOffsetInterpretation::LumaOrChromaUniformBufferStride` enum + `pixel_offset_row_stride()` accessor returning the §3.3 row-stride); lacks §7.2 boundary fix-up + §7.3 reverse decomposition + MC inner loop | — |
| **Indeo 2/4/5** | 🚧 scaffold — pending clean-room workspace; Indeo 4/5 still sandboxed via `oxideav-vfw` | — |

</details>

<details>
<summary><strong>Image</strong> (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **PNG / APNG** | ✅ 100% — 5 colour types × 8/16-bit + APNG + sBIT/pHYs/tIME/bKGD/hIST/eXIf/sRGB/cICP/sPLT + r154 Criterion benches + r183 tRNS keyed transparency promotion + r196 APNG frame-scan Criterion bench + r208 **iCCP + iTXt round-trip** (`metadata::Iccp { name, profile: Vec<u8> }` W3C PNG3 §11.3.2.3 opaque zlib-compressed; single-instance; rejected on duplicate; emitted before PLTE/IDAT in §4.3 rank 2 between cICP and sRGB; `metadata::Itxt { keyword, compressed, language_tag, translated_keyword, text }` PNG3 §11.3.3.4 UTF-8 successor to tEXt with BCP47 language tag, optional zlib body, no-NUL-in-translated-keyword+text rule; multi-instance; emitted after zTXt; 264 tests) + r214 **mDCV + cLLI HDR static metadata** (W3C PNG3 §11.3.2.7 Mastering Display Color Volume 24-byte: 3 RGB primary `(x,y)` + white-point + max/min luminance; §11.3.2.8 Content Light Level Info 8-byte MaxCLL+MaxFALL; stored-int big-endian preserved verbatim per Tables 19/20; ordering cICP→iCCP→sBIT→sRGB→gAMA→cHRM→mDCV→cLLI; zero-in-cLLI sentinel preserved; combined HDR10 cICP+mDCV+cLLI round-trip; 388 tests; metadata-chunk gap list closed) | ✅ 100% — r226 **sub-byte (1/2/4-bit) encode for colour type 0 / 3** (greyscale + indexed write paths now bit-pack per W3C PNG §11.2.2 — closes prior 8-bit-only encode gap) |
| **GIF** | ✅ 100% — 87a/89a + LZW + interlaced + animation + disposal compositor + structured Application Extensions + Plain Text Extension + lenient mode + lazy Playback + animation-timing accessors + fluent AnimationBuilder; clean-room from CompuServe spec + r153 tracked spec-derived fuzz seed corpus (5 seeds × 3 targets) + r224 **§18.c.v / §20.c.viii Sort Flag accessors** (`has_sorted_global_palette()` GCT-level query + `frames_with_sorted_palette()` LCT/GCT precedence iterator + `all_frames_palettes_sorted()` stream short-circuit; surfaces the existing parsed bit at the call site that consumes the palette-display-constrained optimisation) | ✅ 100% — per-frame palettes + `optimize_color_tables()` GCT/LCT hoisting + §7 Required Version enforcement + `upgrade_version_if_needed()` |
| **WebP** (VP8 + VP8L) | ✅ 100% | ✅ 100% |
| **JPEG** (still) | ✅ ~95% — via MJPEG | ✅ ~90% — via MJPEG |
| **TIFF** (6.0) | ✅ ~98% — II/MM + BigTIFF + 7 photometrics + 1/4/8/16-bit + None/PackBits/LZW/Deflate/CCITT-MH/T.4-1D + tiles + multi-page + JPEG-in-TIFF (incl. CMYK) + PlanarConfiguration=2 + cargo-fuzz (panic-free 7.7 M iter) + r213 **§SampleFormat (tag 339) decoder inspection** (`SAMPLE_FORMAT_{UINT,SINT,IEEE_FP,UNDEFINED}` constants; accepts unsigned (1) + undefined (4 folded to unsigned per §SampleFormat note); typed `Error::Unsupported` for signed (2) + IEEE-FP (3); rejects per-component non-uniform + ≥5 out-of-range per spec's `must terminate the import process gracefully` rule; 258 tests) | ✅ Gray8/16/RGB24/Palette8 — None/PackBits/LZW/Deflate + Predictor=2 + PlanarConfiguration=2 separate-planes write + Bilevel CCITT-MH / T.4-1D + tiled chunky write + tiled PlanarConfiguration=2 write |
| **BMP** | ✅ ~97% — 1/4/8/16/24/32-bit + V4/V5 + OS/2 + RLE4/RLE8 + 3 fuzz targets + 31-test property sweep + r205 V4/V5 colour-space + embedded ICC profile decode/encode | ✅ ~97% — top-down + `biClrUsed`-trimmed palette + r205 `encode_bmp_with_icc_profile` + r210 **`encode_bmp_with_linked_icc_profile`** (`bV5CSType = LCS_PROFILE_LINKED` — writes `bV5ProfileData` / `bV5ProfileSize` pointing to a linked UTF-16LE filename payload appended after pixels per Windows GDI BMP V5 ICM contract; pairs with the r205 `PROFILE_EMBEDDED` writer for the full V5 ICC binding) + r225 **V5 + ICC encode now accepts `Rgb565`** (third direct-colour arm on both `encode_bmp_with_icc_profile` + `encode_bmp_with_linked_icc_profile` alongside `Rgba`/`Rgb24`; emits 124-byte `BITMAPV5HEADER` with `biCompression = BI_BITFIELDS` + canonical `R=0xF800/G=0x07E0/B=0x001F` masks in the V5 four-mask region @ offsets 40..56 — no separate 12-byte mask tail; ICC/path blob trails byte-identical to the 32/24-bit arms; new `RGB565_MASKS_V5` const; indexed input still returns `BmpError::Unsupported` on V5 paths) |
| **Netpbm** (PBM/PGM/PPM/PNM/PAM) | ✅ ~95% — all 8 magics at 1/8/16-bit + 6 PAM TUPLTYPEs + r171 cargo-fuzz harness + decoder pre-allocation OOM hardening + r210 `read_be16_row` P5/P6/P7 16-bit MSB-first row helper + r217 **`swap_bytes_u16_row` LE→BE SIMD-friendly encode-side row helper** (replaces per-sample `out.push(chunk[1]); out.push(chunk[0])` across `encode_p5_gray16`/`encode_p6_rgb16`/`encode_p7_rgb16`/`encode_p7_rgba16`; encode P5/P6/P7 16-bit ≈18× faster on M-series at ~48-50 GiB/s; 59 lib tests) | ✅ ~95% |
| **ICO / CUR / ANI** | ✅ ~98% — multi-res + BMP/PNG sub-images + CUR hotspot + ICONDIRENTRY validation + 256×256 PNG round-trip + r198 standalone `read_ani_raw` + r198 `biBitCount` reject + r204 ANI `seq[]` step-index bounds-check + r216 BMP body `biPlanes ∈ {0,1}` reject + r221 **BMP body `biCompression ∈ {BI_RGB, BI_BITFIELDS}` reject** (the ICO-spec-mandated zero-or-BITFIELDS rule; explicitly rejects BI_RLE8/RLE4/JPEG/PNG/ALPHABITFIELDS-as-DIB-compression to prevent codec smuggling through the BMP-DIB sub-image path; PNG body exempt; 91 tests) | ✅ ~92% |
| **JPEG 2000** | 🚧 r15 (post-2026-05-20 orphan) — T.800 main-header + SOT/SOD + typed COC/QCC/POC/RGN/PLT/PPT + JP2 box + §B.10 tier-2 + §B.5 ResolutionLevel + §B.6 precinct + §B.7 code-block partition + Annex C §C.3 tier-1 MQ + Annex D 19 contexts + §B.12.1 5 packet-progression iterators + §B.12.2 POC + r181 Annex F.3 inverse DWT + r187 4 cargo-fuzz targets + r192 Annex E code-block→sub-band reassembly + r195 Annex G MCT primitives + r201 §G.1 DC level-shift + r208 **§F.3.1 IDWT cascade `idwt_5x3` + `idwt_9x7`** (initialises at `levels[0]` NLLL band; iterates `k = 1..=NL` folding each level's `[HL, LH, HH]` triple through `dwt::sr_2d_*` with origin `(levels[k].trx0, levels[k].try0)`; carries LL forward; returns full tile-component resolution `Interleaved2D<i32>`/`<f64>`; handles `NL = 0` per §F.3.1 "`a0LL` is output `I(x, y)`"; 388 tests) + r214 §D.5 segmentation symbol + Table A.19 code-block-style flags + r220 **§D.7 vertically-causal context formation toggle through tier-1 decoder** (`CodeBlock::with_vertically_causal_context(enabled)` clips Figure D.2 below-row neighbour slots `D2`/`V1`/`D3` to insignificant at stripe-bottom rows; `BitPlaneSequencer::with_vertically_causal_context` syncs per call; new `neighbours_in_stripe` helper; default off — byte-identical to r208; 410 tests); lacks §B.12 progression-walker → BlockSource adapter wiring + §D.4.2 termination + §D.6 + HTJ2K Part-15 | 🚧 scaffold |
| **JPEG XL** | 🚧 ~92% — ISO/IEC 18181-1:2024 lossless Modular path + 7 fixtures pixel-correct + VarDCT scaffold + Gaborish/EPF/AFV pure-math complete + §C.8.3 per-block HF coefficient loop + r190 `PerPassNonZerosGrids` per-pass container + r191 WP trace oracle isolating #799 divergence + r195 WP state-evolution backward bisect + r202 row-3 chain widening + r208 **§C.5.4 + §C.8.3 per-LfGroup varblock-walk driver** (`varblock_walk` module: `Varblock { x, y, transform, hf_mul }` + borrow-based `VarblockWalk` raster-order iterator over `DctSelectGrid` skipping Continuation cells; `count_varblocks`; typed per-pass per-channel `decode_varblocks_for_pass_channel` invoking `block_ctx_for_varblock` closure + threading each varblock through `PerPassNonZerosGrids::decode_block_at_for_pass_channel`; closes "per-LfGroup varblock-shape grid" gap r177/r183/r190 module notes repeatedly deferred — bridges r13 DctSelect placement with r190 per-pass per-channel NonZeros routing; 650 tests) + r214 per-LfGroup `BlockContext()` resolver + r221 **three-channel per-LfGroup varblock decode driver** (`decode_varblocks_three_channels_with_resolver`; FDIS §C.8.3 outer raster + inner X→Y→B sweep; `qdc[3]` computed once per varblock (not per-channel); `BlockContextResolver::resolve` 3× per varblock; typed `Vec<ThreeChannelVarblock>` return; eliminates 3× redundant grid walks; 675 lib tests + 12 integration tests); lacks upstream WP fix (#799) + §C.7.2 histograms | — retired |
| **JPEG XS** | 🚧 ~82% — ISO/IEC 21122 Part-1 + 5/3 DWT + Annex C/D/F/G + multi-component + CAP-bit + high bit depth + r190 4:2:0 chroma at NL,y≥3 | 🚧 ~90% — Nc 1/3/4 + Sd>0 + RCT + Star-Tetrix + NL up to 8 + odd dims + vertical prediction + per-band Q + NLT + high-bit-depth Star-Tetrix lossless+lossy + r206 per-slice `Q[p]` override + r212 rate-budget-driven `Q[p]` picker + r218 **rate-budget `R[p]` picker `pick_rp_for_target_bytes`** (linear scan `NL-1..=0` on real-encode measurements; `encode_planar_rp_target_bytes` returns `(codestream, rp)`; companion to r212 per-slice `Q[p]` picker on the §A.4.4/A.4.11 precinct-refinement path; 349 tests) + r224 **joint per-slice `Q[p]` + `R[p]` rate-budget picker** (`pick_q_slices_rp_for_target_bytes(.., hsl, target_bytes, planes) -> (Vec<u8>, u8)` nested search exploiting Annex C.6.2 monotonicity on both axes; outer loop on `rp ∈ 0..NL-1` keeping last fitting; inner loop reuses r212 three-pass Q[p] strategy; `encode_planar_hsl_qslice_rp` joint primitive composing r206 per-slice Q[p] with r115 precinct refinement; `encode_planar_hsl_qslice_rp_target_bytes` returns `(codestream, q_slices, rp)`) |
| **AVIF** | 🚧 ~89% — HEIF→AV1 + grid + imir/clap/colr/pixi/pasp + HDR metadata + AV1 wrap + DoS caps + HEIF item-properties + auxC URN + rloc/lsel/iovl/grpl + `mif1` + r130 tmap §4.2.2 + r188 ISO 21496-1 Annex C.2 `GainMapMetadata` + r193 §5.2.5.3+§5.2.7 value-comparison shalls + r195 §8.2/§8.3 AVIF still-image profile-compliance audit + r201 av1-avif v1.2.0 §3 AVIS shall-level audit + r206 §8.2/§8.3 AVIS sequence-track profile audit + r212 §8.6.6 AVIS Edit List parse + §8.6.6.3 audit + r218 **`inspect_avis(file) -> AvisInfo` aggregator** (single-call walks ftyp+moov+first-track stbl folding §3 sequence + §8.2/§8.3 profile-compliance + §8.6.6.3 edit-list audits; `is_compliant_all`/`missing_all`/`duration_seconds`/`is_avis_brand`/`frame_count` helpers; pinned on real `alpha_video.avif`; 304 default + 289 standalone tests) + r224 **AVIS `mdhd` media-timescale plumb + `EditListEntry` second-conversion helpers** (`parse_avis` walks first track's mdia/mdhd to pull v0/v1 32-bit timescale; missing/truncated/future-version surface as `None`; `AvisMeta::media_timescale` + `AvisInfo::media_timescale` + `EditListEntry::media_time_seconds(media_timescale)` + `EditListEntry::segment_duration_seconds(movie_timescale)` + `AvisInfo::media_duration_seconds()`; §8.6.6.3 empty-edit sentinel (`media_time == -1`) returns None) | — |
| **DDS** | ✅ ~99% — DDS_HEADER + DXT10 + uncompressed (10 layouts) + BC1-7 + BC6H all 14 modes + mipmap + 6-face cubemaps + DX10 arrays + volume textures + 132-entry DXGI table + daily cargo-fuzz + r162 40-case injection-robustness + r176 saturating-math + r192 Criterion benches | ✅ ~96% — uncompressed + BC1-5 + BC7 all 8 modes + BC6H_UF16/SF16 all 14 modes + box-downsample mip chains + cubemap/array + r207 **BC6H second LSQ refinement pass in 17-bit unq integer space** (closes r77 "still deferred" followup; the space `(e0*(64-w) + e1*w + 32) >> 6` decoder interpolation is linear in; pixel-space `half_to_f32`-LSQ over-weights bright-exponent pixels by their float magnitude while unq-space weights uniformly; new `target_unq_uf16` inverts `finish_uf16`, `unq_to_q_uf16` inverts `unquantize_uf16`; SSE-guarded acceptance; **+1.75 dB PSNR uplift (28.00→29.75 dB)** on mixed-dynamic-range test case inside followup's 1-2 dB target) |
| **OpenEXR** | 🚧 ~89% — magic + 8 required attrs + HALF/FLOAT/UINT + NO_COMPRESSION/ZIP/ZIPS/RLE + tiled ONE_LEVEL + sub-sampled chroma + single-part deep scanline + multi-part deep scanline + r130 single-part deep tiled + r181 multi-part deep TILED + r192 multi-part flat TILED ONE_LEVEL read + r196 multi-part flat MIPMAP_LEVELS read + r202 multi-part flat RIPMAP_LEVELS read + r208 **single-part deep tiled MIPMAP_LEVELS read** (`parse_exr_deep_tiled_mipmap` redirects MIPMAP from `parse_exr_deep_tiled` instead of rejecting; composes r130 single-part deep tiled chunk shape with r78 single-part flat MIPMAP iteration order; ROUND_DOWN only; deep ZIP rejected) + r214 **single-part deep tiled RIPMAP_LEVELS read** (`parse_exr_deep_tiled_ripmap` + composed round-130 deep-tiled chunk shape × round-124 flat-RIPMAP iteration order; offset table walks `lvly` outer / `lvlx` inner; INCREASING_Y row-major within cells; chunk headers carry explicit `(lvlx, lvly)`; ONE_LEVEL/MIPMAP redirects with explicit pointer messages; 143 lib + 24 deep_validation tests); PIZ blocked on docs trace | ✅ ~95% — RGBA scanline + r130 single-part deep tiled + r181 multi-part deep TILED + r196 multi-part flat MIPMAP_LEVELS + r202 multi-part flat RIPMAP_LEVELS + r208 **single-part deep tiled MIPMAP_LEVELS write** + r214 **single-part deep tiled RIPMAP_LEVELS write** (`encode_exr_deep_tiled_ripmap` + `DeepRipmapTiledInput`/`DeepTiledRipmapCell`; NONE/RLE/ZIPS; version field only `non_image` (0x800) + `tiles[tiledesc,mode=0x02]` + `type=deeptile`; ROUND_DOWN only; pure-Rust 24×16-in-8×4 ZIPS RIPMAP roundtrip + exrheader rip-map+deeptile validation) |
| **Farbfeld** | ✅ 100% — streaming reader + DoS hardening (dimension overflow + truncated payload guards) + `magick` black-box cross-validator | ✅ 100% |
| **HDR** (Radiance RGBE) | ✅ ~99% — new-RLE + old-RLE + 8 axis-flag combos + shared-exponent + multi-record EXPOSURE/COLORCORR + typed COLORCORR/PRIMARIES/VIEW + apply_exposure/apply_colorcorr + r189 luminance_lm_per_sr_per_m2 + r192 committed-fixture regression anchors + r196 uncompressed scanline R+W + r202 `HdrLimits` resource-cap surface + cargo-fuzz harness + r214 **`HdrImage::effective_primaries()` reference-manual default helper** (returns `header.primaries` or `Primaries::RADIANCE` for §1 default `0.640 0.330 0.290 0.600 0.150 0.060 1/3 1/3` equal-energy white; mirrors r208 `effective_pixaspect`; 88 lib tests) + r226 **chromaticity-derived RGB↔XYZ matrices from `Primaries`** (per-image colour-management hook; CIE 1931 (xy)→XYZ basis solve + 3×3 forward/inverse pair) | ✅ ~98% — new-RLE + old-RLE + auto-RLE + XYZE↔RGB + 8 tonemap ops + CRLF + r179 zero-copy `reorient_for_axis_flags` (~6% encode throughput at 1024²) |
| **QOI** | ✅ 100% — byte-exact vs all 8 reference fixtures + criterion decode bench (540 MiB/s gradient, 1.55 GiB/s solid-RUN) + r162 second cargo-fuzz target encode_roundtrip | ✅ 100% — byte-exact vs reference encoder + r205 **encoder cursor-write hot path** (pre-allocated `vec![0u8; 14 + n*5 + 8]` upper-bound buffer + moving `out_pos` cursor + indexed `buf[out_pos] = ...` stores / `copy_from_slice` + `Vec::truncate` at return; mirrors r183 decoder cursor-write; RGBA 320×240 alpha-changing 1.06→1.96 GiB/s (1.85×), RGBA 320×240 gradient 624→930 MiB/s (1.49×), RGB24 640×480 gradient 431→569 MiB/s (1.32×); 89 default + 89 no-default tests + 5 byte-exact reference fixtures) + r225 **caller-owned-buffer `_into` variants for encode + decode** (`encode_qoi_into` / `encode_qoi_full_into` / `parse_qoi_into` accept `&mut Vec<u8>`; clear-resize-write-truncate keeps the hot path defined exactly once + retained capacity covers the largest image seen; `parse_qoi_into` also returns the parsed `QoiHeader` so callers can size further downstream scratch without keeping the full `QoiImage` around; allocating wrappers now delegate so a future bit-exactness change propagates in one edit) |
| **TGA** | ✅ 100% | ✅ 100% |
| **ICER** (JPL) | 🚧 ~78% — Mars-rover heritage; bit-plane scan + compressed/uncompressed segments + 8 filters + IPN 42-155 §III.B context model + r192 §III.E lenient multi-segment decode (`parse_icer_lenient` / `parse_icer_lenient_with_limits` for DSN-packet-loss spaceflight scenario — `LenientDecode { image, received, missing_count }`; segment 0 required to pin canonical strip dims; missing strips reconstruct as flat 128 matching r6 ROI placeholder; trailing-drop truncates; +9 integration tests) | ✅ ~82% — quota encoding + auto wavelet selection + R-D byte-budget + r189 per-segment §III.D uncompressed fallback + r225 **`segments ∈ {1,2,4,8}` criterion bench sweep** (`encode_compressed_filter_q_segments_64x64` + `decode_compressed_filter_q_segments_64x64`; baseline `segments_1` ≈ 292 µs / 13.4 MiB/s on aarch64-darwin pins per-segment fixed cost of §III.E independent-segment partitioning — 12 B header + fresh AAC context + independent stripe scan per added segment) |
| **WBMP** | ✅ 100% — Type 0 + WbmpLimits DoS caps + adversarial fuzz sweep + r189 caller-selectable `MonoBlack`/`MonoWhite` decode polarity (`parse_wbmp_as` + `CodecParameters::pixel_format` routing) | ✅ 100% — r225 **accumulator-flush pack in `encode_wbmp_from_dither`** (u8 accumulator flushes once per 8 pixels + partial-byte tail for `width % 8 != 0`; bit-positions never collide so byte-identical to per-pixel RMW reference + 2 dedicated exact-bytes tests; 320×240 Gray8 criterion bench ~525 µs → ~514 µs / ~140→~142 MiB/s; structural alignment with threshold path's chunked-eight pack) |
| **PCX** (ZSoft) | ✅ ~98% — 1/2/4/8 bpp planar + packed-bits + 24 bpp RGB planar + grayscale flag + DCX multi-page + DCX `Demuxer` + r136 fuzz-hardened + r197 Criterion bench harness + r215 **1bpp × 3 planes (8-colour EGA RGB) decode** per EGFF + ZSoft rev-5 §4 (`unpack_1bpp_3planes` + `(1,3)` dispatch) + r225 **window-origin (`x_min`/`y_min`) round-trip** (`PcxImage::window_origin: Option<(u16, u16)>` filled by decoder when either header word is non-zero — `None` for conventional zero-origin screen-authored case so re-encode doesn't restate implicit `(0,0)` as data; preserves PCX 3.0+ source-crop metadata across decode→re-encode); lacks 4bpp × 4 planes EGA RGBI | ✅ ~94% — 8 write paths + DCX + r185 framework `Encoder` Rgba/Rgb24/Gray8 + Bgr24/Bgra/MonoBlack/MonoWhite + r215 **`encode_pcx_1bpp_3planes_ega_rgb`** (threshold 0x80 per-channel; PCX 5.0 header w/ even-aligned `bytes_per_line`) + r225 **`encode_pcx_24bpp_window_dpi(x_min, y_min, w, h, &rgb, (h_dpi, v_dpi))`** (single wrapper composing the window-only + DPI-only 24-bit writers so a decode → re-encode pass can round-trip both metadata fields at once without forking the 24-bit body); lacks framework `PixelFormat::Pal8` |
| **ILBM** (Amiga IFF) | ✅ ~94% — BMHD/CMAP/CAMG/BODY + ByteRun1 RLE + EHB + HAM6/HAM8 + PBM + SHAM + PCHG + ANIM op-0/op-5 + CRNG/CCRT + DRNG (DPaint IV extended range, true-colour + register cells); lacks ANIM op-7/op-8, DEEP true-colour | ✅ ~84% — IlbmMuxer parity + masking + ANIM op-5 + CRNG/CCRT/DRNG encoder |
| **PICT** (Apple QuickDraw) | ✅ ~99% — v1 + v2 opcode walkers + drawing rasteriser + DirectBitsRect packType 0..4 + Region + clip + pen-size + Compressed/UncompressedQuickTime skip + r186 indexed PixMap variants + r199 §A-3 reserved-Apple-use v2 opcode skip + r205 v1 (8-bit-opcode) §A-3 Table A-3 completion + r224 **structured Picture Comments ($00A0 ShortComment / $00A1 LongComment)** (`PictComment { kind, data, is_long }` surfaced on `PictImage::comments` + `PictProbe::comments` per Inside Macintosh §A-3 Tables A-2/A-3; passive annotation channel preserved; encoder side `build_short_comment(kind)` / `build_long_comment(kind, data)` + v1 variants + `PictBuilder::short_comment`/`long_comment` chainable methods; u16 size enforced at encode per §A-3's 65535-byte cap; word-alignment for odd-length data; +18 round-trip tests); lacks text rasterisation + embedded `CompressedQuickTime 0x8200` JPEG decode | ✅ ~94% — `PictBuilder` + every v2 drawing-command family + magick cross-decode bit-exact + r211 §A-3 footnote `§` Indexed-PixMap encoder + r217 **structured `PictHeader::{ExtendedV2{hres,vres,optimal_source_rect}, V2{fixed_bounds}}` parser + spec-correct §A-3 emitter** (`Fixed` 16.16 newtype + `SEVENTY_TWO_DPI` + `extended_v2_72dpi(rect)` constructor; v2 emit paths replace `[0u8; 24]` with real headerOp payload; Listings A-5/A-6 closure; 297 tests) |
| **SVG** | ✅ ~99% — full shape set + path + gradients + text + mask + clipPath + use/symbol + svgz + SMIL animate/set/animateTransform + CSS3 Selectors L3 + `@import` + `@font-face` + `@keyframes` + Media Queries L4 + viewBox + 17 filter primitives + CSS Values L4 LengthUnit + CSS Easing L2 + SVG 2 §9.6.1 pathLength + SVG 2 §16.3 `<view>` element + fragment-identifier routing (`#MyView` / `#svgView(...)` + percent-decode + spatial/temporal media-fragment fallthrough) + SVG 2 §5.7 `<switch>` conditional processing (requiredExtensions / systemLanguage) + SVG 2 §13.7.1 `<marker>` typed def capture (refX/refY geometric keywords + markerUnits/orient + verbatim round-trip) + SVG 2 §13.2 `context-fill`/`context-stroke` + SVG 2 §16.5 `<a>` hyperlink (renders as group; link target + HTML attrs preserved across round-trip) + SVG 1.1 §11.5 `display` / `visibility` property handling + SVG 2 §5.8 `<title>` / `<desc>` + §5.9 `<metadata>` capture (multilingual lang, round-trip via PreservedExtras) + r172 SVG 2 §11.10.1.1 text-anchor (start/middle/end, inherited) + §11.8.3 textPath start-offset bias + r215 SVG 1.1 §14.3.5 `clip-rule` + r221 **SVG 2 §13.10.2 `shape-rendering`** (`Auto`/`OptimizeSpeed`/`CrispEdges`/`GeometricPrecision` enum, default `Auto`; inherited via existing cascade; tolerant on `inherit`/unknown; presentation-attribute + inline `style=""` + `<style>`-block lanes all flow through; `ShapeRenderingBinding` preserved-extras side-channel; round-trips on rect/path/g; +18 tests) | ✅ ~88% — round-trips full shape graph + PreservedExtras side-channel + `<view>` re-emit at trailing edge |
| **PDF** | ✅ ~99% — bytes → Scene via xref/xref-streams/ObjStm + `/Prev` incremental + `/Encrypt` R=2..6 + public-key + PKCS#7 + `/Sig` AcroForm + Doc-Timestamp + text extraction + Linearization + Tagged-PDF + EmbeddedFiles + §12.6 actions + 5 stream filters + §8.11 Optional Content + r194 PDF 2.0 §14.13 Associated Files + r197 6 new §12.5.6 annotation subtypes (Line/Polygon/PolyLine/Ink/Caret/Popup/FileAttachment) + r204 **§12.5.6.22 `/Watermark` (Table 190 + Table 191 `FixedPrint` six-number `/Matrix` + `/H`/`/V` percentages)** + §12.5.6.23 `/Redact` non-destructive surface + r215 **§12.5.6.20 `/PrinterMark` + §12.5.6.21 `/TrapNet`** (Table 362 `PrinterMark { mark_name }` from `/MN`; Table 366 `TrapNet { last_modified or version+annot_states, font_fauxing }` + new `decode_indirect_ref_array` / `decode_optional_name_array` helpers; +10 tests; 451-test suite); Movie/Sound/Screen/3D/RichMedia remain `Other` | ✅ ~99% — PDF 1.4/1.5 multi-page + paths/gradients/opacity/clip + RGBA + xref-stream + ObjStm + Linearization writer + `/Encrypt` + public-key + `/Sig` + AcroForm + annotation writer + embedded files + RFC 3161 Document Time-Stamp writer |

</details>

<details>
<summary><strong>3D scenes & assets</strong> (click to expand)</summary>

> The typed Scene3D / Mesh / Material PBR / Skin / Animation / Camera / Light / AudioEmitter model lives in `oxideav-mesh3d`, with `Mesh3DDecoder` / `Mesh3DEncoder` traits and a `Mesh3DRegistry` that's parallel to `oxideav-core::CodecRegistry`. Per-format crates register into it. `oxideav-meta::populate_mesh3d_registry(&mut Mesh3DRegistry)` walks every enabled format's `register()`. Lazy bytes flow through `AssetSource` (with a `raw_storage` pass-through hook for archive-backed sources, e.g. ZIP-stored USDZ textures + audio).

| Format | Decode | Encode |
|--------|--------|--------|
| **STL** (ASCII + binary) | ✅ ~99% — ASCII + binary + per-face attrs + 16-bit colour + multi-`solid` + topology + 9-step repair pipeline + r199 `repair_translate_to_positive_octant` + r205 `repair_make_winding_consistent` + r210 `repair_split_t_junctions` matching fix-up for the validate-module T-junctions sub-check (per-Triangles edge-fan replacement; plane + winding preserved) + r216 **`ValidationReport::defect_total` / `defects_by_rule` quantitative summary accessors** (allocation-free; stable-string keys: `facet_orientation`/`non_unit_normal`/`positive_octant`/`boundary_edges`/`non_manifold_edges`/`t_junction`/`inconsistent_winding`; sums-to-total invariant; 173 lib tests) + r225 **`Bbox::point` / `Bbox::merge` / `Bbox::expanded_by` composition helpers** (`point(p)` degenerate seed for incremental merge chains; `merge(&other)` commutative-associative component-wise union — merge of per-mesh `bbox_of_mesh` reports equals scene-wide bbox by construction; `expanded_by(margin)` symmetric-grow on every face for slicer pre-flight safety margin — 0.0 identity / negative shrinks may produce degenerate or inverted result) | ✅ ~99% — both formats + attribute pass-through + `EncodeStats` + configurable float precision |
| **OBJ** (+ MTL) | ✅ ~98% — full Wavefront grammar + MTL (Phong + Wavefront-PBR + map_* options + typed refl) + smoothing/display attrs + free-form geometry + `xyzrgb` per-vertex colour + Bezier/B-spline/NURBS/Cardinal/Taylor `curv` + `surf` 2D-surface tessellation + r171 cargo-fuzz + r188 `curv2` 2D trimming-curve tessellation + r206 `scrv` special-curve tessellation + r212 MTL `illum`-model property decomposition + r218 **multi-patch Bezier `surf` decomposition** (inverts §"Bezier" `parm_count=K/n+1` to recover `K=degu·(parm_u_count−1)`, walks `patches_u×patches_v` tensor-product grid with per-patch `(degu+1)×(degv+1)` de Casteljau; `obj:surface_patches` provenance emitted when >1×1; single-patch fast path unchanged; 206 tests); lacks surface-aware tri-edge-constrained re-meshing | ✅ ~96% — symmetric + negative-index encoder + polyline rejoin |
| **glTF 2.0** (+ .glb) | ✅ ~97% — JSON + .glb + full PBR + 12 KHR_materials extensions + skin + skeletal animation + sparse accessors + morph-targets + 12 spec-MUST validators + KHR_texture_transform + r188 KHR_mesh_quantization decode + r199 KHR_node_visibility + r212 KHR_xmp_json_ld at 5 of 7 spec surfaces + r218 **KHR_animation_pointer** (animation channels target arbitrary mutable properties via RFC 6901 JSON Pointer; decode siphons pointer-targeted channels into `Scene3D::extras["KHR_animation_pointer"]` as structured roster; encode lifts back to fresh FLOAT input/output accessors + sampler with `target.path = "pointer"`; six §3.12 rejection rules incl. duplicate/syntax/data/node; FLOAT output lane only — normalized-int + bool follow; 354 tests); lacks KHR_audio_emitter + quantized morph-targets + KHR_materials_variants | ✅ ~93% — symmetric round-trip incl. KHR_xmp_json_ld declarations and packet preservation |
| **USDZ** (+ USDA) | ✅ ~93% — ZIP STORED walker + USDA parser + UsdGeomMesh + UsdPreviewSurface PBR + UsdUVTexture pass-through + xformOp transforms + UsdMediaSpatialAudio + variantSet + LIVRPS variant-selection composition + composition-arc round-trip + in-archive sublayer + references/payload arc composition + r180 in-layer `inherits`/`specializes` class-arc composition + r188 reader-side CRC-32/ISO-HDLC verify on `walk()` + r200 `.usdc` (Pixar Crate binary) bootstrap parser + r206 `usdc::decode_int_array` §3b compressed-integer decoder + r212 **§3a `CompressedBuffer`/`CompressedChunk` framing** (single-chunk `0x00` + multi-chunk `(int32 LE length, bytes)`; LZ4 block payloads opaque since spec not staged) + **§4.1 `TokensSection` header** (3 `int64` `numTokens`/`uncompressedSize`/`compressedSize` with 16 Mi/256 MiB defensive caps) + `split_tokens_blob` NUL-delimited token splitter + r217 **§4.2 STRINGS section** (`usdc::StringsHeader` 8-byte `int64 count` w/ 16 Mi defensive cap + `StringsSection<'a>::parse` strict `8 + count*4 == size`; `parse_indices()` materialises `Vec<u32>` for callers walking the pool; STRINGS = subset of TOKENS used as USDA string-typed values, §4.3 FIELDS string reps index into it; cross-validated against Elephant fixture TOC; 89 tests) + r222 **§4.3 FIELDS section framing parser** (`usdc::FieldsHeader` + `usdc::FieldsSection` expose the int64 `count` + per-field token-index/value-rep raw 8-byte tuple stream with defensive 16 Mi caps; consumed by future field-rep type-code decoder); lacks per-section semantics, FIELDS value-rep type-codes, UsdSkel*, UsdGeomSubset | ✅ ~88% — symmetric writer + zero-re-encode pass-through + variant writer + composition-arc writer |
| **FBX** | 🚧 ~92% — binary + ASCII container + object-graph + mesh + animation + deformers + Material/Texture/Video + bind pose + LayerElementMaterial/Color + Properties70 P-record grammar + multi-UV-set surfacing + r207 Light + Camera `NodeAttribute` surfacing + r213 **ASCII FBX writer** (`write_ascii_document` + `AsciiWriterOptions { emit_banner }`; full §1-§7a grammar coverage incl. §3a `Key:  {` two-space SDK quirk + §6 typed-array shorthand `Key: *N { a: ... }`; parse-write-parse closure on `docs/3d/fbx/fixtures/cubes-ascii-v7500.fbx`; `Mesh3DEncoder` Scene3D→FBX still NYI; 124 lib tests) + r226 **bind-pose `bone_to_parent` derivation** (rest-pose inverse-bind chain captures per-joint local transform for skinned mesh ingest). Lacks: multi-LayerElementNormal | ✅ ~58% — symmetric binary writer + opt-in zlib deflate |
| **Alembic** | 🚧 0% — Sphinx API reference + Python examples staged at `docs/3d/alembic/`; on-disk Ogawa binary needs Wayback PDF recovery (Imageworks 2010-2012 manuals 404 today) or commissioned trace | — |

Cross-format integration: `oxideav-cli-convert` exposes a 3D conversion path through `oxideav_meta::populate_mesh3d_registry` — `oxideav convert in.obj out.gltf` (or `--probe` for structural inspection). `crates/oxideav-tests/tests/mesh3d_*.rs` runs the cross-format roundtrip suite. Convert verb has accumulated IM-compatible ops including `-resize` / `-thumbnail` / `-define` / r178 `-extent WxH±X±Y` (canvas re-window w/ source-order `-background` colour) / r184 `-monochrome` (gray + 2 colors + Floyd-Steinberg shorthand) / r222 `-roll ±X±Y` (IM-style circular pixel shift — columns right by `dx`, rows down by `dy`; negative = opposite direction), USDZ encoder + 3D→raster renderer (Gouraud + Phong + `-light` / `-camera` / `-projection` / `-fov` / `-bg`), `-render normal-debug|depth-debug` + `-aa N` supersampling, and multi-size ICO via `-define icon:auto-resize`. Black-box oracles in `tests/mesh3d_{usdz_apple,blender_assimp}_oracle.rs` cross-validate against Apple `usdzconvert` + Blender + assimp.

</details>

<details>
<summary><strong>Trackers</strong> (decode-only by design) (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **MOD / STM / XM** | ✅ ~97% MOD — 4-channel Paula mixer + full ProTracker 1.1B effect set + FT-extension `8xx`/`E8x` pan + XM E3x glissando + Lxy set-envelope-position + E4x/E7x vibrato/tremolo waveform shapes + r171 cargo-fuzz; ✅ ~90% STM — r197 codec id `stm` promoted from stub to real `StmDecoder`; ✅ XM r203 codec id `xm` promoted from stub to full playback decoder + r215 **STM `E6x` pattern loop + `EEx` pattern delay** (per spec/STM §3.x; new `pat_loop_row`/`pat_loop_count` on `StmChannel`; new `pending_pat_loop_row`/`pattern_delay`/`in_pattern_delay_repeat` on `StmPlayerState`; precedence EE delay → Bxy/Dxy → E6 rewind → linear advance; +6 tests) + r226 **STM `E9x` retrigger-note effect** (per-tick re-trigger driver landed on `StmChannel`; honours pattern-delay + Bxy/Dxy precedence) | — |
| **STM** (Scream Tracker v1) | ✅ ~85% — structural parse + shared-mixer playback; XM-parity effects (Gxy/Jxy/Bxy/Cxy/Exy/Hxy + 7xy tremolo + volume-slide variants); hard-pan LRRL | — |
| **XM** (FastTracker 2) | ✅ ~90% — structural parse + full playback; envelopes + fadeout + key-off; vibrato + tone porta + pattern jumps + fine/extra-fine porta + Exy/Kxy subcommands + volume-column slides | — |
| **S3M** | ✅ ~96% — stereo + full ST3 v3.20 effect set + per-channel effect memory + Dxy case matrix + S3x/S4x bit-2 retention + Qxy persistent-counter retrigger + Cxx row-≥64 ignore + Kxy/Lxy continue + r171 +128 channel-mute + r183 spec-correct default-pan + r197 header-driven playback corrections + r203 Vxx spec-correct value range + r211 §Mixing MV-byte clamp + stereo×11/8 gain + r216 **PCM active-volume peak = 63 (not 64)** per multimedia.cx §Playback Notes (every effect-side volume write — instrument-load, vol-column, fine/per-tick/fast `Dxy`, `Qxy` retrigger modifiers, `Rxy` tremolo, `Ixy` tremor — funnelled through `clamp_pcm_volume(u16) -> u8` capped at `PCM_VOLUME_PEAK = 63`; mixer reads defensively for external `Channel` literals; ~0.14 dB ceiling correction; 91 tests); lacks AdLib FM synth | — |

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
| MSMPEG4 v3 (DIV3) | `mpg4c32.dll` | wmpcdcs8-2001 reference binary | ✅ **DECODE 17/17 frames at 42.9 dB PSNR-RGB + ENCODE end-to-end externally validated** — full `ICCompress*` lifecycle wired r51; 176×144 BGR24 → 970-byte MP43 I-frame (78×); self-roundtrip 27.83 dB; AVI 1.0 wrap decodes cleanly through ffmpeg + mpv + ffprobe (mean 20.86 dB at q=5000). Covers I/P frames, skip-MB (~38%), alt-MV-VLC, AC-prediction. See crate README for the per-round forensic ladder. | Required: 13 stubs + x87 ISA (FLD/FST/FADD…/FSIN/FCOS/FPREM) + DirectShow GUID handshake + `ICINFO_SIZE = 568` gate. 12 dB matrix delta intrinsic (codec rejects every non-BI_RGB output 4CC). |
| MSMPEG4 v3 DShow | `mpg4ds32.ax` | winxp | ✅ **Full GOP DirectShow decode + 20/20 across 16 fixture-runs** — covers 6/6 FOURCC variants (MP43/DIV3/DIV4/DVX3/AP41/COL1) all routed through MP43 subtype; motion-pan-352×288 + skip-MB + AC-pred fixtures all green. See crate README for per-round forensic ladder. | DirectShow IBaseFilter wrapper: COM scaffolding + ole32 stubs + HostIFilterGraph + HostIPin + HostIMemAllocator (committed state) + HostIMediaSample + IMediaFilter Pause/Run/GetState. CLSID `{82CCD3E0-F71A-11D0-9FE5-00609778EA66}`. |
| WMV1/2 DShow | `wmvds32.ax` | winxp | CLASS_E_CLASSNOTAVAILABLE on default CLSID | Needs the shipped `wmvax.inf` filter CLSID; round-26+ |
| MSADDS audio | `msadds32.ax` | winxp | 🚧 **Pipeline driven through Receive, E_FAIL inside inner-decode (r70)** — full PE-load + COM + dual-pin allocator handshake green; ffmpeg-derived extradata flips Receive HRESULT 0x8000FFFF → 0x80004005. **r70 pinned the actual bail JCC at `0xe282`**: `cmp edi, [ebp+0x10]` then `jge → 0xe2bb`, with EDI=0x748 emission counter walked up to declared sample-count bound 0x748. Round 69's `0xea3a` hypothesis falsified at one of 9 distinct JCCs reaching `0xe2bb`. r63 helper_addref patch retirement confirmed (phase-2 A/B identical reach-sets). See crate README for round ladder. | Same scaffolding as MP43 video; `AmtBlueprint::wma_{criteria_passing,with_ffmpeg_extradata_prefix}()`; QueryAccept disasm at `docs/codec/msadds32-query-accept-validation.md` |

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
| **`oxideav-videotoolbox`** | macOS (Apple Silicon + Intel Macs) | 🚧 H.264 + HEVC + ProRes + MJPEG + MPEG-2 + VP9 + MPEG-4 Pt 2 + AV1 (M3+) | 🚧 H.264 + HEVC + ProRes + MJPEG | r198 encoder knobs wired across H.264 / HEVC / MJPEG / ProRes: `bit_rate` → `AverageBitRate`, `options["quality"]` (`Float32 [0,1]`) → `Quality`, `options["profile"]` aliases (H.264 baseline/main/high/extended; HEVC main/main10/main4_2_2_10) → `ProfileLevel`; `make_prores_encoder` dispatches via `prores_codec_type_for_tag()` across all 6 fourCCs (apco/apcs/apcn/apch/ap4h/ap4x). PSNR_Y: MPEG-2 ~61 dB; H.264 ~51 dB; HEVC ~54 dB; ProRes ~52 dB; MJPEG ~36 dB; AV1 ≥30 dB (M3+/macOS 14+). r178 VP9 + r184 MPEG-4 Pt 2 + r190 VOL→ESDS + r205 AV1 `av1C` extension-atom path + r211 VVC (H.266) decode path + r217 **H.264 + HEVC profile-alias map expansion + HEVC `Main42210` output-value fix** (30 H.264 named-level aliases `baseline_/main_/high_/extended_` × `1_3..5_2` mapping to `H264_*` SDK strings + 2 constrained `constrained_baseline/constrained_high` `_auto/_autolevel` → macOS 12.0+ `H264_Constrained{Baseline,High}_AutoLevel`; canonical-form pass-through for 35 documented `H264_*` + 3 documented `HEVC_*` SDK strings as closed set; round-9 HEVC Main4:2:2 10-bit alias emitted wrong `Main4_2_2_10` → silent VT fallback to Main; r217 emits actual SDK CFString `HEVC_Main42210`; black-box validated via Apple-SDK CFString reader; +7 tests) + r222 **keyframe-cadence + frame-rate VTCompressionSession property writes** (`options["keyframe_interval"]` → CFNumber-i32 `MaxKeyFrameInterval`; `options["keyframe_interval_duration"]` → CFNumber-Float64 `MaxKeyFrameIntervalDuration`; `options["expected_frame_rate"]` → CFNumber-Float64 `ExpectedFrameRate` with `params.frame_rate.as_f64()` fall-back; new `sys::cf_number_f64` helper; 0-sentinel accepted, NaN/±∞/negative rejected; H.264/HEVC/MJPEG/ProRes paths; +5 unit + 1 live VT integration). |
| **`oxideav-audiotoolbox`** | macOS | 🚧 AAC LC + HE-AAC v1/v2 + AAC-LD/ELD + ALAC + iLBC + AMR-NB + AMR-WB + MP3 + **FLAC** | 🚧 AAC LC + HE-AAC v1/v2 + AAC-LD/ELD + ALAC + iLBC | r178 AAC encoder bitrate read-back; r184 iLBC; r190 AMR-NB; r193 AMR-WB; r199 MP3 decode via `kAudioFormatMPEGLayer3` (bit-exact 33×1152 PCM @ ≈89.8 dB SNR); r206 FLAC decode via `kAudioFormatFLAC` + r212 **ALAC S32 lossless contract tightening** (`AudioStreamBasicDescription::pcm_s32` signed-integer/packed; `AlacAtDecoder` picks output ASBD per `params.sample_format` — `Some(S32)` → `pcm_s32`, else legacy `pcm_s16` byte-identical for existing callers; bit-exact 190,464/192,000 i32 round-trip with 24-bit low-bit noise below S16 quantisation floor) + r218 **FLAC encoder via `kAudioFormatFLAC`** (bit-exact 188 416 / 192 000 i16 round-trip @ 48 k/2 ch; S16/S32 input, 24-bit output cap; persistent `Box<PcmContext>` feeder fixes AT EOF-poison on zero-byte callback; AT vends `dfLa` magic cookie verbatim; 187 tests) + r224 **Opus decoder + encoder via `kAudioFormatOpus` (`'opus'`)** (RFC 7845 §5.1 OpusHead parser/builder w/ mapping family 0/1/255; RFC 6716 Table 2 frame-duration→PCM-frame mapper `frames_per_packet_48k`; `OpusAtDecoder` outputs interleaved S16 PCM at any of the five RFC 6716 §2.1.1 rates 8/12/16/24/48 kHz; `OpusAtEncoder` symmetric; ~26 dB SNR roundtrip; wall-isolated `OpusHeadError` keeps `src/opus.rs` `oxideav-core`-dep-free for `--no-default-features` CI). Roadmap closed. |
| **`oxideav-vaapi`** | Linux (Intel iGPU + AMD Radeon, via libva) | — stub | — stub | Crate exists; impl is a single-line `// stub` + r223 **codec id → VAProfile family map lifted into public module** (first surface piece of the planned decode ladder). Planned decode ladder: H.264 + HEVC + VP9 + AV1 (Mesa Radeon, Intel Media Driver). |
| **`oxideav-vdpau`** | Linux (NVIDIA legacy / Nouveau) | 🚧 H.264 + HEVC + VP9 + MPEG-2 (r225) | — stub | r225 **register four `CodecInfo` entries for h264/hevc/vp9/mpeg2video** with `CodecCapabilities::video("<codec>_vdpau")` + `with_decode()`+`with_lossy(true)`+`with_hardware(true)`+`with_priority(15)` (slightly worse than VA-API's 10) + `with_max_size(8192, 8192)`; per-codec FourCC/Matroska codec-id container tags; `with_engine_id("vdpau")` + `with_engine_probe(engine_info)`; pre-flight load of libvdpau.so.1 / libX11.so.6 — no-op return when frameworks absent. |
| **`oxideav-nvidia`** | Cross-platform (NVENC + NVDEC via libnvcuvid + libnvidia-encode) | 🚧 VP9 + AV1 + MPEG-2 (r225) | — | r225 **`Mpeg2NvDecoder` + MPEG-2 NVDEC factory** (codec-agnostic cuvidParser since round 4 — MPEG-2 follows the VP9/AV1 template w/ `CudaVideoCodec::Mpeg2` + `bAnnexb=0` since MPEG-2's `0x000001` start codes are walked natively; pre-flight `cuvidGetDecoderCaps(Mpeg2, 4:2:0, 8-bit)` surfaces `Error::Unsupported` early on datacenter SKUs without a video engine → registry falls back to pure-Rust oxideav-mpeg12video; registered at `priority(5)` w/ QT/MP4 fourCC family `mp2v`/`MPG2`/`mpg2`/`hdv2`/`m2v1` + Matroska `V_MPEG2`; NVENC MPEG-2 intentionally not registered). |
| **`oxideav-vulkan-video`** | Cross-platform (Vulkan VK_KHR_video_*) | 🚧 H.264 + HEVC + AV1 capability queries (r225) | — empty | r225 **HEVC + AV1 chained capability queries** through `vkGetPhysicalDeviceVideoCapabilitiesKHR` populating `max_width`/`max_height`/`max_dpb_slots`/`max_active_reference_pictures`/`max_level` on the `HwCodecCaps` rows surfaced to the CLI info command; `sys.rs` adds the StdVideo H.265 + AV1 type aliases, four sType discriminants (1000187000/003 + 1000512000/003), profile + anchor-level constants, std-header EXTENSION_NAME + SPEC_VERSION, and four new repr(C) structs (`VkVideoDecode{H265,AV1}{ProfileInfo,Capabilities}KHR`) cross-checked against the C ABI; `query_video_decode_h265_capabilities` (H.265 Main 8-bit 4:2:0) + `query_video_decode_av1_capabilities` (AV1 Main 8-bit 4:2:0, film-grain opt-out). |

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
| **`oxideav-source`** | URI resolution + file reader + prefetching BufferedSource | ✅ `file://` + `mem://` + `data:` (RFC 2397) + `concat:` (`|`-separated; r184 widened to mem://`/`data:`/`slice:` inner schemes) + r178 `slice:<offset>+<length>!<inner-uri>` byte-window + `FileScope` allow-list + r212 **`FileScope::deny_dir` carve-outs over allow-list** (`permissive().deny_dir("/etc")` = everything-except; canonicalised + dedup + component-aware; `is_allowed_path` inspector) + r222 **`file://` URI percent-decoding per RFC 3986 §2.1** (`file:///tmp/foo%20bar.txt` now resolves to `/tmp/foo bar.txt`; invalid sequences surface a typed parse error; pct-encoded query/fragment dropped on extraction) |
| **`oxideav-http`** | HTTP / HTTPS source driver | ✅ `http://` + `https://` via pure-Rust `ureq` + `rustls` + `webpki-roots`; Range-request seeking; `HttpConfig` policy + r171 RFC 7233 §4.2 Content-Range validation + §3.1 200-fallback prefix-drop + r179 §15.5.17 + §14.4 416 handling + r186 RFC 9110 §13.1.5 If-Range strong-validator + r197 §8.6 Content-Length cross-checks + r203 RFC 9110 §13.1.5 **HTTP-date now accepts all three forms** (IMF-fixdate already supported; rfc850-date `weekday, DD-Mon-YY HH:MM:SS GMT` + asctime-date `Wkd Mon DD HH:MM:SS YYYY` parse + canonicalize through the strong-validator pipeline; +2 fuzz seeds `rfc850_date`/`asctime_date`; +468 LOC in src/lib.rs) + `parse_headers` cargo-fuzz harness (10 seeds) |
| **`oxideav-generator`** | Synthetic media source (`generate://...` URIs) + zero-input filters | ✅ audio synth (sine + chirp/FM/DTMF/multitone/ADSR/ringmod + r180 5-colour noise + r198 `pwm` + r205 `supersaw`/`saws` detuned-sawtooth stack) + image (xc/gradient/pattern/fractal/plasma/noise/label; r188 Perlin-2001 + r210 **Worley/cellular** — `noise:worley`/`noise:cellular` with `dist=euclidean|manhattan|chebyshev`, `k ∈ [1,4]` F-k selector, `points ∈ [1,4]` per-cell feature points; in-tree LCG-keyed feature-point placement; third basis alongside perlin/simplex; Worley 1996 SIGGRAPH first-principles maths) + video (testsrc/smptebars/fractal_zoom/gradient_animate/zoneplate) + r225 **synth `tremolo` / `trem`** (carrier ∈ `sine/square/triangle/sawtooth` mirroring `adsr`; unipolar cosine envelope `e(t) = 1 − depth · 0.5 · (1 − cos(2π·lfo·t))` strictly in `[1−depth, 1] ⊆ [0,1]`; defaults wave=sine, freq=440, lfo=5 Hz, depth=0.5; spectrum stays centred on `fc` with low-frequency sidebands at `fc ± lfo` — perceptually classical guitar-amp tremolo, not `am`'s prosthaphaeresis sideband audio-rate modulator); 177 lib + 26 integ tests |
| **`oxideav-rtmp`** | RTMP ingest + push | ✅ Server + client; AMF0/AMF3 parser/builder; Enhanced-RTMP v1 video + v2 audio + ModEx; pluggable key-verification; `rtmp://` PacketSource; symmetric teardown + client `poll_event` + r179 v2 `MultichannelConfig` audio body (24 SMPTE ST 2036-2-2008 22.2 channel positions) + r187 Enhanced-RTMP v2 Multitrack body parser+builder + r198 §E FLV file/byte-stream writer + r205 §E `FlvReader<R: Write>` inverse-of-FlvWriter + r211 NetConnection `connect` capability negotiation + r217 **§7.1.6 Aggregate Message (type 22) parser + builder** (FLV-shaped sub-header per §6.1.1/§E.4.1; §E.3 `PreviousTagSize == 11 + DataSize` back-pointer invariant; §7.1.6 outer-timestamp-overrides-sub-StreamID rule; timestamp renormalisation `t_i + (T_agg − T_0)`; 1024-iter xorshift fuzz of `parse_aggregate` produces typed `Err` only; 286 tests) + r224 **fold ModEx `TimestampOffsetNano` onto Packet timeline** (`RTMP_TIME_BASE` switched 1/1000→1/1_000_000_000 so the 0..999_999 ns adjustment integrates losslessly; `audio_to_packet` emits `pts==dts == ts_ms·RTMP_MS_TO_NS + AudioTag::timestamp_offset_nano()`; `video_to_packet` keeps `dts = ts_ms·RTMP_MS_TO_NS` per spec + composes `pts = (ts_ms + composition_time)·RTMP_MS_TO_NS + VideoTag::timestamp_offset_nano()`; multiple `TimestampOffsetNano` entries sum; new `RTMP_MS_TO_NS=1_000_000` const; +5 tests) |
| **`oxideav-sysaudio`** | Native audio output | ✅ Runtime-loaded backends (ALSA, PulseAudio, WASAPI, CoreAudio); no C build-time linkage. CoreAudio + WASAPI backends report **real HAL latency** — CoreAudio sums `kAudioDevicePropertyLatency` + `BufferFrameSize` + `SafetyOffset` + `kAudioStreamPropertyLatency`; WASAPI reads `IAudioClock`-derived presentation latency. Output-device enumeration (names + default flag) across WASAPI / ALSA / CoreAudio. r178 per-device routing API + r184 CoreAudio per-device routing (all 4 backends route) + r206 `StreamRequest::buffer_frames` honoured on every functional backend + r212 `Driver::preferred_format` introspection on WASAPI + CoreAudio + r218 **ALSA `preferred_format` via throwaway `snd_pcm_open(NONBLOCK)` + `snd_pcm_hw_params_any`** (canonical 48 k/2 ch hint fed through `set_rate_near` + newly-resolved `set_channels_near`; PCM closes before return — no contention with later real `open()`; all 4 functional backends now introspect; PulseAudio simple-API stays `Ok(None)`) + r224 **functional OSS `/dev/dsp` backend via dlopen'd libc** (`SNDCTL_DSP_SETFMT AFMT_S16_LE` + `SNDCTL_DSP_CHANNELS` + `SNDCTL_DSP_SPEED` negotiation through `libloading` against `libc.so.6` / `libc.so` / `libc.musl-*` so produced binary has no audio library in NEEDED list; worker converts f32 callback to S16_LE before each `write(2)`; `SNDCTL_DSP_RESET` on `stop` keeps Drop snappy; per-device routing via `with_device("/dev/dsp1")`). BT-aware; falls back to software estimate if HAL unavailable. |
| **`oxideav-pipeline`** | Pipeline composition (source → transforms → sink) | ✅ JSON transcode-graph executor; pipelined multithreaded runtime + `Executor::with_channel_caps(ChannelCaps { packets, frames })` configurable per-track depth (embedded `{1,1}` → offline `{64,32}`) + `Executor::with_max_queue_bytes(n)` orthogonal byte-ceiling on demux→worker queues + r178 `Progress::elapsed_micros` wall-clock stamp on every emission (realtime ratio + live-source drift diagnostics) + r184 `packets_skipped: u64` on `Progress` + `ExecutorStats` + r205 **`Progress::packets_read: u64` demuxer-cumulative count** (headroom = `packets_read − frames − packets_skipped` = wedged-decoder signature; demuxer-still-reading vs decode-stage-stalled now distinguishable per emission) + r205 EOF Progress retry-up-to-100×1ms ride-out for backed-up receivers (drops on saturation rather than blocking; fixes pre-existing Windows-runner flake `elapsed_micros_bounded_by_eof_value`) + r222 **`Progress::packets_copied` sink-cumulative count** (mirrors `ExecutorStats::packets_copied` per-emission; complements r205 `packets_read` so source-vs-sink lag is one subtraction; copy-stage stall now distinguishable from demux/decode stall) |
| **`oxideav-scene`** | Time-based scene / composition model | 🚧 data model for PDF pages / RTMP streaming compositor / NLE timelines + r179 per-frame `Sample` + animation-track composition helpers + r188 `RasterRenderer` (bg solid/gradient + Rect/Polygon + `ObjectKind::Vector` → RGBA via oxideav-raster) + r198 **`ObjectKind::Group` nested composition** (per-child resolution at scene time, parent affine/opacity/clip merge, cycle-break, dead-child exclusion) + r198 SVG 1.1 path-data lowering (M/L/H/V/C/S/Q/T/Z + relative) + r204 arc (A/a) per F.6.1 + r211 `ObjectKind::Image(ImageSource::Decoded)` RGBA8 composition via `RasterRenderer` + r216 **`Background::DecodedImage(Arc<VideoFrame>)` full-canvas backdrop** (symmetric with r211 object-side; lowers into `Node::Image` spanning canvas `(0,0)..(w,h)` stretched via raster sampler; non-canonical stride drops silently; path-based `Background::Image(_)` still skips pending decoder); 214 tests |
| **`oxideav-audio-filter`** | Audio effects & conversions (streaming) | ✅ ~48 filters: classic + transient/spatial/restoration family + r106 SlewLimiter + r188 LR4 crossover + r198 `true_peak_detector` + r205 **`state_variable` Chamberlin SVF** (Chamberlin two-integrator-loop State Variable Filter; single recurrence emits LP / BP / HP / Notch from one pair of integrator states; `SvfMode` selects output tap without touching state; analog-prototype-matched `f = 2·sin(π·f_c/f_s)` + `q = 1/Q`; clamps enforce `f_c ≤ f_s/6.5` and `Q ∈ [0.5, 50.0]`; `"svf"` registry entry with JSON `mode`/`cutoff_hz`/`q` keys; modulation-friendly synth filter property — coefficient resolve is one `sin` per `set_cutoff`; 280 tests) + r215 **Criterion benchmark harness `benches/filters.rs`** (7 scenarios — `biquad_lpf`/`equalizer_3band`/`loudness_itu`/`compressor`/`reverb`/`resample_44k1_48k`/`true_peak_4x`; depth-mode polish on saturated row) + r226 **`crest_factor_meter`** (peak-to-RMS sliding-window observer; dynamic-range / loudness diagnostic filter joining the metering family) — see crate README for the catalogue |
| **`oxideav-image-filter`** | Single-frame image effects (stateless) | ✅ 130 filter types / 178 factory names — r198 `Gabor` + r205 **`Niblack` adaptive local-statistics threshold** (Niblack 1986 textbook §5.1 page-segmentation example; per-pixel `T(x,y) = μ + k·σ` over `(2·radius+1)²` neighbourhood, default `k = -0.2`; two-pass separable box-sum via `Var(X) = E(X²) − E(X)²` identity, variance clamped to 0 before `sqrt` for FP-cancellation safety; `O(W·H)` regardless of radius; joins segmentation family at the local-stats threshold slot — complements `AdaptiveThreshold` (k=0) + `OtsuThreshold` (global); 15 unit + 3 factory smoke tests) + r215 **`CurveInterpolation::NaturalCubic`** (Curves interpolant family 3→4; classical C² natural cubic spline via O(n) Thomas tridiagonal sweep + per-segment cubic eval; JSON aliases `natural-cubic`/`natural_cubic`/`natural`; 2-point linear-identity edge case; 1090 lib tests) + r226 **`CentripetalCatmullRom`** (Yuksel 2011 / Barry-Goldman parametric tension-free Catmull-Rom variant; Curves interpolant family 4→5; avoids self-intersection on closely-spaced control points) + bundled rule-E scrub — see crate README for the catalogue |
| **`oxideav-pixfmt`** | Pixel-format conversion + palette + dither | ✅ YUV↔RGB matrices (BT.601 / BT.709 / BT.2020 / BT.2100), chroma subsampling + r179 packed 4:2:2 (YUYV / UYVY) ↔ planar/RGB/RGBA, palette quantisation, Floyd-Steinberg dither, PQ + HLG + BT.1886 transfer functions + r197 Porter-Duff alpha property sweep + r203 `Ya8` (luma+alpha) wired into `convert()` dispatch + r211 **direct `NV12`/`NV21` ↔ `Rgb24`/`Rgba` in `convert()` dispatch** (8 new pairs; bit-exact to staged two-step `Nv ↔ Yuv420P ↔ Rgb`; saves one intermediate `VideoFrame` alloc; odd dims rejected per 4:2:0 half-pixel constraint; PSNR>30 dB pin matches planar; 148 tests) + Criterion suite for compositing hot path (alpha bench) |

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
| **SRT** (SubRip)    | ✅ | ✅ | `<b>/<i>/<u>/<s>`, `<font color>` hex + 17 named, `<font face size>` + r211 **structural tolerance: leading preamble / PEM-armoured envelope + duplicate-index rows + whitespace-only continuation lines** (cue-discovery loop forward-scans for timing line within the current non-blank block and terminates body only on truly empty line or new timing line; `bytes_to_cue` picks up preamble tolerance too; 167 tests) |
| **WebVTT**          | ✅ | ✅ | Header, STYLE ::cue(.class), REGION, inline b/i/u/c/v/lang/ruby/timestamp (full §3.5 round-trip incl. BCP 47 lang chains, ruby implicit `</rt>`, multi-byte UTF-8), cue-settings round-trip + full REGION block + r216 **§4.1 NOTE comment-block round-trip** (`vtt_note.<idx>` + `vtt_note_pos.<idx>` per-block metadata; case-sensitive `NOTE`/`NOTE `/`NOTE\t` detection; W3C §1.5 worked example byte-stable through both extradata + synth writers) + r222 **§3.4 cue identifier round-trip via `vtt_cue_id.<idx>` metadata** (optional non-empty single-line preamble before the timing line is captured per cue and re-emitted on encode; embeds in the unified subtitle IR without altering cue text) |
| **MicroDVD**        | ✅ | ✅ | frame-based, `{y:b/i/u/s}`, `{c:$BBGGRR}`, `{f:family}` |
| **MPL2**            | ✅ | ✅ | decisecond timing, `/` italic, `\|` break |
| **MPsub**           | ✅ | ✅ | relative-start timing, `FORMAT=TIME`, `TITLE=`/`AUTHOR=` |
| **VPlayer**         | ✅ | ✅ | `HH:MM:SS:text`, end inferred |
| **PJS**             | ✅ | ✅ | frame-based, quoted body |
| **AQTitle**         | ✅ | ✅ | `-->> N` frame markers |
| **JACOsub**         | ✅ | ✅ | `\B/\I/\U`, `#TITLE`/`#TIMERES` headers |
| **RealText**        | ✅ | ✅ | HTML-like `<time>/<b>/<i>/<u>/<font>/<br/>` |
| **SubViewer 1/2**   | ✅ | ✅ | marker-based v1, `[INFORMATION]` header v2 |
| **TTML**            | ✅ | ✅ | W3C Timed Text, `<tt>/<head>/<styling>/<style>/<p>/<span>/<br/>`, tts:* styling + r171 IMSC 1.2: `<layout>` regions + `tts:textAlign` + 22 IR-unmodelled `tts:*` / `itts:*` style extras + 11 `ttp:*` / `ittp:*` parameter attrs + `HH:MM:SS:FF` / `<n>f` / `<n>t` against `ttp:frameRate` / `ttp:tickRate` |
| **SAMI**            | ✅ | ✅ | Microsoft, `<SYNC Start=ms>` + `<STYLE>` CSS classes |
| **EBU STL**         | ✅ | ✅ | ISO/IEC 18041 binary GSI+TTI (text mode only; bitmap + colour variants deferred) |

**Advanced text (own crate)** — `oxideav-ass`:

| Format              | Decode | Encode | Notes |
|---------------------|:------:|:------:|-------|
| **ASS / SSA**       | ✅ | ✅ | Script Info + V4+/V4 Styles (BGR+inv-alpha) + override tags + r172 `\fn`/`\fe`/`\b<weight>`/`\r[<style>]` + r177 `\pbo` + r183 face-flag toggles + r186 typed `\p<scale>` + r198 `\fax`/`\fay` shear baked into per-cue affine + r204 `\an<n>` numpad alignment baked into renderer + r210 `\1a` primary-fill alpha bake + r216 **`\blur<strength>` Gaussian post-step baked into `AnimatedRenderedDecoder`** (separable Gaussian via `oxideav-image-filter::Blur` applied to rasterised RGBA buffer; kernel radius `ceil(3·sigma)` clamped to canvas's shorter side; `\be` stays on its own iterative-box channel per spec; `\t(0,T,\blur σ)` ramps monotonically; 237 tests) + r226 **`\iclip(rect)` + `\iclip(drawing)` baked into rasterised clip** (inverse-clip rectangle + path-fill subtraction landed in `AnimatedRenderedDecoder`'s clip composer) |

**Bitmap-native (own crate)** — `oxideav-sub-image`:

| Format              | Decode | Encode | Notes |
|---------------------|:------:|:------:|-------|
| **PGS / HDMV** (`.sup`) | ✅ | ✅ | Blu-ray subtitle stream; PCS/WDS/PDS/ODS + RLE + YCbCr palette → RGBA + r183 RLE codec property+negative sweep (1500 randomised roundtrips + edge cases) + r226 **PCS `composition_state` classified + routed to Packet keyframe flag** (Epoch-Start / Acquisition-Point recognised as random-access entries; downstream `Packet::keyframe` honours composition reset rule) |
| **DVB subtitles**   | ✅ | — | ETSI EN 300 743 segments + 2/4/8-bit pixel-coded objects |
| **VobSub** (`.idx`+`.sub`) | ✅ | — | DVD SPU with control commands + RLE + 16-colour palette + r201 SP_DCSQ 0x07 CHG_COLCON length-skip + r214 **CHG_COLCON application** (param payload parsed into `Vec<ChgColConBand { csln, ctln, entries: Vec<ChgColConEntry { start_col, palette_sel, alpha }> }>` exposed on `Spu::chg_colcon`; decoder applies per-pixel replacements during canvas paint with right-most-PX_CTLI-wins horizontally, last-band-wins vertically, sentinel-optional payload tolerated; absolute-display coords mapped via `(spu.x1, spu.y1)`; 85 lib tests) |

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

The `oxideav-id3` crate parses ID3v2.2 / v2.3 / v2.4 tags (whole-tag
and per-frame unsync, extended header with **CRC-32 [ISO-3309]
verification and emission** since r153, v2.4 data-length indicator,
encrypted/compressed frames recorded as `Unknown`, **r161 v2.4 §3.4
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

The `oxideav-job` crate is a declarative way to describe multi-output
transcode pipelines. A job is a JSON object: keys are output
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

- `scripts/update-crates.sh` — clones every missing OxideAV sibling. Idempotent; safe to re-run.
- `scripts/update-crates.sh` — clones the missing ones AND fast-forwards already-cloned siblings to upstream tip via a single GraphQL call. Skips siblings whose upstream is already an ancestor of local HEAD and refuses to fast-forward when local commits have diverged, so in-progress work is preserved.

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
