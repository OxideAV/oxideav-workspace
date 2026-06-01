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
  selection, AGL glyph-name→Unicode, full `name`-table accessor API), `oxideav-otf` (CFF / Type 2 charstrings incl. CID-keyed ROS/FDArray/FDSelect + arithmetic/stack/storage/conditional ops + Top-DICT FontMatrix/PaintType/CharstringType/StrokeWidth, ISOAdobe/Expert/ExpertSubset predefined charsets, cubic outlines),
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
| WAV       | ✅ | ✅ | ✅ | LIST/INFO metadata; byte-offset seek; BWF `bext` metadata (EBU 3285); r180 smpl + inst chunks + r190 plst Playlist + r193 `fact` chunk per RIFF MCI §3 (`dwFileSize` per-channel sample count → authoritative `StreamInfo::duration` for non-PCM; heuristic-vs-fact `wav:fact.mismatch` diagnostic; muxer emits `fact` between `fmt`/`data` for non-PCM `wFormatTag` per spec; PCM byte-identical pre-r193) |
| FLAC      | ✅ | ✅ | ✅ | VORBIS_COMMENT, streaminfo, PICTURE block; SEEKTABLE-based seek; CUESHEET round-trip (read + write per RFC 9639 §8.7); r182 in-place symmetric-pair Levinson-Durbin update (encoder, eliminates up to 36 Vec allocs/subframe, bit-exact regression-pinned) |
| Ogg       | ✅ | ✅ | ✅ | Vorbis/Opus/Theora/Speex pages + comments; page-granule bisection + page-level seek index + chained-link-aware duration + page-loss/hole detection + page-sync recapture + public CRC-32 validation API + r172 Criterion bench harness + r183 streaming CRC + r185 Skeleton 3.0/4.0 + r192 slice-by-4 CRC-32 + branch-free `compute_page_checksum` (3-segment dispatch drops 65535 branches from max-size page; **page/parse/max 493 MiB/s → 1.3 GiB/s = 2.5-3× over r172**) + r196 Skeleton 4.0 `index` packet index-accelerated `seek_to` (per-stream keyframe index pre-walk skips O(N) bisection on indexed streams) |
| Matroska  | ✅ | ✅ | ✅ | MKV/MKA/MKS; Cues seek; SeekHead/Chapters/Attachments/subtitles; opt-in block lacing on write; EBML CRC-32 validation (r186 per-Cluster CRC-32 now validated on advance() + Cue-driven seek, dedup via HashSet, RFC 8794 §11.3.1 + RFC 9559 §6.2); typed Tag/TrackOperation/ContentEncodings/chapters() decode; typed Video FlagInterlaced/FieldOrder + geometry quartet + Colour master + SMPTE 2086 MasteringMetadata + StereoMode + r177 Projection + r183 AlphaMode / AspectRatioType / UncompressedFourCC typed decode + 16-test injection-robustness pin + r196 §5.1.6 write-side Attachments + r202 §6.2 write-side CRC-32 on Top-Level masters (Info/Tracks/Cues/Chapters/Attachments now carry 6-byte CRC-32 child id 0xBF; late-Cues rescan path also CRC-validated; SeekHead + Cluster intentionally pinned without CRC; 192 tests) |
| WebM      | ✅ | ✅ | ✅ | First-class: separate fourcc, codec whitelist (VP8/VP9/AV1/Vorbis/Opus); inherits Matroska Cues seek |
| MP4       | ✅ | ✅ | ✅ | mp4/ismv; faststart; iTunes ilst; fragmented demux+mux (DASH/HLS/CMAF) + sidx/mfra/tfra/styp; AC-3/E-AC-3/DTS sample entries; subtitle/timed-text; protected sample-entry unwrap; typed track refs + edts/elst mux + elng + kind + cslg + stsh + sdtp + sample-group sbgp/sgpd + §8.16.5 prft demux + r162 atom-walker robustness + r182 sidx-driven seek fast-path + r189 `read_box_header` largesize overflow reject + r196 ISO/IEC 23001-7 §8 CENC parser + r203 §8.7.8-9 `saiz`/`saio` Sample Auxiliary Information parser (track + traf surfaces; 32-bit/64-bit `saio` offsets widened to u64; structured `Mp4Demuxer::sai_records()` typed accessor for CENC consumers; 10 new tests = 179 lib); lacks AES-CTR/CBC decryption driver |
| MOV (QuickTime) | ✅ | — | ✅ | Apple QTFF + ISO BMFF meta + HEIF/HEIC item-properties + grid/iovl/tmap + symmetric muxer + fragmented-MP4 seek + DASH sidx/styp + stbl + traf saiz/saio sample-aux + r182 ISO 14496-12 §4.2/§11.1 `uuid` User-Type Box parser + r187 largesize overflow reject + r199 §8.3.4 `trgr` Track Group Box + r204 §8.7.3.3 **`stz2` Compact Sample Size Box** at stbl scope (Mp4Box-style 4/8/16-bits-per-entry packing; entries widen to `u32` into `SampleTable::stsz_table` so all downstream consumers work unchanged; `SampleSizeSource::{Stsz, Stz2 { field_size }}` discriminator + `MovDemuxer::sample_size_source(track_index)`; field_size ≠ 4/8/16 rejected per §8.7.3.3.2, non-zero 24-bit `reserved` rejected per §8.7.3.3.1, MSB-first 4-bit packing with trailing-nibble silent drop; first-wins on malformed both-stsz-and-stz2; 10 new tests = 468 lib); ffprobe-accepted |
| AVI       | ✅ | ✅ | ✅ | AVI 1.0 + OpenDML 2.0 demux/mux; AVIX/dmlh/vprp + 2-field interlaced + VBR audio + LIST INFO + typed PaletteChange/TextChunk/AvihFlags/Idx1Flags + r197 OpenDML AVISUPERINDEX `bIndexSubType` surface (`super_index_sub_type` / `super_index_is_2field` / `avi:indx.<n>.sub_type_2field` metadata; AVI_INDEX_SUB_2FIELD == 0x01) + ODML keyframe seek + WAVEFORMATEXTENSIBLE + `strn`/`strd` + CBR-audio validator + dmlh.dwTotalFrames + IDIT/ISMP/rcFrame/wLanguage + dwInitialFrames + r163 typed `dwChannelMask`/`Speaker`/`ChannelLayout` + r182 typed `strh.wPriority` + r203 per-stream `strh.dwStart` (32-bit DWORD at AVISTREAMHEADER byte 28; `AviDemuxer::stream_start(idx)` + `AviMuxOptions::with_stream_start(idx, start)`; default-0 → None mapping; 11 round-trip tests) |
| Blu-ray (BD-ROM) | ✅ | — | — | `oxideav-bluray` Phase 2 — UDF 2.50 mount (ECMA-167 3rd ed.) + BDMV walk (`index.bdmv`/`MovieObject.bdmv`/`.mpls`/`.clpi`) + `.m2ts` stream (192→188-byte TP_extra_header strip) + `bluray://` URI handler with auto-detect; r93 typed `Cpi { ep_map: Vec<EpMap { stream_pid, ep_stream_type, entries: Vec<EpEntry { pts_ep_start, spn_ep_start, is_angle_change_point, … }> }> }` CPI EP_map decode per BD-ROM AV §5.7 (coarse + fine two-level table folded into a flat per-PID list a seeker can binary-search); r96 keyframe-aligned `TitleSource::seek_to(pts_90k)` (PTS→clip→I-frame→SPN×192, AACS-unit-aligned); `StreamDecryptor` trait hooks `oxideav-aacs` without hard dep. + r180 multi-angle PlayItem parsing (BD-ROM Part 3 §5.4.4.1) + `open_title_with_angle` / `max_angle` per-angle title open (AV §5.2.3.3) + r188 `Disc::chapters(title)` from PlayListMark entry marks + r200 `Disc::title_streams(title) -> TrackCatalogue` deduplicating per-PlayItem STN_table entries by `(PID, kind)` (AV §5.2.3.3 / Part 3 §5.4.4.4) + mount-time `TitleInfo::languages` from audio/subtitle entries (133 tests). Lacks HDMV opcode exec, BD-J, mid-stream angle switching, cross-PlayItem STC PTS remap |
| DVD-Video | ✅ | — | — | `oxideav-dvd` Phase 3b — ISO 9660 + UDF 1.02 mount + VIDEO_TS walk + IFO body parser (VMGI/VTSI + TT_SRPT + VTS_PTT_SRPT + PGCI [+ PGC subpicture colour-LUT + pre/post/cell nav command table] + VTS_C_ADT + chapter materialiser) + VOB demux (MPEG-PS pack/PES + Nav-Pack PCI/DSI [+ PCI highlight + DSI typed sections] + DVD substream router for AC-3/DTS/LPCM/subpicture) + VOB → MKV mux (`mkv-output` feature; per-PES PTS preserved + ChapterAtom per `DvdChapter` via RFC 9559 §5.1.7) + `dvd://` URI handler + r172 typed NavInstruction VM disassembler (Phase 3c precursor: full Link family + 13-entry link-subset + Jump/Call SS + Set arithmetic + Type 4..6 classifier). + r179 Sub-Picture Unit (SPU) decoder (SPUH+DCSQT walker, 8 typed commands, 2-bit/four-form PXD RLE, 90 kHz STM-DTS conversion) + r188 SPU RGBA compositor (`composite()`: SET_COLOR/SET_CONTR → PGC palette LUT → BT.601 studio-swing YCbCr→RGB + top/bottom-field PXD interleave) + r200 Phase 3c VM execution (RegisterFile w/ SPRM defaults + RSM call/return stack + `step()/run_list()` honoring Goto/Break/Exit with step-budget; SET-arithmetic + 7 CmpOps + 12 SetOps; typed `VmAction { Link/Jump/Call/Resume/Exit/Break/NoOpRaw }`; 163 tests). Lacks Type 4..6 compound SET-then-CMP-then-LNK ordering, CSS auth (`oxideav-css`) |
| MP3       | ✅ | — | ✅ | demuxer LANDED (ID3v2/ID3v1 skip + Xing/Info VBR + CBR/VBR seek_to); r177 Decoder-trait stereo widening (independent + joint MS + intensity, planar AudioFrame) |
| IFF (EA IFF 85) | ✅ | ✅ | — | One crate for the whole `FORM/LIST/CAT` family — Amiga `8SVX` audio + `ILBM` images (1..8-plane indexed + 24-bit literal-RGB true-colour, EHB/HAM6/HAM8, ByteRun1, HasMask, GRAB, SHAM, PCHG; CRNG/CCRT/DRNG `cycle_step`) + `ANIM` (op-0 literal + op-5 vertical-delta encode/decode + r192 op-7 Short/Long Vertical Delta decode) + Apple `AIFF / AIFF-C` (FORM/COMM/SSND walker, 80-bit IEEE-extended sample-rate decode, NONE/twos/sowt/raw/fl32/FL32/fl64/FL64 PCM, codec-bearing FourCCs ima4/ulaw/alaw routed to sibling crates) + r198 §6.0 AIFF MARK chunk parsing + r203 §9 AIFF `INST` (Instrument) chunk parsing (`InstrumentChunk { baseNote / detune / low+highNote / low+highVelocity / gain / sustainLoop / releaseLoop }` + `PlayMode { NoLooping / ForwardLooping / ForwardBackwardLooping }` + `resolve_sustain_loop`/`resolve_release_loop` join against MARK with begin<end ordering guard; MIDI 0..=127 + detune -50..=+50 + velocity 1..=127 validation; +22 tests = 244 lib); lacks ANIM op-7 encode + op-8, DEEP true-colour, COMT/AESD/APPL surfacing + MARK/INST write side |
| IVF       | ✅ | — | — | VP8 elementary stream container |
| AMV       | ✅ | ✅ | — | Chinese MP4 player format (RIFF-like) — r191 clean-room demuxer rebuilt from `docs/container/amv/amv-container-trace.md`: position-coded RIFF prelude + `amvh` packed `[s,m,h,0]` duration + 20-byte WAVEFORMATEX + §4 no-byte-padding chunk walker + two-stream Demuxer (video=mjpeg, audio=adpcm_amv) + `AMV_END_` trailer + r197 byte-faithful `AmvMuxer` + r203 `Demuxer::seek_to` (linear walk over `movi` since AMV has no `idx1`/OpenDML index per trace §1 quirk #2; rewind-on-backwards + forward-walk-from-cursor; cumulative §4b PTS for audio stream; chunk bodies skipped via `Seek`; 39 tests w/ real-fixture comedian.amv frame-500 SOI verification) |
| FLV       | ✅ | ✅ | — | Flash Video — MP3/AAC/H.264 audio + VP6f/VP6a/H.264 video + Enhanced RTMP ExVideoTagHeader + AMF0 onMetaData/onXMPData/onCuePoint + Annex F encryption + E-FLV ModEx walk + multitrack body splitter + HDR colorInfo metadata + r161 injection-robustness suite + 16 MB OOM-lever guard + r182 onMetaData catch-all preserves Date/Null/StrictArray/AMF3-nested + r186 unknown-script-name argument preservation + r196 first muxer slice (audio-only) + r202 §E.4.3 / §E.4.3.1 video-tag muxer slice (write_h263_tag + write_vp6_tag + write_vp6a_tag w/ AlphaOffset + write_avc_sequence_header + write_avc_nalu_tag w/ SI24 CompositionTime + write_avc_end_of_sequence + VideoTagHeader↔byte round-trips; 241 tests) |
| WebP      | ✅ | ✅ | — | RIFF/WEBP (lossy + lossless + animation; ANIM + ANMF emit) |
| TIFF      | ✅ | ✅ | — | TIFF 6.0 single-image + r177 BigTIFF write (magic 43 / 8-byte offsets / LONG8 strip+tile arrays) + r183 PhotometricInterpretation=8 1976 CIE L*a*b* decode + r185 CCITT T.4 2-D + T.6 (Group 4) fax decode (READ algorithm; tiffcp-oracle pixel-exact) |
| PNG / APNG| ✅ | ✅ | — | 8 + 16-bit, all color types, APNG animation + r188 gAMA/cHRM round-trip + r202 §4.2.10 zTXt compressed-textual-data round-trip (PNG3 §11.3.3.3; deflate body + compression-method=0 enforced + 166 tests); metadata lacks only iCCP/iTXt |
| GIF       | ✅ | ✅ | — | GIF87a/GIF89a, LZW, animation + NETSCAPE2.0 loop + multi-frame compositor (§23 disposal-method state machine, 4 modes) + r181 `GifImage::frames_with_palette` §21 active-table iterator + r188 §23 `has_transparency()` / `requires_user_input()` stream-level GCE flag queries — clean-room rebuilt from CompuServe spec |
| JPEG      | ✅ | ✅ | — | Still-image wrapper around the MJPEG codec |
| BMP       | ✅ | ✅ | — | Windows bitmap — DIB headers BITMAPINFOHEADER / V4 / V5, 1/4/8/16/24/32-bit + r182 BI_ALPHABITFIELDS (compression=6, V3 four-mask alpha variant); also exposes the DIB helpers used by ICO / CUR sub-images |
| Netpbm    | ✅ | ✅ | — | All seven PNM magics + PAM (P1-P7); 1/8/16-bit; comment-tolerant ASCII + binary; .pbm/.pgm/.ppm/.pnm/.pam + r183 user-defined PAM TUPLTYPE + r189 ASCII (P1/P2/P3) hot-path rewrite (stack-buffer digit writer + u8-direct emitters + checked u32 accumulator: encode P1 7.3→139 MiB/s ×19, P2 60→322 MiB/s ×5.4, P3 58→295 MiB/s ×5.1) |
| ICO / CUR | ✅ | ✅ | — | Windows icon + cursor — multi-resolution, BMP and PNG sub-images; r178 body-dim `(0,256]` reject + r184 CUR hotspot body-derived bound (closes fuzz hotspot probe-vs-render panic) |
| slin      | ✅ | ✅ | — | Asterisk raw-PCM: .sln/.slin/.sln8..192 |
| MOD / S3M / STM | ✅ | — | — | Tracker modules (decode-only by design; STM structural-parse only; r186 XM vol-col panning-slide + r192 XM instrument auto-vibrato `vibrato_type` byte selector + `+4` "don't retrigger" flag via `waveform_lfo(type & 3, pos>>2)` shared with E4x/E7x — closes hardcoded SINE_TABLE gap) |

Cross-container remux works for any pair whose codecs don't require
rewriting (FLAC ↔ MKV, Ogg ↔ MKV, MP4 ↔ MOV, etc.).

### Content protection

| Layer | Status | Notes |
|-------|:-------|-------|
| AACS  | ✅ Common 0.953 + BD-Prerecorded 0.953 | `oxideav-aacs` clean-room — KEYDB.cfg parser, `MKB_RO.inf` / `Unit_Key_RO.inf` parsers, Subset-Difference tree walk, Device-Key → Processing-Key → Media-Key → VUK derivation, AES-128-CBC Aligned Unit decryption, Title Key unwrap + Phase B SCSI MMC drive-command wire layer (REPORT_KEY / SEND_KEY / READ_DISC_STRUCTURE typed CDBs + AGID / Drive-Cert-Challenge / Drive-Key / Host-Cert-Challenge / Host-Key / Volume-ID sub-payload codecs + `DriveCommand` trait + `MockDrive` synthetic-fixture impl) + Phase C Drive-Host AKE (clean-room ECDSA over the AACS 160-bit curve + FIPS 180-2 SHA-1 + AES-128-CMAC; `host_authenticate` §4.3 state machine + `DriveAuthState` wired into `MockDrive`; Bus Key = lsb_128 of shared ECDH x-coord; §4.4 Volume-ID transfer w/ CMAC verify). + r177 READ_DISC_STRUCTURE Format 0x81 / 0x82 / 0x83 typed sub-payloads (PMSN, Media-ID, MKB-pack body up to 32 KiB; CMAC verify per §4.5/§4.6/§4.14.3.4; MockDrive serves Format 0x81/0x82). + r183 MKB ECDSA verify §3.2.5.1.2/.3/.8 (host/drive revocation list + end-of-block signature; caller-supplied AACS LA pubkey) + r188 BD-Prerecorded §2.3 Content Hash Table + r200 `KEYDB::parse_with_report` structured `ParseReport` (1-based `line_number` + UTF-8-boundary-safe 80-byte `snippet` + `Display`-formatted `AacsError` `reason` per skipped line; `KeyDb::parse` unchanged) + 27-case fuzz/robustness suite (per-record-type malformations × scope rules × mixed CRLF/LF × printable-ASCII leader sweep × multi-byte UTF-8 × 10 KiB-long bad line; 193 tests). Lacks signed Content Certificate Table 2-1 verify, AACS 2.0 (UHD-BD) |

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
| **Vorbis** | 🚧 r9 (post-2026-05-20 orphan) — identification + comment + §3.2.1 codebook + Huffman tree + full §4.2.4 setup-header walker + §3.2.1/§3.3 VQ vector unpack + §8.6 residue decode (formats 0/1/2) + §7.2.3/§7.2.4 floor type 1 + §6.2.2/§6.2.3 floor type 0 LSP + §1.3.2/§4.3.1 Vorbis window + §4.3.5 inverse channel coupling + §4.3.3 nonzero-vector propagate + §4.3.6 floor×residue + §4.3.1–§4.3.8 audio-packet driver + r180 §4.3.7 IMDCT + r186 streaming overlap-add | 🚧 r2 — r195 §4.2.1+§4.2.2 identification-header WRITE + §5.2.1 comment-header WRITE + r201 §3.2.1 codebook WRITE + §9.2.2 `float32_pack` (bit-exact roundtrip across 3 length encodings × 3 lookup types; auto-picks densest sparse/ordered/dense; 378 tests); lacks §7.2 floor WRITE + §8.6 residue WRITE + audio-packet WRITE |
| **Opus** | 🚧 ~33% — RFC 6716 range decoder + full SILK pipeline + §4.3 Table 56 CELT pre-band header + §3.1/§4.2 framing dispatch + r182 celt_band_layout + r183 §4.3.4.3 spread + r190 §4.3.4.5 TF-resolution lookup + r191 §4.3.3 LOG2_FRAC_TABLE + intensity_rsv/reserve_stereo + r193 §4.5.1 CELT redundancy + r195 §4.5.2 SILK+CELT state-reset policy (`decide_state_resets` 4 rules → `StateReset {silk, celt: None/BeforeFrame/BeforeRedundantOnly}`; full 3×3 mode × redundancy matrix + §4.5.3 Figure 18 cross-checks; 467 + 20 tests); §4.5 mode-switching now §4.5.1 + §4.5.2 complete; §4.5 mode-switching now §4.5.1 + §4.5.2 complete + r200 §4.5.1.4 redundant-frame decode params + r204 **§4.3.2.1 CELT coarse-energy Laplace-model parameter surface** (`E_PROB_MODEL: [[[u8;42];2];4]` 336 Q8 bytes via `e_prob_pair(lm, mode, band)` + typed `EnergyPredictionMode::{Inter, Intra}` + intra `α=0` / `β=4915/32768` Q15 constants; 514 lib + 20 integration tests; per-LM inter-mode `(alpha, beta)` deferred as §4.3.2.1 docs gap "depend on the frame size in use"); CELT bands still gated on #936 | 🚧 scaffold |
| **MP1 / MP2** | ✅ Layer I + Layer II decode to PCM + §2.4.3.1 CRC-16 verify + mp2 frame-level decode loop + r191 Annex D Phase-2 psy + r203 Annex D Phase-3 Step 3 LTq offset + Model 2 spreading + (allocator still pending D.1/D.3/D.4 PNG→text #1262) | 🚧 ~82% — Layer I encoder + Layer II §C.1.5.2.7 bit-allocation + r192 §C.1.3 polyphase analysis filterbank + r197 §C.1.5.1.4 Layer II per-part scalefactor extraction; lacks top-level `Mp1Encoder` Layer II switch + Table C.4 SCFSI selection (PNG→text gap) |
| **MP2** | 🚧 ~35% (post-2026-05-24 orphan) — §2.4.1.3/§2.4.2.3 Layer II header parser + §2.4.3.1 frame sizing + Annex B Table 3-B.1/3-B.2a..d/3-B.4 + joint-stereo allocation + scfsi + §2.4.3.3.4 sample requantizer + §2.4.3.1 CRC-16 + r162 malformed-input property suite + r185 full LSF Layer II wiring + r202 §C.1.5.2.5/§C.1.5.2.6 SCFSI Table C.4 encoder-side selection (`select_scfsi` adjusted-`used` triple + `TransmissionPattern` + 2-bit code matching `audio_data::Scfsi`; ~5-class dscf classification per page 73; 198 tests); lacks §2.4.3.2 polyphase synthesis + iterative bit-allocator + audio-data writer | 🚧 scaffold |
| **MP3** | ✅ ~100% — bit-exact decode + ID3v2/Xing seek + MPEG-2.5 framing; 634 tests | 🚧 ~94% — Phase-2 + r194 long + r197 pure-short + r204 **mixed-block per-band threshold-in-quiet path** (`outer_loop_search_mixed_per_band` consumes split `xmin_long[sfb]` + `xmin_short[sfb][win]`; closes the long/short/mixed dispatcher trio so per-band LTq is end-to-end; existing `outer_loop_search_mixed` refactored to a scalar shim, byte-identical regression-anchored; 671 tests); lacks Model 1/2 psy + intensity-stereo |
| **AAC** | 🚧 Phase 1 — ADTS + raw_data_block walker + AudioSpecificConfig + program_config_element + r177 §4.4.1 GASpecificConfig extensionFlag + Table 1.15 epConfig + r192 §1.6.5 Table 1.15 trailing `syncExtensionType=0x2b7` implicit-SBR/PS/ER-BSAC probe (`AudioSpecificConfig.trailing_sbr_probe`; ext-AOT 5 reads sbrPresentFlag + optional 4-bit ext-sfi w/ 24-bit escape + secondary `0x548` sync gating psPresentFlag; ext-AOT 22 reads sbrPresentFlag + mandatory 4-bit ext-channel-config; `parse_bits_bounded` for LATM/esds carrier-bounded callers) + r194 §4.5.4.1 SWB offset tables (`SWB_OFFSET_LONG_WINDOW[13]` / `SWB_OFFSET_SHORT_WINDOW[13]` from Tables 4.129-4.141; `long_window_offsets`/`short_window_offsets` accessors) + §4.6.13 `apply_pulse_data` reconstruction + r200 §4.6.9.4 TNS_MAX_ORDER/BANDS clamp surface (Tables 4.102/4.103 + AAC-LD Tables 4.119/4.120 from §4.6.17.2.5; `clamp_tns_order`/`clamp_tns_band` accessors; cross-module invariant `tns_max_bands ≤ num_swb_*_window` for every AOT×fs combo; 488 tests); decoder body still pending Huffman codebooks 1-11 + channel-element body walker | 🚧 scaffold — Phase-2 writers: section_data + ics_info + pulse_data + tns_data + scale_factor_data + DPCM + r160 raw_data_block + r165 Pce::write + r183 gain_control_data SSR + r187 §4.4.2.7 extension_payload; SBR types pending QMF |
| **CELT** | 🚧 r11 (post-2026-05-20 orphan) — RFC 6716 §4.1 range decoder + §4.3 prefix + §4.3.2.1 coarse-energy scaffold + §4.3.3 bit-allocation fields + §4.3.4 tf_change/tf_select + r181 §4.3.4.3 spread + r187 §4.3.7.1 post-filter + §4.3.7.2 de-emphasis + r195 §4.3.4.5 Walsh-Hadamard primitives + r200 §4.3.3 `cache_caps50` + dynamic-band-boost decode loop (closes #943 docs-gap; per-band cap[] derived via `cap = (cache.caps[i] + 64) * channels * N / 4`; `decode_band_boosts` literal §4.3.3 prose; 178 tests); blocked on docs #936 (Laplace) | 🚧 scaffold |
| **Speex** | 🚧 ~25% — Ogg stream-header + NB + WB high-band + §5.5 in-band signalling + r179 `BitWriter` + r187 encoder-side `write` + r191 22 CELP companion-table accessors + r194 NB LSP-VQ → Q10 LSP reconstruction + r200 §9.1 per-sub-frame LSP linear interpolation (`NbSubFrameLsp` producing `[[i32;10];4]` Q12 matrix per frame via 3/4+1/4, 2/4+2/4, 1/4+3/4, 4/4 weights; downstream LSP→LPC ready); 189 tests; lacks §9.1 LSP→LPC + synthesis + UWB framing | 🚧 scaffold |
| **GSM 06.10** | 🚧 ~28% — r185 clean-room §5.3 fixed-point RPE-LTP decoder pipeline + r200 §4.4 in-band decoder-homing protocol (matching decoder-homing input → 160×`0x0008` encoder-homing PCM output + §4.6 state reset; `DecoderState::decode_frame_with_homing` + `encoder_homing_frame_pcm()` + `is_decoder_homing_frame()` predicates) + r200 §5.1 `norm`/`div` saturating primitives staged for encoder slice; 47 tests; per-container framing for `.gsm` / RTP / MS-GSM WAV still DOCS-GAP; lacks §6 conformance vectors + encoder | 🚧 scaffold |
| **G.711** (μ/A-law) | ✅ 100% | ✅ 100% |
| **G.722** | 🚧 r185 clean-room SB-ADPCM decoder bring-up against staged ITU-T G.722 Recommendation (Table-14 column tables Q6/QQ6/etc. from the spec, not C reference) + r200 BLOCK1/QMF predictor split into shared `src/predictor.rs` | 🚧 r200 SB-ADPCM **encoder bring-up** — 24-tap transmit QMF (clause 3.1) + 60-level QUANTL + 4-level QUANTH (clause 6.2.1.1 with Note-2 LDL==LDU row-exclusion) + 64 kbit/s octet multiplexer (clause 1.4.4) + Tables 16/20 forward output codes; 31 unit + 1 doc-test; encoder→decoder silence-envelope green; lacks Appendix-II conformance fixtures |
| **G.723.1** | ✅ 100% | ✅ 100% — both 5.3k + 6.3k |
| **G.728** | 🚧 ~10% — clean-room decoder front-end: Annex A/B/C/D tables + block-50 Levinson-Durbin + blocks 29/31/32 + r195 blocks 30/33 backward synthesis-filter + vector-gain adapters + r201 blocks 73-77 postfilter AGC tail (§4.6 unity-DC lowpass `H(z)=0.01/(1−0.99·z⁻¹)` + per-vector Σ|sd|/Σ|sf| ratio + §4.6.1 sf=sd passthrough is bit-exact vs `decode_vector`; `decode_vector_postfiltered`; 81 tests); lacks postfilter blocks 71 (long-term/pitch) + 72 (short-term) | 🚧 scaffold |
| **G.729** | 🚧 ~6% — clean-room from staged trace #859: r173 numeric tables + r189 7 more tables + r191 ITU serial bitstream parser + conformance-corpus harness + r195 §3.2.4 LSP-quantiser L1 (128×10 Q13) + L2/L3 packed (32×10 Q13) codebooks with bounds-checked lookups + corpus harness validating every `LSP.BIT` frame's L1/L2/L3 indices lie in NC0/NC1 + r201 §3.2.4 MA-predictor `fg` family (2 modes × MA_NP=4 × M=10 Q15 cube + per-mode `Q15_ONE − Σ fg` factor + Q12 reciprocal; completes LSP-reconstruction inputs; 43 tests); lacks §3.2.4 reconstruction (rearrange + stability clamp + LP synthesis) + gain GA/GB + postfilter + Annex B DTX | 🚧 scaffold |
| **IMA-ADPCM (AMV)** | ✅ 100% | ✅ 100% |
| **MS-ADPCM / IMA-ADPCM (WAV)** | ✅ 100% | ✅ 100% — block-aligned WAV encoder for both nibble layouts |
| **OKI / Dialogic VOX** | ✅ 100% — r186 clean-room from Dialogic app note 00-1366-001 (1988); HiFirst (VOX/MSM6295) + LoFirst (MSM6258) nibble orders, Native12 + Wide16 output | ✅ 100% — symmetric §3 closed-form encode; mono-only via registry (Dialogic hardware constraint) |
| **8SVX** | ✅ 100% | ✅ 100% |
| **iLBC** (RFC 3951) | ✅ 100% — NB 20/30 ms | ✅ 100% |
| **AC-3 / AC-4** (Dolby Digital / Dolby AC-4) | ✅ ~97% — AC-3 full decode + E-AC-3 SPX + TPNP + AHT + §7.8.2 LtRt downmix + r126 Annex D mix-level + WAVE_FORMAT_EXTENSIBLE + r172 SPX-attenuation border + r182 §7.10.1 CRC verifier + r187 §7.10.1 augmented crc2 + r193 typed `BitStreamMode` accessor for Table 5.7 + r196 §E.2.3.1.8 E-AC-3 `chanmap` routing + r202 §7.7.2.2 typed `CompressionGain` (Table 7.30 `compr`/`compr2` 8-bit byte → `x: i8` + `y: u8` w/ `linear()`/`decibels()`; `Bsi::compr` + `Bsi::compr_ch2` lifted from parse-and-discard; mirrored on Annex E `Bsi`; 170 lib + 38 integ tests) | 🚧 AC-3 ~95% — acmod 1/2/2.1/3/6/7 + LFE + DBA + 5-fbw coupling + E-AC-3 indep+dep + per-channel PSNR gates + r95 two-stage equalise + spread-cap greedy for per-channel `fsnroffst[ch]` |
<!-- ac3 decode r129: E-AC-3 mixmdata mix-levels (ltrt/loro c/sur) now surfaced + routed through §7.8 downmix in process_eac3_frame -->
| **AC-4** (Dolby) | 🚧 ~98% — A-SPX + DRC + 60+ ETSI codebooks + 5_X/7_X ACPL_1/2/3 + cfg0/1/2/3 + LFE + SSF/SNF + SAP + Pseudocode 121 companding + IMS bitstream_version≥2 walker + r181 §5.7.7.7 Pseudocode 121 + r190 Table 126 `aspx_int_class = FIXFIX` writer width fix; lacks ETSI fixture RMS audit, object/a-joc | 🚧 IMS ~72% — v0/v2 TOC + mono/stereo/joint M/S + 5.0/5.1/7.1 SIMPLE Cfg3Five + 5_X SIMPLE/ASPX_ACPL_1/2 + ASPX_ACPL_3 + r132/r135/r139/r144 real per-band α+β for ACPL_1/2 + r193 real per-band β1/β2 for 5_X ASPX_ACPL_3 + r196 real per-band α1/α2 for 5_X ASPX_ACPL_3 + r202 **real per-parameter-band α + β for 7.0/7.1 SIMPLE/ASPX_ACPL_2** (§4.2.6.14 Table 33 `case ASPX_ACPL_2:` + §5.7.7.5 Pseudocode 116 + §5.7.7.6.1 Pseudocode 117; `encode_frame_pcm_7_{0,1}_acpl2_real_alpha_beta` 7-/8-ch entry points; D0 module L→Ls, D1 module R→Rs; LFE rides round-80 mono path; 805 tests); lacks γ + ASPX envelope coding |
| **MIDI** (SMF) | ✅ ~99% — SMF Type 0/1/2 → PCM via 32-voice mixer + SF2/SFZ/DLS + r186 `cue_points()` FF 07 + r192 `track_names()` FF 03 + r196 `instrument_names()` + r202 `texts()` FF 01 + `copyrights()` FF 02 (closes SMF FF 01..07 text-meta family at 10 iterators; 366 tests); r172 cargo-fuzz (30M+ panic-free) | — synthesis only |
| **NSF** (NES) | 🚧 ~96% — full 6502 + IRQ/NMI + 5/5 2A03 APU + DMC DMA + six expansion chips + NSF v1/v2/NSFe + Dendy region + r154 Namco 163 + r185 VRC7 OPLL pipeline + r199 VRC7 register semantics + r204 **VRC7 KSR (Key Scale of RATE)** per YM2413 §III-1-2 Table III-2 (`Envelope::update_rks(block, fnum_msb)` cached RKS: KSR=0 → `block >> 1`; KSR=1 → `(block << 1) \| fnum_msb`; 4-bit per-stage R widens to 6-bit RATE = 4·R + RKS via `Envelope::effective_rate(r)` with explicit R=0→RATE=0 halt carve-out; pitch-only `$1X`/`$2X` writes trigger mid-note `refresh_rks` glide; 213 tests). Lacks §4 KSL + §7 per-rate env tables (provenance-pending) + rhythm mode | — synthesis only |
| **Shorten** (.shn) | 🚧 r13 (post-2026-05-18 orphan) — `ajkg` magic + v2/v3 ulong + svar(n) + per-block function dispatch + VERBATIM/QUIT + DIFF0..3 + Rice residual + per-channel carry + spec/05 §2.5 running mean + QLPC predictor + r7 `decode_stream` + r145 `Decoder` trait + r181 block-by-block + r187 streaming `Decoder` + r191 envelope encoder surface + r197 **`write_diff0_block` predictor encoder** (full `<fn=0> <energy> <residual>×bs` command per spec/03 §3.1 + spec/05 §3.1; `min_energy_for_diff0` selector; encode→decode round-trips byte-exact through `decode_stream` across DIFF0+VERBATIM splice, silent block, ±100 max-natural residuals; +15 tests = 203); lacks DIFF1..3/QLPC predictor encoders + #1267 spec/04 §2 BLOCK_FN_QUIT contradiction | 🚧 scaffold |
| **TTA** (True Audio) | ✅ ~98% — TTA1 fmt=1/2 + password + ID3v1/APEv2 trailer + r187 streaming + random-access decode API + r198 streaming bench parameter-cube + r204 **`Decoder::new_with_password` brings streaming + random-access onto format=2 streams** (ECMA-182 CRC-64 digest from `spec/07` §3.2 + Stage-A `qm[0..7]` priming at every per-channel frame init per §3.5–§3.6; format=1 transparent alias via clear_priming; +2 bench cube cells at ~138–140 MiB/s matching fmt=1; 96 lib tests) | ✅ ~96% — TTA1 fmt=1/2 + password; bit-exact self-roundtrip |
| **APE** (Monkey's Audio) | 🚧 r190 Phase 1 — 8-byte `MAC ` magic + decimal-coded version + 5 compression-level enum prefix parser; per-version header tail + IIR coefficients + residual `k` recurrence + range-decoder bounds + channel decorrelation all DOCS-GAP | 🚧 scaffold |
| **Musepack** | 🚧 r197 — SV7 §2.5/§2.6 requantiser constants + SV7/SV8 stream-magic recognisers + SV8 packet outer-frame walker + r197 SV7 mpc_huffman tables + CNS PRNG + r201 SV7 §2.5 per-band sample-decode dispatcher (`BandDecodeCase` classifier covers all 18 spec cases; Cns=−1 / Empty=0 / HuffmanPerSample=3..=7 / PcmEscape=8..=17 live; Grouped1/2 + SV8 canonical-Huffman walk surface as DOCS-GAP via `Error::UnsupportedBandType(i8)` per #1323; 67 tests); lacks SV7 fixed-header field map + SV8 canonical-Huffman entropy layer + 32-band synthesis | 🚧 scaffold |
| **Cook** (RealMedia) | 🚧 r4 — flavor table + cookie parser + 8 DSP parameter tables + r194 open-time `DecodeConfig` (cookie ↔ flavor cross-check + sub-packet accounting) + r197 wire-level real-stream integration test + r203 cookie→flavor multi-match API (`iter_flavor_records` + `flavor_indices_matching_cookie(&CookCookie)` returns every record whose 4 cookie-checkable fields agree — cookie lacks `frame_bytes`/`sample_rate_hz`/`coupling_mode` so 21+22 both match on the real fixture; 41 tests); lacks bitstream decode | — |
| **WMA** | 🚧 r4 — patent-disclosed primitives (r197 mid-side stereo + run/level walker) + r203 **quantization-matrix differential coding + entropy-mode selector** (`qmatrix.rs` differential walker + `entropy_mode.rs` per-band entropy-coder choice; +818 LOC across 2 new modules); lacks codeword Huffman tables / exponent partition / LSP codebook / sign-bit layout / escape coding (`[GAP]` per docs) | — |
| **WavPack** | 🚧 ~82% (post-2026-05-18 orphan) — v4 block/metadata/decorrelation/entropy parse + LSB bit-reader + Golomb (base,add) interval + r186 `parse_block` aggregate + r191 `AdaptiveMedians` §3.2 + r194 **first PCM-producing API** `decode_packed_samples_mono` + r199 stereo per-sample decode loop + r201 `EntropyInfo→AdaptiveMedians` bridges + `from_entropy` wrappers (`EntropyInfo::stereo`, channel-indexed `AdaptiveMedians::from_entropy`, `stereo_pair_from_entropy`, top-level `decode_packed_samples_{mono,stereo}_from_entropy`; eliminates hand-rolled per-channel seed extraction; new `InvalidEntropyInfoForMono` / `InvalidEntropyInfoForStereo` malformed-input arms; 272 tests); lacks hybrid 0x0B+0x0C / float / multichannel / CRC / encoder | 🚧 scaffold |
| **APE** (Monkey's Audio) | 🚧 r190 Phase 1 bootstrap (new crate) — 8-byte `MAC ` magic + decimal-coded version u16 + 5 compression-level enum (1000/2000/3000/4000/5000) prefix parser; 14 unit + 6 integration tests + standalone-build OK; per-version header tail (sound params/frame count/seek table/embedded WAV) + IIR coefficients + residual `k` recurrence + range-decoder bounds + channel decorrelation reconstruction all DOCS-GAP | 🚧 scaffold |
| **DTS** (Core) | 🚧 ~38% — frame-sync header + 14↔16-bit pack/unpack + r192 `iter_frames_14bit` 14-bit container iterator + r195 §5.4.1 ABITS/SCALES side-info decoders + Annex D §D.5.6 12-level BHUFF codebooks (A12/B12/C12/D12/E12) + §D.5.3/§D.5.4 small-Huffman codebooks (A5/B5/C5/A7/B7) routed to SA129..SE129 difference symbols for SHUFF=0..4 + §D.1.1 `RMS_6BIT[64]` + §D.1.2 `RMS_7BIT[128]` + r202 §5.3 SFREQ/AMODE/PCMR typed resolvers (Tables 5-5/5-4/5-17: `SampleFrequency` 9 fixed + 7 reserved; `AmodeArrangement` 16 standard + UserDefined; `SourcePcmResolution` 6 valid + 2 invalid; 235 tests); lacks subframe walker + §5.4 polyphase filterbank + DIALNORM | — |
| **aptX** (classic + HD) | 🚧 ~70% — 4-band QMF + ADPCM; bit-exact NDA-blocked + r189 RFC 2361 §A.24 `WAVE_FORMAT_TAG_APTX = 0x0025` IANA tag + `CODEC_ID_STR = "aptx"` registry (lets RIFF containers route 0x0025 → clean NotImplemented) | — |

</details>

<details>
<summary><strong>Video</strong> (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **MJPEG** | ✅ ~97% — baseline + progressive 4:2:0/4:2:2/4:4:4/grey + 12-bit YUV (baseline + r183 SOF2 P=12 progressive) + SOF9 arithmetic + lossless SOF3 + RFC 2435 RTP/JPEG depacketization + r190 §G.1.1 SOF2 4-component CMYK / YCCK progressive at P=8 + r197 **8th cargo-fuzz target `arith_decode`** wraps fuzz bytes in SOF9 envelope to drive `ArithDecoder` Q-coder per-iteration coverage (`Initdec`/`Renorm_d`/`Byte_in`/`decode_dc_diff`/`decode_ac`/`decode_magnitude` + DRI restart + per-component stats) | ✅ ~96% — baseline + progressive + lossless SOF3 grey/RGB + DRI/RSTn + non-zero point transform Pt 0..15 + r193 public 4-component CMYK encoder |
| **FFV1** | 🚧 ~73% — RFC 9043 decoder + demux + decode_frame driver (YCbCr + RGB Y/Cb bit-exact; RGB Cr divergence open) + r179 `coder_type==2` alternative state-transition table wired through decode + encode | 🚧 ~96% — Slice Footer + Slice Header + Golomb-Rice primitives + frame-level Golomb-Rice + YCbCr encoder + r164 range-coded SliceContent encoder + r179 derived-table encode path + r190/r193 §4.7 RGB + RCT encode across all 3 `coder_type` branches + r196 unified `encode_frame` dispatch helper + r202 §4.2 Parameters + §4.1 Quantization Table Set cascade encoder (`encode_configuration_record_with_quant_tables`; §4.3.2 `configuration_record_crc_parity` solved via `CRC(M||CRC(M))==0`; 4 corpus extradata fixtures round-trip parse→encode→re-parse field-equal; 272 tests); lacks RGB Cr decode fix + §4.2.14 tail (#904) |
| **MPEG-1 video** | 🚧 ~42% — sequence/GOP/picture/slice + macroblock walk + intra-DC + §2.4.3.7 dct_coeff walker + §2.4.4 dequantiser + r185 §A 8×8 IDCT + IEEE P1180/D2 conformance + r194 §7.3 `mpeg2_inverse_scan` + r202 §6.2.6 MPEG-2 `block(i)` driver (`mpeg2_decode_block` chains §7.2.1 DC prelude → §7.2.2 residual VLC walker w/ FIRST/NEXT alternation → §7.3 inverse scan → §7.4 inverse-quant → §A 8×8 IDCT into one bitstream→`f[y][x]` entry; 677 tests) | 🚧 scaffold |
| **MPEG-2 video** | 🚧 ~62% — §6.2.x sequence/GOP/picture/slice + macroblock walk + §7.6.3.x PMV + §7.6.4-8 forming-predictions/combine/add-saturate + r165 §7.6 driver + r179 §7.4 inverse-quantisation + r185 §A 8×8 IDCT + r192 §7.2.2 residual VLC walker + r194 §7.3 `mpeg2_inverse_scan` + r199 §7.2.1 **intra-block DC prelude** (Tables B-12/B-13 extended to sizes 0..=11, `dc_dct_differential` → `dct_diff` `half_range` reconstruction, three-cell `dc_dct_pred[Y/Cb/Cr]` predictor state with Table 7-2 reset values, three-trigger reset contract slice-start/non-intra-MB/skipped-MB, `QFS[0] ∈ [0, 2^(8+intra_dc_precision)-1]` bitstream-constraint enforcement; 582 lib + 74 integ); lacks block-iterator driver chaining §7.2.1→§7.2.2→§7.3→§7.4→§A IDCT into `decode_intra_block` | 🚧 scaffold |
| **MPEG-4 Part 2** | 🚧 ~62% — I-VOP intra + inter texture + §6.2.5 video_packet_header + §7.8.7.3 GMC + r182 §7.6.2.1 half-sample bilinear + r190 §7.6.2.2 quarter-sample + Table 7-13 chroma MV reduction + r193 §7.6.9.5.2 B-VOP direct-mode MV derivation + r195 §7.6.9.5.3 B-VOP luminance prediction-block + r201 §7.6.5 chroma MV derivation `MVDCHR` (Tables 7.10–7.13 covering K∈{1,2,3,4} luma sub-block MVs; `chroma_mv_from_luma_blocks` with `i32::div_euclid` floor + fractional-rounding via per-K table; 532 lib + 7 doc tests); lacks B-VOP chroma MC plane + §7.6.1.6 padding + §6.2.6.2 MV-body parser wiring + encoder | 🚧 scaffold |
| **Theora** | 🚧 ~46% — §6.1–§6.4 setup-header + Appendix B.2/B.3 VP3-default tables + §6.4.x quant + DCT-token Huffman + §7.1–§7.5 frame walk + r160 §7.5 motion vectors + r179 §7.7.1 EOB Token decode + r185 §6.4.1 LFLIMS + r191 §7.7.2 Coefficient Token Decode + r195 §7.7.3 DCT Coefficient Decode driver + r201 §7.8.1 DC predictor compute (`compute_dc_predictor` per Table 7.46 reference-frame mapping + Table 7.47 weights with signed `[29, -26, 29]` triplet + 3-ref `LASTDC` fallback + DC2→DC0→DC1 outranging-guard order + truncated-toward-zero divide; 303 tests); lacks §7.8.2 DC prediction loop | 🚧 scaffold |
| **H.263** | 🚧 ~89% (post-2026-05-18 orphan) — §5.1-§5.4 baseline + §6 IDCT/MV/half-pel/INTER + Annex J §J.3 deblock + Annex I AIC + Annex D UMV + Annex F 4-MV + OBMC + §5.1.4 PLUSPTYPE + Annex K §K.2 SS + r187/r192 §I.3 AIC reconstruction pipeline + r196 §I.2/§I.3 AIC MB-grid driver wiring + r202 `decode_picture_layer` PLUSPTYPE entry-point (`plus_ptype_to_baseline_shim` validates supported-layer-set UFEP=001 + standardised source format + no custom-PCF/CPM/SAC/SS/IS/AIV/MQ/RRU + INTRA/INTER + UMV gated on UUI=1; OPPTYPE-signalled AIC + DF OR-merge into `DecodeOptions`; 378 tests); lacks Annex K driver + PB-frames + custom-format + UFEP=000 inherited-state | 🚧 scaffold |
| **H.261** | ✅ ~98% — I+P QCIF/CIF + integer-pel + loop filter + BCH FEC + Annex B HRD + RFC 4587 RTP + RFC 3550 RTCP SR/RR/SDES/BYE/APP; r189 §6.2.1 SDP offer/answer negotiation + r198 3rd cargo-fuzz target `decode_bch_multiframe` + r204 **4th target `parse_rtp_payload`** (RTP §5.1 fixed header + RFC 4587 §4.1 H.261 payload header SBIT/EBIT/I/V/GOBN/MBAP/QUANT/HMVD/VMVD + multi-packet `depacketize` bit-walker; 9-buffer seed corpus + stable-CI mirror; distinct from `decode_h261`/`parse_rtcp_compound`/`decode_bch_multiframe`) | ✅ ~98% — spiral+diamond ME + GQUANT-from-bitrate + BCH framing + RTP wrap + RTCP compound build/parse; 45 dB at 64 kbit/s QCIF |
| **MS-MPEG-4** (v1/v2/v3) | 🚧 ~44% — clean-room scaffold + r202 `Macroblock4MvDecoder` 4-MV-per-MB bitstream tests (4 integration tests pin picture-corner rule-4 + within-MB candidate chaining + four-zero-MVD rigid-motion + parallel-reader cross-check against `predict_block_mv`; 80 integration tests) + r181 `GFamily` accessors + r185 Figure 7-34 MV-predictor walk + r191 1-MV predictor routed through `predict_block_mv(Block::TopLeft, …)` + r196 §7.6.5 **4-MV-per-MB batch predictor** (`predict_block_mv_all_four(MbContext)` returns `[Mv; 4]` for `Block::{TopLeft, TopRight, BottomLeft, BottomRight}` in one pass — single-MB neighbour-fetch shared across sub-blocks; closes r185 CHANGELOG queue for 4-MV path; matches per-block one-shot bit-exactly). Still lacks G0..G3 primary canonical-Huffman bit-length array + alt-MV VLC + 4-MV MCBPC. VfW-sandboxed mpg4c32.dll runs in parallel | — |
| **H.264** | 🚧 ~80% — I/P/B + 4:2:0/4:2:2/4:4:4 + CAVLC + CABAC + DPB + 41 SEI types + fuzz-hardened + r183 SEI type 46 + r187 §8.2.1 POC i64-staged + `PocError::Overflow` + r192 ISO/IEC 14496-15 §5.2.4.1.1 strict avcC parser + High-family extension trailer (rejects `lengthSizeMinusOne == 2` up front; parses chroma_format/bit_depth + SPS-Ext list; §7.4.2.1.1 cap) + r194 §7.3.5.3.1 CAVLC call-contract guards + r200 Annex G MVC SEI types 39 `multiview_scene_info` (`max_disparity` ue(v) 0..=1023) + 43 `operation_point_not_present` (anti-OOM 65536 cap before `Vec::with_capacity`; same shape as r177 §D.2.20 fix); 43 SEI types implemented; 1069 tests; lacks MBAFF, SVC/3D/MVC body | 🚧 ~83% — I+P (1MV/4MV, ¼-pel) + B + CABAC at all chroma layouts + Trellis-quant RDOQ-lite (1227 tests); ffmpeg PSNR_Y 44.20 dB |
| **H.265 (HEVC)** | 🚧 ~52% — VPS+SPS+PPS bodies + scaling-list + scan + §9.3 CABAC engine + slice header through §7.3.6.3 pred_weight_table + r182 §7.3.6.2 ref_pic_lists_modification() + r190 §7.4.8 inter-RPS-prediction typed builder + r193 §7.3.2.3.1 `PpsExtensionFlags` + r195 §9.3.4.2 binarization scaffold + r200 §9.3.4.2.4 `coded_sub_block_flag` ctxInc (eq 9-35..9-39 with `Min(csbfCtx,1)` luma + `2+Min(csbfCtx,1)` chroma + edge-gate variant) + §9.3.4.2.2 Table 9-49 `split_cu_flag` / `cu_skip_flag` ctxInc via shared `left_above_ctx_inc` shape; 251 tests; lacks `sig_coeff_flag`, `coeff_abs_level_g{1,2}` + residual/IDCT | 🚧 scaffold |
| **H.266 (VVC)** | 🚧 ~70% — 4:2:0 IDR intra + ALF/SAO/CC-ALF + P/B merge+skip + HMVP + MMVD + CIIP + BCW + BDOF + GPM + AMVR + HBD + DMVR + affine + PROF + AMVP + SbTMVP + r181 VPS + r193 §7.3.10.10 `amvr_flag`/`amvr_precision_idx` CABAC reader; 1106 lib tests | 🚧 ~90% — forward CABAC + DCT-II + SAO/ALF/cu_qp_delta + MTT BT+TT RDO + P+B + sub-pel MC + multi-ref DPB + weighted bi-pred + r190 §7.3.11.7 `encode_non_merge_inter_pre_residual` + r195 encoder-side §7.3.10.10 `amvr_enc` + r201 §7.3.10.5 `bcw_idx_enc` encoder mirror (TR `cMax = NoBackwardPredFlag ? 4 : 2`, bin0 ctx-coded against Table 91 + ctxInc=0 per Table 132, tail bypass; `encode_non_merge_inter_pre_residual_with_amvr_and_bcw` composite walker chains steps 1-11 in §7.3.11.7 order); 1125 lib tests; lacks numCpMv>1 affine MVD |
| **VP6** | 🚧 r17 — §13 static tables + §3 RawBitReader + §7.3 BoolCoder + r198 §13.2.1 DC arithmetic + r204 §13.3.1 **AC coefficient arithmetic decoder** (`decode_ac_token` Figure 15 walk with EOB-branch + "implicitly-1" first-decision shortcut gated on `prec == WasZero && encoded_coeffs > 1`; `decode_ac_coefficient` wrapper returning `AcOutcome::{EndOfBlock, ZeroRun, Value}`; `AcBand` (Table 30) + `AcPlane` (Table 28) + `AcPrecContext` (Table 29) with `seed_from_dc(dc: i32)` per §13.3.1 first-AC seeding; reuses r16 `decode_token_value` magnitude/sign kernel; 363 tests; +27); unblocks §13.3.3.1 zero-run-length + §13.3 per-frame `AcProbs` update | 🚧 scaffold |
| **VP8** | ✅ 100% | ✅ 100% |
| **VP9** | 🚧 ~42% — §6.2 walk + §9.2 Bool decoder + §6.3 compressed-header primitives chain complete + §6.4.24 coeff + §8.6 dequant + §8.7 inverse transforms + §8.5.1 intra pred + §6.4.3 decode_partition + §6.4.13 read_is_inter + r183 §6.3.17 update_mv_prob + r194 §6.3.18 `setup_compound_reference_mode` + r199 §6.3.12 **`frame_reference_mode` compressed-header outer driver** (two-`L(1)` walker computes `compoundReferenceAllowed` via for-i-in-1..REFS_PER_FRAME loop against `ref_frame_sign_bias[LAST_FRAME]`, short-circuits all-agree tuples to `SingleReference` with zero bool-coder reads, returns `(ReferenceMode, Option<CompoundReferenceConfig>)`; 402 lib tests); lacks §6.3.16 mv_probs outer driver + §6.2.5 inter-frame branch of uncompressed-header walker + §6.4.4 decode_block + §8.4 loop filter | 🚧 scaffold |
| **AV1** | 🚧 ~94% — decoder feature-complete + **standalone `decode_av1` public entry** + r203 §6.7.2 Y-only (monochrome) on the dyn pixel driver (1504 tests + integration roundtrips) | 🚧 ~28% encoder — pixel-space YUV→IVF driver + 14-mode intra picker + §7.13.3 forward 2D dispatcher + WHT lossless + forward quantize + r194 §7.11.5.3 UV_CFL_PRED + r196 `base_q_idx > 0` lossy quant + r197 rectangular extents (12 cardinal shapes) + r203 **monochrome encoder dyn driver** (`pixel_driver_dyn.rs` Y-only encode path mirroring the decoder; +449 LOC; +453 LOC decoder side; encoder→decoder pixel-exact roundtrip on `mono_color_index_only_set=0` mode). Lacks rectangular **TX_SIZE family** (TX_4X8/8X4/8X16/16X8) + §5.11.18 inter mode_info + RD picker + multi-SB tiling + per-block delta_q |
| **Dirac / VC-2** | ✅ ~95% — VC-2 LD+HQ intra + Dirac core-syntax intra/inter + OBMC + 7 wavelets + 10/12-bit + bit-exact intra fixtures + r165 fuzz oracle + r190 Criterion bench harness + r195 `vh_synth`/`vh_analysis` row-major slice driving + r201 §12.4.4 `extended_transform_parameters` parser (VC-2 v3 streams that reduce to the §12.4.4 NOTE symmetric default decode cleanly; genuinely asymmetric v3 streams surface typed `AsymmetricTransformUnsupported { wavelet_index_ho, dwt_depth_ho }`; `dwt_depth_ho ≤ 6` enforced; 343 tests) | 🚧 ~95% — HQ+LD intra + Dirac core-syntax + adaptive sub-pel + 2-ref bipred + post-OBMC refinement + picture/sequence rate-control + r179 intra-encoder fuzz oracle + r193 inter-encoder fuzz oracle (3-oracle coverage complete) |
| **AMV video** | 🚧 scaffold (orphan rebuild post-audit 2026-05-18; clean-room re-implementation pending) | 🚧 scaffold (orphan rebuild post-audit 2026-05-18; clean-room re-implementation pending) |
| **ProRes** | ✅ ~96% — RDD 36 entropy + 8/10/12-bit + 4:4:4:4 alpha + interlaced + RAW refused; ffmpeg interop 60-68 dB + cargo-fuzz + r185 `idct8x8_dc_only` fast path + r195 §5.1+§7.5.3 SHA-256 lockstep pin on two 1920×1080 interlaced apcn 10-bit fixtures + r201 §5.1+§5.3+§7.2+§7.5.1 SHA-256 lockstep pin on 7 progressive fixtures (proxy apco / lt apcs / sq apcn / hq apch / 4444 ap4h / 4444xq ap4x / 4444+alpha; per-fixture pinned constants; reference fixed-point SHA reported alongside ours so ~1-LSB drift stays visible; 259 tests) | ✅ ~97% — RDD 36 all 6 profiles + interlaced + alpha + perceptual quant matrices + r193 ffmpeg cross-decode acceptance §5.3 MBs/slice knob (58.85-63.93 dB luma PSNR) |
| **EVC** (MPEG-5) | 🚧 ~89% — NAL + SPS/PPS/APS + §9.3 CABAC + §8 intra + DCT-II + P/B inter + RPL + HMVP + DPB + ALF + DRA + IBC §8.6 + r187 §8.9.7 `DraChromaDerived` + r193 §8.9.8 `DraJoinedScaleFlag=1` joined-chroma-scale + r195 §7.4.3.1 SPS-signalled `ChromaQpTable` + r201 §7.4.3.1 identity `ChromaQpTable` for non-4:2:0 `ChromaArrayType` (monochrome / 4:2:2 / 4:4:4 per spec pp.67 "Otherwise" branch) + `chroma_qp_table_for_sps` three-way SPS adapter (signalled → default Table 5/6 → identity); 429 tests; lacks Main-profile toolset (BTT/ADMVP/EIPD/ATS/AMVR/affine) | — |
| **HuffYUV** / FFVHuff | ✅ ~97% — HFYU + FFVH FourCCs + 6 predictors + 8-bit only + interlaced field-stride=2 + fast-LUT decoder + SWAR 8-byte gradient post-pass + r181 YUY2 LEFT macropixel-step branch-free decoder + r196 cargo-fuzz `encode_roundtrip` target + narrow-YUY2 Median forward-encode width fix + r202 YUY2 Median tail-loop dead-branch strip (`pos>=row_bytes+8` invariant makes three intra-loop branches provably dead; straight-line per-byte body; 161 tests) | ✅ ~96% — full encoder symmetry × YUY2/RGB24/RGB32 + v1.x + v2.x ClassicV2/CustomV2 + r181 YUY2 LEFT forward branch-free + r186 `forward_rgb_left_subtract_linear` single-stride RGB24/RGB32 LEFT-residual walk + r202 encoder-side dead-branch parity |
| **Lagarith** | ✅ ~95% — all 11 wire types + modern range coder + legacy adaptive-CDF + Fibonacci-Zeckendorf prefix + JPEG-LS Median + G-pivot decorr + zero-run RLE + pair-packed 513-entry CDF + modern RGB(A) first-column predictor Rule B + r198 deeper channel-body fuzz (bit-XOR + multi-byte burst + shift sweeps layered on r192's truncation + single-byte-flip; closes the §6.1 channel-body parser surface) | 🚧 ~76% — encoder for SOLID/RGB/RGBA/YV12/YUY2/legacy-RGB + spec/02 §5 Step-A + Step-B + Step-C `freqs[]` cache + r135/r138/r141 modern + per-channel header-form selection; byte-exact vs proprietary encoder Auditor-blocked |
| **Ut Video** | ✅ ~97% — 5 native FourCCs × 4 predictors + RGB inter-plane decorrelation + LUT-accelerated canonical Huffman + slice-parallel decode (5.63× at 720p) + criterion baseline + r186 `Decoder` trait factory + r196 Gradient/Median per-row branch-hoist + r203 **row-strided None + Left predictor refactor** (single shared stride-aware row driver replaces two near-duplicate per-predictor inner loops; tests/round16_predictor_row_stride.rs covers contiguous + odd-stride + tail-partial-row equivalence vs r186 baseline; +468 test LOC; observable byte-identical) | ✅ ~96% — slice-parallel encode (3.28×) + content-fixture corpus + r161 cargo-fuzz oracle |
| **MagicYUV** | ✅ 100% — 17 v7 FOURCCs + Median + JPEG-LS Median (HBD) + raw-mode + interlaced + r130 `decode_into(&mut DecodedFrame)` streaming entry point + r186 `HuffmanTable::build` opt-9 (HashMap→direct-indexed `Vec<i32>` for two-level path; `core::mem::take` for `start`; observable table byte-identical); trace JSONL strict-jq-line-diff-equal to cleanroom Python ref | ✅ 100% — `encode_frame` across all 17 FOURCCs + spec/04 §3 Dynamic + spec/05 §6.2 Auto Huffman/raw + length-limited Package-Merge Huffman + r127 decoder packed `Vec<u32>` + r136 daily cargo-fuzz (~980k exec/60 s, 0 crashes) |
| **Cinepak** (CVID) | ✅ ~98% — frame header + multi-strip + V1/V4 codebooks + intra/inter + grayscale + Sega FILM demuxer + Saturn/3DO deviant + r181 codebook_chunk_apply + r192 `decode_vector_chunk` cargo-fuzz target + criterion benches + r196 `decode_multi_frame` cargo-fuzz target + r202 named seed-corpora for `codebook_chunk_apply` / `decode_vector_chunk` / `decode_deviant_frame` (27 deterministic seeds via `examples/seed_fuzz_corpora.rs` + in-memory verification test through public entry points) | ✅ ~98% — stateful encoder + rolling codebooks + RDO + LBG + 3-axis grid picker + bitrate-target rate-control + keyframe-interval (34.18 dB PSNR; decode 4.4 GiB/s, stateful GOP 13.5 ms/frame) |
| **SVQ1/SVQ3** (Sorenson) | 🚧 r11 — SVQ1 framework + r194 L=0..L=3 codebook payload + r197 L=4/L=5 ABSENCE + r203 **SVQ1 saturating-clip + bit-mask helper LUTs** (`build.rs` extension stages `clip_lut.csv` 769-row table + `MANIFEST-02.sha256` integrity; `svq1_helper_luts.rs` typed-LUT accessors for `saturating_clip` + `mask_bits`; +237 LOC LUT module + +175 LOC build extension; `tables/clip_lut.meta` binary-disassembly-tier provenance YAML only); lacks intra-vs-inter ordering + stage interleave + SVQ3 MV-VLC + #1256 svq3.c attribution scrub | — |
| **Indeo 3** (IV31/IV32) | 🚧 r14 — clean-room codec-frame header + bitstream header + spec/02 picture-layer + spec/03 macroblock-layer + spec/04 VQ codebook + spec/06 byte-level entropy + spec/07 output-reconstruction + four cell-shape kernels + spec/02 strip-context array + spec/03 per-cell sub-array wiring + r181 spec/05 §1 mc_table + r186 spec/05 §2.2/§2.3/§3.3/§3.4 packed-MV bit-layout + r196 spec/05 §5.4 cell-position decoding + r202 spec/05 §4.2 ping-pong bank-selection (`Bank::{Primary,Secondary}` from `frame_flags` bit 9; `BANK_INVERSION_DELTA=3`; `McBankAssignment::resolve(flags, plane_idx)` returns typed `(dst_slot, src_slot, dst_bank)` triple with `is_self_copy()`/`slot_delta()`; closes round-15 `McCellAddressPair::resolve` deferred bank-pick; 320 tests); lacks §7.2 boundary fix-up + §7.3 reverse decomposition + pixel-buffer edge fix-up + MC inner loop | — |
| **Indeo 2/4/5** | 🚧 scaffold — pending clean-room workspace; Indeo 4/5 still sandboxed via `oxideav-vfw` | — |

</details>

<details>
<summary><strong>Image</strong> (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **PNG / APNG** | ✅ 100% — 5 colour types × 8/16-bit + APNG + sBIT/pHYs/tIME/bKGD/hIST/eXIf/sRGB/cICP/sPLT + r154 Criterion benches + r183 tRNS keyed transparency promotion for ct=0/2 + structural rejection of prohibited tRNS on ct=4/ct=6 + r196 APNG frame-scan Criterion bench (per-frame iterator throughput across acTL/fcTL/fdAT chunk walks at 256² / 720p / 1080p) | ✅ 100% |
| **GIF** | ✅ 100% — 87a/89a + LZW + interlaced + animation + disposal compositor + structured Application Extensions + Plain Text Extension + lenient mode + lazy Playback + animation-timing accessors + fluent AnimationBuilder; clean-room from CompuServe spec + r153 tracked spec-derived fuzz seed corpus (5 seeds × 3 targets) | ✅ 100% — per-frame palettes + `optimize_color_tables()` GCT/LCT hoisting + §7 Required Version enforcement + `upgrade_version_if_needed()` |
| **WebP** (VP8 + VP8L) | ✅ 100% | ✅ 100% |
| **JPEG** (still) | ✅ ~95% — via MJPEG | ✅ ~90% — via MJPEG |
| **TIFF** (6.0) | ✅ ~98% — II/MM + BigTIFF read + 7 photometrics (incl. PI=4 Transparency Mask r172) + 1/4/8/16-bit + None/PackBits/LZW/Deflate/CCITT-MH/T.4-1D + FillOrder + tiles + multi-page + JPEG-in-TIFF (incl. CMYK-JPEG: Compression=7 + Photometric=5 + SamplesPerPixel=4) + PlanarConfiguration=2 (separate component planes across strips/tiles + chunky re-interleave + Predictor=2 driven per-plane) + cargo-fuzz decoder (panic-free, 7.7 M iter green) | ✅ Gray8/16/RGB24/Palette8 — None/PackBits/LZW/Deflate + Predictor=2 + PlanarConfiguration=2 separate-planes write (Rgb24 × None/PackBits/LZW/Deflate ± Predictor=2) + Bilevel CCITT-MH / T.4-1D, single+multi-page + tiled chunky write (Gray8/16/RGB24/Palette8 × None/PackBits/LZW/Deflate ± Predictor=2, §15) + tiled PlanarConfiguration=2 write (Rgb24, one grid per plane, §15) |
| **BMP** | ✅ ~96% — 1/4/8/16/24/32-bit + V4/V5 + OS/2 BITMAPCOREHEADER + RLE4/RLE8 + top-down + daily fuzz CI (3 targets: decode + r162 `rle_stream` RLE-state-machine focus + r198 `encode_roundtrip` direct-colour byte-level — 1.33 M execs/60 s / 0 crashes / 0 roundtrip mismatches) + 31-test property-test sweep | ✅ ~96% — top-down + minimal `biClrUsed`-trimmed palette encoder |
| **Netpbm** (PBM/PGM/PPM/PNM/PAM) | ✅ ~95% — all 8 magics at 1/8/16-bit + 6 PAM TUPLTYPEs + r171 cargo-fuzz harness + decoder pre-allocation OOM hardening | ✅ ~95% |
| **ICO / CUR / ANI** | ✅ ~98% — multi-res + BMP/PNG sub-images + CUR hotspot + ICONDIRENTRY validation + 256×256 PNG round-trip + r198 standalone `read_ani_raw` RIFF/ACON parser + r198 `biBitCount` reject + r204 **ANI `seq[]` step-index bounds-check** (`read_ani_raw` rejects upfront when any `seq[i] >= nFrames` rather than emit a sequence array unsafe to dereference; closes adversarial `seq[k] = 0xFFFFFFFF` OOB-read shape; same probe-vs-render hardening as r188 CUR-hotspot + r198 biBitCount; 73 lib tests) | ✅ ~92% |
| **JPEG 2000** | 🚧 r15 (post-2026-05-20 orphan) — T.800 main-header + SOT/SOD + typed COC/QCC/POC/RGN/PLT/PPT + JP2 box + §B.10 tier-2 + §B.5 ResolutionLevel + §B.6 precinct + §B.7 code-block partition + Annex C §C.3 tier-1 MQ + Annex D 19 contexts + §B.12.1 5 packet-progression iterators + §B.12.2 POC + r181 Annex F.3 inverse DWT + r187 4 cargo-fuzz targets + r192 Annex E code-block→sub-band reassembly + r195 Annex G MCT primitives + r201 §G.1 DC level-shift completed (forward Equation G-1 symmetric to inverse + `{forward,inverse}_dc_level_shift_unsigned_i64` lifts Ssiz cap from ≤31 to Table A.11's full 1..=38 range + signed-aware dispatchers (no-op for signed components per §G.1.1/§G.1.2 prologue) + §G.1.2 NOTE "clip to dynamic range" helper; mct module 12→29 tests; 381 tests); lacks per-resolution cascade + HTJ2K Part-15 | 🚧 scaffold |
| **JPEG XL** | 🚧 ~92% — ISO/IEC 18181-1:2024 lossless Modular path + 7 fixtures pixel-correct + VarDCT scaffold + Gaborish/EPF/AFV pure-math complete + §C.8.3 per-block HF coefficient loop + r190 `PerPassNonZerosGrids` per-pass container + r191 WP trace oracle isolating #799 divergence + r195 WP state-evolution backward bisect + r202 row-3 chain widening (`r202_wp_row3_chain` 7 tests across samples 192..=200; new finding: `Δ pred8 = -50` at sample 192 vs +8 at 194 — divergence is 2 samples earlier than previously pinned; asymmetric row-2 defect at sample 129 vs 130 vs 131; production `v(195)=88` vs spec `10` cascades into wrong MA-tree leaf-pick) + Hat-2 scrub of 7 pre-existing decorative libjxl lines; 641 tests; lacks upstream WP fix + §C.7.2 histograms | — retired |
| **JPEG XS** | 🚧 ~82% — ISO/IEC 21122 Part-1 + 5/3 DWT + Annex C/D/F/G + multi-component + CAP-bit + high bit depth + r190 4:2:0 chroma at NL,y≥3 | 🚧 ~88% — Nc 1/3/4 + Sd>0 + RCT + Star-Tetrix + NL up to 8 + odd dims + vertical prediction + per-band Q + NLT + r193 Annex G.5 NLT extended at bd∈9..=16 + r195 high-bit-depth Star-Tetrix + r201 high-bit-depth Star-Tetrix **lossy** (`encode_planar_star_tetrix_highbd_lossy`, `Cpih=3` + `q>0` + `B[i]∈9..=16` per-band deadzone truncation; encoder `B[i]>8` surface now complete lossless+lossy across all colour-transform/NLT modes; 328 tests) |
| **AVIF** | 🚧 ~88% — HEIF→AV1 + grid + imir/clap/colr/pixi/pasp + HDR metadata + AV1 wrap + DoS caps + HEIF item-properties + auxC URN + rloc/lsel/iovl/grpl + `mif1` + r130 tmap §4.2.2 + r188 ISO 21496-1 Annex C.2 `GainMapMetadata` + r193 §5.2.5.3+§5.2.7 value-comparison shalls + r195 §8.2/§8.3 AVIF profile-compliance audit + r201 av1-avif v1.2.0 §3 AV1 Image Sequence `shall`-level audit (`audit_avis_sequence`: `mdia/hdlr=='pict'` + single `'av01'` SampleEntry + cross-sample Sequence Header OBU byte-identical; `AvisMeta.{handler, sample_description_types}`; pinned on Netflix `alpha_video.avif`); 270 + 58 tests | — |
| **DDS** | ✅ ~99% — DDS_HEADER + DXT10 + uncompressed (10 layouts) + BC1-7 + BC6H all 14 modes + mipmap + 6-face cubemaps + DX10 arrays + volume textures + 132-entry DXGI table + daily cargo-fuzz + r162 40-case injection-robustness + r176 saturating-math + r192 Criterion benches (decode BC1-5 @ 512² + BC6H/BC7 @ 256², encode BC1-5 @ 256² + BC6H/BC7 mode-pickers @ 128², roundtrip A8R8G8B8 single+9-mip + DXT10 R8G8B8A8_UNORM + L8 separating container vs per-block hot path; xorshift-seeded synthetic inputs no binary fixtures) | ✅ ~95% — uncompressed + BC1-5 + BC7 all 8 modes + BC6H_UF16/SF16 all 14 modes + box-downsample mip chains + cubemap/array |
| **OpenEXR** | 🚧 ~87% — magic + 8 required attrs + HALF/FLOAT/UINT + NO_COMPRESSION/ZIP/ZIPS/RLE + tiled ONE_LEVEL + sub-sampled chroma + single-part deep scanline + multi-part deep scanline + r130 single-part deep tiled + r181 multi-part deep TILED + r192 multi-part flat TILED ONE_LEVEL read + r196 multi-part flat MIPMAP_LEVELS read + r202 multi-part flat **RIPMAP_LEVELS** read (2-D lvly-outer/lvlx-inner walk alongside ONE_LEVEL+MIPMAP_LEVELS; closes last open multi-part flat-tiled level-mode; 213 tests); PIZ blocked on docs trace | ✅ ~94% — RGBA scanline + r130 single-part deep tiled + r181 multi-part deep TILED + r196 multi-part flat MIPMAP_LEVELS write + r202 multi-part flat RIPMAP_LEVELS write (`MultipartRipmapTiledPart` 2-D grid, version-field bit 0x1000 only + per-part `tiles[tiledesc, level_mode=2]`; exrheader + exrmultipart -separate validated pixel-exact) |
| **Farbfeld** | ✅ 100% — streaming reader + DoS hardening (dimension overflow + truncated payload guards) + `magick` black-box cross-validator | ✅ 100% |
| **HDR** (Radiance RGBE) | ✅ ~99% — new-RLE + old-RLE + 8 axis-flag combos + shared-exponent + multi-record EXPOSURE/COLORCORR + typed COLORCORR/PRIMARIES/VIEW + apply_exposure/apply_colorcorr + r189 luminance_lm_per_sr_per_m2 + r192 committed-fixture regression anchors + r196 uncompressed scanline R+W + r202 `HdrLimits` resource-cap surface (`parse_hdr_with_limits` + `parse_hdr_with_options_and_limits` + `HdrError::TooLarge`; default 32767×32767 + 256 MiB pixel-bytes; checked_mul width×height×12 BEFORE alloc) + cargo-fuzz harness (decode/roundtrip/headers); 81 lib tests | ✅ ~98% — new-RLE + old-RLE + auto-RLE + XYZE↔RGB + 8 tonemap ops + CRLF + r179 zero-copy `reorient_for_axis_flags` (~6% encode throughput at 1024²) |
| **QOI** | ✅ 100% — byte-exact vs all 8 reference fixtures + criterion decode bench (540 MiB/s gradient, 1.55 GiB/s solid-RUN) + r162 second cargo-fuzz target encode_roundtrip (5 seeds, 33k local iters clean) | ✅ 100% — byte-exact vs reference encoder + criterion encode bench (640 MiB/s gradient, 2.13 GiB/s solid-RUN) |
| **TGA** | ✅ 100% — types 1/2/3/9/10/11 + TGA 2.0 extension + thumbnail + developer area + CCT + scan-line table + typed AttributesType alpha + r188 image-descriptor bit-4 right-to-left column ordering + r201 §3.3/§C.3 image-identification field round-trip (`parse_tga_image_id` borrowed slice + `splice_image_id` post-encode splice, `TGA_IMAGE_ID_MAX=255`; composes w/ extension-area entry point; 153 tests); magick cross-validated + r154 cargo-fuzz daily decode harness | ✅ 100% — all six image types + full TGA 2.0 extension + thumbnail + RGB24-input entry points |
| **ICER** (JPL) | 🚧 ~78% — Mars-rover heritage; bit-plane scan + compressed/uncompressed segments + 8 filters + IPN 42-155 §III.B context model + r192 §III.E lenient multi-segment decode (`parse_icer_lenient` / `parse_icer_lenient_with_limits` for DSN-packet-loss spaceflight scenario — `LenientDecode { image, received, missing_count }`; segment 0 required to pin canonical strip dims; missing strips reconstruct as flat 128 matching r6 ROI placeholder; trailing-drop truncates; +9 integration tests) | ✅ ~82% — quota encoding + auto wavelet selection + R-D byte-budget + r189 per-segment §III.D uncompressed fallback |
| **WBMP** | ✅ 100% — Type 0 + WbmpLimits DoS caps + adversarial fuzz sweep + r189 caller-selectable `MonoBlack`/`MonoWhite` decode polarity (`parse_wbmp_as` + `CodecParameters::pixel_format` routing) | ✅ 100% |
| **PCX** (ZSoft) | ✅ ~97% — 1/2/4/8 bpp planar + packed-bits + 24 bpp RGB planar + grayscale flag + DCX multi-page + DCX `Demuxer` + r136 fuzz-hardened + r197 Criterion bench harness (decode/encode/roundtrip across 9 scenarios: 1bpp×4 EGA / 2bpp CGA / 4bpp packed / 8bpp palette / 8bpp grayscale / 24-bit RGB + DCX multi-page; xorshift32 deterministic fills, no committed fixtures) | ✅ ~92% — 8 write paths + DCX; r185 framework `Encoder` widened to Rgba/Rgb24/Gray8 + Bgr24/Bgra/MonoBlack/MonoWhite |
| **ILBM** (Amiga IFF) | ✅ ~94% — BMHD/CMAP/CAMG/BODY + ByteRun1 RLE + EHB + HAM6/HAM8 + PBM + SHAM + PCHG + ANIM op-0/op-5 + CRNG/CCRT + DRNG (DPaint IV extended range, true-colour + register cells); lacks ANIM op-7/op-8, DEEP true-colour | ✅ ~84% — IlbmMuxer parity + masking + ANIM op-5 + CRNG/CCRT/DRNG encoder |
| **PICT** (Apple QuickDraw) | ✅ ~99% — v1 + v2 opcode walkers + drawing rasteriser + DirectBitsRect packType 0/1/2/3/4 + Region + clip-region + pen-size aware + Compressed/UncompressedQuickTime opcode skip + monochrome stipple + PixPat colour 8×8 type 1/2 + r186 indexed PixMap variant of `BitsRect/BitsRgn/PackBitsRect/PackBitsRgn` + r199 §A-3 **reserved-for-Apple-use v2 opcode skip table** (`opcodes::reserved_v2_payload_size(opcode)` classifies as `Fixed(n)`/`U16Prefixed`/`U32Prefixed`/`PolygonSized`/`RegionSized`; decoder + probe walk past via documented shape instead of fatal-erroring; +27 tests = 100 root + 12 integration); lacks text rasterisation + embedded `CompressedQuickTime 0x8200` JPEG decode | ✅ ~93% — `PictBuilder` + every v2 drawing-command family + state opcodes + mono+PixPat pattern setters + DirectBitsRect packType 1/2/3/4 + BitsRgn / PackBitsRgn; magick cross-decode bit-exact |
| **SVG** | ✅ ~99% — full shape set + path + gradients + text + mask + clipPath + use/symbol + svgz + SMIL animate/set/animateTransform + CSS3 Selectors L3 + `@import` + `@font-face` + `@keyframes` + Media Queries L4 + viewBox + 17 filter primitives + CSS Values L4 LengthUnit + CSS Easing L2 + SVG 2 §9.6.1 pathLength + SVG 2 §16.3 `<view>` element + fragment-identifier routing (`#MyView` / `#svgView(...)` + percent-decode + spatial/temporal media-fragment fallthrough) + SVG 2 §5.7 `<switch>` conditional processing (requiredExtensions / systemLanguage) + SVG 2 §13.7.1 `<marker>` typed def capture (refX/refY geometric keywords + markerUnits/orient + verbatim round-trip) + SVG 2 §13.2 `context-fill`/`context-stroke` + SVG 2 §16.5 `<a>` hyperlink (renders as group; link target + HTML attrs preserved across round-trip) + SVG 1.1 §11.5 `display` / `visibility` property handling + SVG 2 §5.8 `<title>` / `<desc>` + §5.9 `<metadata>` capture (multilingual lang, round-trip via PreservedExtras) + r172 SVG 2 §11.10.1.1 text-anchor (start/middle/end, inherited) + §11.8.3 textPath start-offset bias | ✅ ~88% — round-trips full shape graph + PreservedExtras side-channel + `<view>` re-emit at trailing edge |
| **PDF** | ✅ ~99% — bytes → Scene via xref/xref-streams/ObjStm + `/Prev` incremental + `/Encrypt` R=2..6 + public-key + PKCS#7 + `/Sig` AcroForm + Doc-Timestamp + text extraction + Linearization + Tagged-PDF + EmbeddedFiles + §12.6 actions + 5 stream filters + §8.11 Optional Content + r194 PDF 2.0 §14.13 Associated Files + r197 6 new §12.5.6 annotation subtypes (Line/Polygon/PolyLine/Ink/Caret/Popup/FileAttachment) + r204 **§12.5.6.22 `/Watermark` (Table 190 + Table 191 `FixedPrint` six-number `/Matrix` + `/H`/`/V` percentages)** + **§12.5.6.23 `/Redact` non-destructive surface** (Table 192 `/QuadPoints` Option-typed + 3-component DeviceRGB `/IC` validated + `/RO` Form XObject as `ObjectId` + `/OverlayText` UTF-16BE-BOM + `/Repeat` + `/DA` + `/Q` clamped 0..=2); 497 + 15 tests; Movie/Sound/Screen/3D/RichMedia remain `Other` | ✅ ~99% — PDF 1.4/1.5 multi-page + paths/gradients/opacity/clip + RGBA + xref-stream + ObjStm + Linearization writer + `/Encrypt` + public-key + `/Sig` + AcroForm + annotation writer + embedded files + RFC 3161 Document Time-Stamp writer |

</details>

<details>
<summary><strong>3D scenes & assets</strong> (click to expand)</summary>

> The typed Scene3D / Mesh / Material PBR / Skin / Animation / Camera / Light / AudioEmitter model lives in `oxideav-mesh3d`, with `Mesh3DDecoder` / `Mesh3DEncoder` traits and a `Mesh3DRegistry` that's parallel to `oxideav-core::CodecRegistry`. Per-format crates register into it. `oxideav-meta::populate_mesh3d_registry(&mut Mesh3DRegistry)` walks every enabled format's `register()`. Lazy bytes flow through `AssetSource` (with a `raw_storage` pass-through hook for archive-backed sources, e.g. ZIP-stored USDZ textures + audio).

| Format | Decode | Encode |
|--------|--------|--------|
| **STL** (ASCII + binary) | ✅ ~99% — ASCII + binary + per-face attrs + 16-bit colour + multi-`solid` + topology + 8-step repair pipeline + r205 `repair_make_winding_consistent` (BFS over manifold-edge adjacency; flips any neighbour whose winding agrees with the seed on a shared edge; matches `validate`'s `inconsistent_winding_edges` rule, closing the diagnostic↔repair matrix for the spec's mesh-wide winding invariant) + daily cargo-fuzz + r161 Criterion | ✅ ~99% — both formats + attribute pass-through + `EncodeStats` + configurable float precision |
| **OBJ** (+ MTL) | ✅ ~98% — full Wavefront grammar + MTL (Phong + Wavefront-PBR + map_* options + typed refl) + smoothing/display attrs + free-form geometry pass-through + `xyzrgb` per-vertex colour + Bezier/B-spline/NURBS/Cardinal/Taylor `curv` + `surf` 2D-surface tessellation + r171 cargo-fuzz harness + r188 `curv2` 2D trimming-curve tessellation (trim/hole/scrv parameter-space curves → LineStrip via existing evaluators; rational weights + parm-u windows + negative indices); lacks surface-clip-against-trim-loops, multi-patch decomposition | ✅ ~96% — symmetric + negative-index encoder + polyline rejoin |
| **glTF 2.0** (+ .glb) | ✅ ~97% — JSON + .glb + full PBR + 12 KHR_materials extensions + skin + skeletal animation + sparse accessors + morph-targets + 12 spec-MUST validators + KHR_texture_transform + r188 KHR_mesh_quantization decode + r199 **KHR_node_visibility** (Khronos ratified per-node Boolean visible flag; spec default `true`; false hides node subtree; decoder lifts into `Node::extras["KHR_node_visibility"]=Bool`; encoder rebuilds typed extension + declares in `extensionsUsed`; §3.12 validator rejects undeclared use; +10 tests); lacks KHR_audio_emitter + quantized morph-targets + KHR_materials_variants | ✅ ~92% — symmetric + sparse-encoding heuristic + signed+unsigned normalised-int quantisation + KHR_node_visibility round-trip + KHR_materials_unlit emit |
| **USDZ** (+ USDA) | ✅ ~93% — ZIP STORED walker + USDA parser + UsdGeomMesh + UsdPreviewSurface PBR + UsdUVTexture pass-through + xformOp transforms + UsdMediaSpatialAudio + variantSet + LIVRPS variant-selection composition + composition-arc round-trip + in-archive sublayer + references/payload arc composition + r180 in-layer `inherits`/`specializes` class-arc composition + r188 reader-side CRC-32/ISO-HDLC verify on `walk()` + r200 `.usdc` (Pixar Crate binary) bootstrap parser — `PXR-USDC` magic + 88-byte fixed header `Bootstrap { version, toc_offset }` + `Toc::parse` (section-region overlap/EOF/bootstrap-collision validation, defensive 4096 section cap) + six standard `SectionName` enum (TOKENS/STRINGS/FIELDS/FIELDSETS/PATHS/SPECS); USDZ Default Layer now validates `.usdc` bootstrap+TOC before `Error::Unsupported`; real-fixture trace cross-check (v0.8.0 elephant scene); lacks `.usdc` payload decode (LZ4+2-bit-control delta integers + value-rep), UsdSkel*, UsdGeomSubset | ✅ ~88% — symmetric writer + zero-re-encode pass-through + variant writer + composition-arc writer |
| **FBX** | 🚧 ~88% — binary + ASCII container (r200 ASCII reader: tokenizer + recursive parser produces the same `FbxDocument` tree as binary; `FbxDecoder::decode` dispatches binary-vs-ASCII via leading-bytes sniff; covers comments, value-then-body openings, typed-array shorthand `*N {a:...}`, bare-letter T/F booleans, UTF-8/Cyrillic; end-to-end `cubes-ascii-v7500.fbx` fixture → 4 meshes through `build_scene`) + object-graph + mesh + animation + deformers + Material/Texture/Video + bind pose + LayerElementMaterial/Color + Properties70 P-record grammar + multi-UV-set surfacing; 87 unit + 93 integration tests. Lacks: Light/Camera NodeAttribute, multi-LayerElementNormal, ASCII writer | ✅ ~58% — symmetric binary writer + opt-in zlib deflate |
| **Alembic** | 🚧 0% — Sphinx API reference + Python examples staged at `docs/3d/alembic/`; on-disk Ogawa binary needs Wayback PDF recovery (Imageworks 2010-2012 manuals 404 today) or commissioned trace | — |

Cross-format integration: `oxideav-cli-convert` exposes a 3D conversion path through `oxideav_meta::populate_mesh3d_registry` — `oxideav convert in.obj out.gltf` (or `--probe` for structural inspection). `crates/oxideav-tests/tests/mesh3d_*.rs` runs the cross-format roundtrip suite. Convert verb has accumulated IM-compatible ops including `-resize` / `-thumbnail` / `-define` / r178 `-extent WxH±X±Y` (canvas re-window w/ source-order `-background` colour) / r184 `-monochrome` (gray + 2 colors + Floyd-Steinberg shorthand), USDZ encoder + 3D→raster renderer (Gouraud + Phong + `-light` / `-camera` / `-projection` / `-fov` / `-bg`), `-render normal-debug|depth-debug` + `-aa N` supersampling, and multi-size ICO via `-define icon:auto-resize`. Black-box oracles in `tests/mesh3d_{usdz_apple,blender_assimp}_oracle.rs` cross-validate against Apple `usdzconvert` + Blender + assimp.

</details>

<details>
<summary><strong>Trackers</strong> (decode-only by design) (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **MOD / STM / XM** | ✅ ~97% MOD — 4-channel Paula mixer + full ProTracker 1.1B effect set + FT-extension `8xx`/`E8x` pan + XM E3x glissando + Lxy set-envelope-position + E4x/E7x vibrato/tremolo waveform shapes + r171 cargo-fuzz; ✅ ~90% STM — r197 codec id `stm` promoted from stub to real `StmDecoder`; ✅ XM r203 codec id `xm` **promoted from stub to full playback decoder** (mirrors ModDecoder/StmDecoder shape: structural parse → patterns → envelopes/fadeout → `XmPlayerState::render` into interleaved S16 stereo; +214 LOC `decoder.rs` + +130 LOC `xm_player.rs`; +46 LOC `tests/xm_smoke.rs`) | — |
| **STM** (Scream Tracker v1) | ✅ ~85% — structural parse + shared-mixer playback; XM-parity effects (Gxy/Jxy/Bxy/Cxy/Exy/Hxy + 7xy tremolo + volume-slide variants); hard-pan LRRL | — |
| **XM** (FastTracker 2) | ✅ ~90% — structural parse + full playback; envelopes + fadeout + key-off; vibrato + tone porta + pattern jumps + fine/extra-fine porta + Exy/Kxy subcommands + volume-column slides | — |
| **S3M** | ✅ ~96% — stereo + full ST3 v3.20 effect set + per-channel effect memory + Dxy case matrix + S3x/S4x bit-2 retention + Qxy persistent-counter retrigger + Cxx row-≥64 ignore + Kxy/Lxy continue + r171 +128 channel-mute + r183 spec-correct default-pan + r197 header-driven playback corrections + r203 **Vxx effect: spec-correct value range + tick-1 timing + speed-1 skip** (ST3 Vxx global-volume `0x00..=0x40` clamping; tick-1 trigger semantics for V00 vs V-non-0; `speed = 1` row-end short-circuit to avoid double-volume application; 79+ tests); lacks AdLib FM synth | — |

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
| **`oxideav-videotoolbox`** | macOS (Apple Silicon + Intel Macs) | 🚧 H.264 + HEVC + ProRes + MJPEG + MPEG-2 + VP9 + MPEG-4 Pt 2 + AV1 (M3+) | 🚧 H.264 + HEVC + ProRes + MJPEG | r198 encoder knobs wired across H.264 / HEVC / MJPEG / ProRes: `bit_rate` → `AverageBitRate`, `options["quality"]` (`Float32 [0,1]`) → `Quality`, `options["profile"]` aliases (H.264 baseline/main/high/extended; HEVC main/main10/main4_2_2_10) → `ProfileLevel`; `make_prores_encoder` dispatches via `prores_codec_type_for_tag()` across all 6 fourCCs (apco/apcs/apcn/apch/ap4h/ap4x). PSNR_Y: MPEG-2 ~61 dB; H.264 ~51 dB; HEVC ~54 dB; ProRes ~52 dB; MJPEG ~36 dB; AV1 ≥30 dB vs libaom-av1 (M3+/macOS 14+). r178 VP9 + r184 MPEG-4 Pt 2 + r190 VOL→ESDS. |
| **`oxideav-audiotoolbox`** | macOS | 🚧 AAC LC + HE-AAC v1/v2 + AAC-LD/ELD + ALAC + iLBC + AMR-NB + AMR-WB + **MP3** (MPEG-1/2/2.5 Layer III) | 🚧 AAC LC + HE-AAC v1/v2 + AAC-LD/ELD + ALAC + iLBC | r178 AAC encoder bitrate read-back; r184 iLBC; r190 AMR-NB; r193 AMR-WB decode + r199 **MP3 decode via `kAudioFormatMPEGLayer3`** (32-bit MPEG-audio frame-header parser + per-(version × layer) bitrate/sample-rate/samples-per-frame/frame-length tables per ISO/IEC 11172-3 §2.4.2.3 + 13818-3 §2.4.2.3 + MPEG-2.5 rates; `Mp3AtDecoder` lazy first-frame configure + persistent input-queue + one-packet slack lookahead; tags `.mp3`/OTI 0x6B/A_MPEG/L3/wFormatTag 0x0055; bit-exact 33×1152 PCM @ ≈89.8 dB SNR vs staged fixture; 138 tests). Roadmap: FLAC, Opus. |
| **`oxideav-vaapi`** | Linux (Intel iGPU + AMD Radeon, via libva) | — stub | — stub | Crate exists; impl is a single-line `// stub`. Planned decode ladder: H.264 + HEVC + VP9 + AV1 (Mesa Radeon, Intel Media Driver). |
| **`oxideav-vdpau`** | Linux (NVIDIA legacy / Nouveau) | — stub | — stub | Stub crate. VDPAU is the older NVIDIA accel API — still useful on systems without proprietary CUDA stack. |
| **`oxideav-nvidia`** | Cross-platform (NVENC + NVDEC via libnvcuvid + libnvidia-encode) | — stub | — stub | Stub crate. Will register as `*_nvenc` / `*_nvdec`. |
| **`oxideav-vulkan-video`** | Cross-platform (Vulkan VK_KHR_video_*) | — empty | — empty | No code yet. Cross-vendor decode ladder per `VK_KHR_video_decode_h264` / `_h265` / `_av1` extensions; encode side per `VK_KHR_video_encode_*`. |

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
| **`oxideav-source`** | URI resolution + file reader + prefetching BufferedSource | ✅ `file://` + `mem://` + `data:` (RFC 2397 inline base64/percent) + `concat:` (`|`-separated segments; r184 widened to mem://`/`data:`/`slice:` inner schemes alongside `file://`) + r178 `slice:<offset>+<length>!<inner-uri>` byte-window driver (composes recursively over file/mem/data) + `FileScope` allow-list policy; generic `SourceRegistry` for pluggable schemes |
| **`oxideav-http`** | HTTP / HTTPS source driver | ✅ `http://` + `https://` via pure-Rust `ureq` + `rustls` + `webpki-roots`; Range-request seeking; `HttpConfig` policy + r171 RFC 7233 §4.2 Content-Range validation + §3.1 200-fallback prefix-drop + r179 §15.5.17 + §14.4 416 handling + r186 RFC 9110 §13.1.5 If-Range strong-validator + r197 §8.6 Content-Length cross-checks + r203 RFC 9110 §13.1.5 **HTTP-date now accepts all three forms** (IMF-fixdate already supported; rfc850-date `weekday, DD-Mon-YY HH:MM:SS GMT` + asctime-date `Wkd Mon DD HH:MM:SS YYYY` parse + canonicalize through the strong-validator pipeline; +2 fuzz seeds `rfc850_date`/`asctime_date`; +468 LOC in src/lib.rs) + `parse_headers` cargo-fuzz harness (10 seeds) |
| **`oxideav-generator`** | Synthetic media source (`generate://...` URIs) + zero-input filters | ✅ audio synth (sine + chirp/FM/DTMF/multitone/ADSR/ringmod + r180 5-colour noise + r198 **`pwm`/`pulse` PWM oscillator** with optional LFO duty sweep, resolution-aware edge clamp + duty-bound clamp) + image (xc/gradient/pattern/fractal/plasma/noise/label; r188 Perlin-2001 2-D simplex) + video (testsrc/smptebars/fractal_zoom/gradient_animate/zoneplate); 155 lib + 26 integ tests |
| **`oxideav-rtmp`** | RTMP ingest + push | ✅ Server + client; AMF0/AMF3 parser/builder; Enhanced-RTMP v1 video + v2 audio + ModEx; pluggable key-verification; `rtmp://` PacketSource; symmetric teardown + client `poll_event` + r179 v2 `MultichannelConfig` audio body (24 SMPTE ST 2036-2-2008 22.2 channel positions) + r187 Enhanced-RTMP v2 Multitrack body parser+builder + r198 §E **FLV file/byte-stream writer** (`FlvWriter<W: Write>` Annex-E 9-byte header + §E.3 PreviousTagSize back-pointers + §E.4 FLVTAG + §E.4.4 script-data; frames every legacy AVC/AAC + Enhanced-RTMP v1/v2 FourCC/ExHeader/Multitrack/ModEx shape; HTTP-FLV bridge now a one-liner; 222 tests = +20) |
| **`oxideav-sysaudio`** | Native audio output | ✅ Runtime-loaded backends (ALSA, PulseAudio, WASAPI, CoreAudio); no C build-time linkage. CoreAudio + WASAPI backends report **real HAL latency** — CoreAudio sums `kAudioDevicePropertyLatency` + `BufferFrameSize` + `SafetyOffset` + `kAudioStreamPropertyLatency`; WASAPI reads `IAudioClock`-derived presentation latency. Output-device enumeration (names + default flag) across WASAPI / ALSA / CoreAudio. r178 per-device routing API (`StreamRequest::with_device(id)` / `open_on`) — r184 CoreAudio wired via HAL `kAudioDevicePropertyDeviceUID` + `AudioQueueSetProperty(kAudioQueueProperty_CurrentDevice)`, all 4 backends now route per-device. BT-aware; falls back to software estimate if HAL unavailable. |
| **`oxideav-pipeline`** | Pipeline composition (source → transforms → sink) | ✅ JSON transcode-graph executor; pipelined multithreaded runtime + `Executor::with_channel_caps(ChannelCaps { packets, frames })` configurable per-track depth (embedded `{1,1}` → offline `{64,32}`) + `Executor::with_max_queue_bytes(n)` orthogonal byte-ceiling on demux→worker queues + r178 `Progress::elapsed_micros` wall-clock stamp on every emission (realtime ratio + live-source drift diagnostics) + r184 `packets_skipped: u64` on `Progress` + `ExecutorStats` (decoder error-tolerance visibility; staged + serial paths both increment, partial-output produced_any suppresses double-count) |
| **`oxideav-scene`** | Time-based scene / composition model | 🚧 data model for PDF pages / RTMP streaming compositor / NLE timelines + r179 per-frame `Sample` + animation-track composition helpers + r188 `RasterRenderer` (bg solid/gradient + Rect/Polygon + `ObjectKind::Vector` → RGBA via oxideav-raster) + r198 **`ObjectKind::Group` nested composition** (per-child resolution at scene time, parent affine/opacity/clip merge, cycle-break, dead-child exclusion) + r198 SVG 1.1 path-data lowering (M/L/H/V/C/S/Q/T/Z + relative) + r204 **arc (A/a) per F.6.1** (single-digit `fA`/`fS` flag grammar incl. minified `A5,5 0 0010,10`; degrees→radians; F.6.2 out-of-range: neg-radii absoluted / zero radius → `line_to` / coincident endpoints omitted; reuses `oxideav_core::PathCommand::ArcTo` + `oxideav_raster::flatten_arc_to_cubics`; `parse_bbox` extends pen-walk with `max(|rx|,|ry|)` endpoint enclosure; `SvgPathError::InvalidArcFlag`); 201 tests — SVG 1.1 path grammar complete; image/video/text ObjectKind pending |
| **`oxideav-audio-filter`** | Audio effects & conversions (streaming) | ✅ ~47 filters: classic + transient/spatial/restoration family + MidSide / EnvelopeFollower / DeEsser / Wah / OctaveDoubler / AdaptiveNoiseGate + Exciter / MultibandCompressor / StereoImager / Talkbox + TransientDesigner / Ducker / GainNormalizer / FreqShifter + HardClipper + r106 SlewLimiter + r188 LR4 crossover + r198 **`true_peak_detector`** (4× polyphase Kaiser-windowed FIR oversampled inter-sample dBTP observer, pass-through audio with `current_dbtp`/`max_dbtp`/`overs` — recovers ~0 dBTP on fs/4 + π/4-phase full-scale sine vs −3.01 dBFS sample peak; +19 tests = 268 lib) — see crate README for the catalogue |
| **`oxideav-image-filter`** | Single-frame image effects (stateless) | ✅ 129 filter types / 176 factory names — r198 **`Gabor` filter** (oriented Gaussian-modulated cosine kernel per Gabor 1946 / Daugman 1985; `(2·radius+1)²` grid with auto-radius `ceil(3·σ·max(1, 1/γ))` clamped to [1,32]; zero-mean DC-removal; `GaborMode::{Signed, Magnitude}` output; +24 tests) joins the edge family at the oriented-bandpass slot; r186 Dither (Bayer + 7 error-diffusion kernels); r105 Scharr / r101 Prewitt / r24 Roberts / r22 Reinhard-Hable-Drago tone-mapping + Curves + Borgefors distance transform + Cyanotype — see crate README for the catalogue |
| **`oxideav-pixfmt`** | Pixel-format conversion + palette + dither | ✅ YUV↔RGB matrices (BT.601 / BT.709 / BT.2020 / BT.2100), chroma subsampling + r179 packed 4:2:2 (YUYV / UYVY) ↔ planar/RGB/RGBA, palette quantisation, Floyd-Steinberg dither, PQ + HLG + BT.1886 transfer functions + r197 Porter-Duff alpha property sweep + r203 **`Ya8` (luma+alpha) wired into `convert()` dispatch** (Ya8 ↔ Gray8 / Rgb24 / Rgba8 with premultiplied + straight-alpha variants; `src/gray.rs` typed Ya8 surface; +134 LOC convert + +81 LOC gray + +118 LOC `tests/conversions.rs`) + Criterion suite for compositing hot path (alpha bench) |

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
| **SRT** (SubRip)    | ✅ | ✅ | `<b>/<i>/<u>/<s>`, `<font color>` hex + 17 named, `<font face size>` |
| **WebVTT**          | ✅ | ✅ | Header, STYLE ::cue(.class), REGION, inline b/i/u/c/v/lang/ruby/timestamp (full §3.5 round-trip incl. BCP 47 lang chains, ruby implicit `</rt>`, multi-byte UTF-8), cue-settings round-trip (vertical / line+position align / region) + full REGION block (id/width/lines/regionanchor/viewportanchor/scroll) |
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
| **ASS / SSA**       | ✅ | ✅ | Script Info + V4+/V4 Styles (BGR+inv-alpha) + override tags + r172 `\fn`/`\fe`/`\b<weight>`/`\r[<style>]` + r177 `\pbo` + r183 face-flag toggles + r186 typed `\p<scale>` + r198 `\fax`/`\fay` shear baked into per-cue affine + r204 **`\an<n>` numpad alignment baked into renderer** (`AnimatedRenderedDecoder` honours `RenderState::alignment` from `\an<n>` + legacy `\a<n>`; decomposes 1..9 into horizontal `TextAlign` + `VerticalRow` — bottom-row 1/2/3 keeps last-baseline anchor; top-row 7/8/9 anchors first baseline at `bottom_margin_px + ascent`; middle-row 4/5/6 centres the full block on canvas mid-line; horizontal column overrides `CuePosition::align`; 13 tests with DejaVuSans TTF fixture) |

**Bitmap-native (own crate)** — `oxideav-sub-image`:

| Format              | Decode | Encode | Notes |
|---------------------|:------:|:------:|-------|
| **PGS / HDMV** (`.sup`) | ✅ | ✅ | Blu-ray subtitle stream; PCS/WDS/PDS/ODS + RLE + YCbCr palette → RGBA + r183 RLE codec property+negative sweep (1500 randomised roundtrips + edge cases) |
| **DVB subtitles**   | ✅ | — | ETSI EN 300 743 segments + 2/4/8-bit pixel-coded objects |
| **VobSub** (`.idx`+`.sub`) | ✅ | — | DVD SPU with control commands + RLE + 16-colour palette + r201 SP_DCSQ 0x07 CHG_COLCON length-skip (mpucoder-spu §size-word validated, payload bounded, surfaces via `Spu::saw_chg_colcon`; 49 tests). Lacks per-rectangle palette/alpha-override application during render |

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
