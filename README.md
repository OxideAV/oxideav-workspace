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
| WAV       | ✅ | ✅ | ✅ | LIST/INFO + BWF `bext` (EBU 3285) + smpl/inst/plst Playlist + r193 `fact` chunk per RIFF MCI §3 + r205 **`iXML` chunk** (third-party production-recorder UTF-8 XML payload trimmed at NUL + `wav:ixml.body_len` always-on raw size for NUL-padded reserved-for-editing regions; odd-length bodies force RIFF 1-byte pad; 51 unit tests) |
| FLAC      | ✅ | ✅ | ✅ | VORBIS_COMMENT, streaminfo, PICTURE block; SEEKTABLE-based seek; CUESHEET round-trip (read + write per RFC 9639 §8.7); r182 in-place symmetric-pair Levinson-Durbin update (encoder, eliminates up to 36 Vec allocs/subframe, bit-exact regression-pinned) |
| Ogg       | ✅ | ✅ | ✅ | Vorbis/Opus/Theora/Speex pages + comments; page-granule bisection + page-level seek index + chained-link-aware duration + page-loss/hole detection + page-sync recapture + public CRC-32 validation API + r172 Criterion bench harness + r183 streaming CRC + r185 Skeleton 3.0/4.0 + r192 slice-by-4 CRC-32 + branch-free `compute_page_checksum` (3-segment dispatch drops 65535 branches from max-size page; **page/parse/max 493 MiB/s → 1.3 GiB/s = 2.5-3× over r172**) + r196 Skeleton 4.0 `index` packet index-accelerated `seek_to` (per-stream keyframe index pre-walk skips O(N) bisection on indexed streams) |
| Matroska  | ✅ | ✅ | ✅ | MKV/MKA/MKS; Cues seek; SeekHead/Chapters/Attachments/subtitles; opt-in block lacing on write; EBML CRC-32 validation + r186 per-Cluster CRC-32 validated on advance() + Cue-driven seek; typed Tag/TrackOperation/ContentEncodings/chapters() decode; typed Video FlagInterlaced/FieldOrder + geometry quartet + Colour master + SMPTE 2086 MasteringMetadata + StereoMode + r177 Projection + r183 AlphaMode/AspectRatioType/UncompressedFourCC typed decode + r196 §5.1.6 write-side Attachments + r202 §6.2 write-side CRC-32 on Top-Level masters + r208 **§5.1.4.1.28 Video FlagInterlaced + FieldOrder write** (`MkvMuxer::set_video_interlacing(stream_index, FlagInterlaced, Option<FieldOrder>)`; spec-gated post-header / non-video / FieldOrder-only-with-Interlaced rejects; omitting call leaves elements off-disk so demuxer materialises defaults; demux-side `FlagInterlaced::to_raw()` / `FieldOrder::to_raw()` round-trip every Table 3 / Table 4 value plus `Other(u64)` forward-compat; 224 tests) |
| WebM      | ✅ | ✅ | ✅ | First-class: separate fourcc, codec whitelist (VP8/VP9/AV1/Vorbis/Opus); inherits Matroska Cues seek |
| MP4       | ✅ | ✅ | ✅ | mp4/ismv; faststart; iTunes ilst; fragmented demux+mux (DASH/HLS/CMAF) + sidx/mfra/tfra/styp; AC-3/E-AC-3/DTS sample entries; subtitle/timed-text; protected sample-entry unwrap; typed track refs + edts/elst mux + elng + kind + cslg + stsh + sdtp + sample-group sbgp/sgpd + §8.16.5 prft demux + r162 atom-walker robustness + r182 sidx-driven seek fast-path + r189 `read_box_header` largesize overflow reject + r196 ISO/IEC 23001-7 §8 CENC parser + r203 §8.7.8-9 `saiz`/`saio` Sample Auxiliary Information parser (track + traf surfaces; 32-bit/64-bit `saio` offsets widened to u64; structured `Mp4Demuxer::sai_records()` typed accessor for CENC consumers; 10 new tests = 179 lib); lacks AES-CTR/CBC decryption driver |
| MOV (QuickTime) | ✅ | — | ✅ | Apple QTFF + ISO BMFF meta + HEIF/HEIC item-properties + grid/iovl/tmap + symmetric muxer + fragmented-MP4 seek + DASH sidx/styp + stbl + traf saiz/saio sample-aux + r182 ISO 14496-12 §4.2/§11.1 `uuid` User-Type Box parser + r187 largesize overflow reject + r199 §8.3.4 `trgr` Track Group Box + r204 §8.7.3.3 **`stz2` Compact Sample Size Box** at stbl scope (Mp4Box-style 4/8/16-bits-per-entry packing; entries widen to `u32` into `SampleTable::stsz_table` so all downstream consumers work unchanged; `SampleSizeSource::{Stsz, Stz2 { field_size }}` discriminator + `MovDemuxer::sample_size_source(track_index)`; field_size ≠ 4/8/16 rejected per §8.7.3.3.2, non-zero 24-bit `reserved` rejected per §8.7.3.3.1, MSB-first 4-bit packing with trailing-nibble silent drop; first-wins on malformed both-stsz-and-stz2; 10 new tests = 468 lib); ffprobe-accepted |
| AVI       | ✅ | ✅ | ✅ | AVI 1.0 + OpenDML 2.0 demux/mux; AVIX/dmlh/vprp + 2-field interlaced + VBR audio + LIST INFO + typed PaletteChange/TextChunk/AvihFlags/Idx1Flags + r197 OpenDML AVISUPERINDEX `bIndexSubType` surface (`super_index_sub_type` / `super_index_is_2field` / `avi:indx.<n>.sub_type_2field` metadata; AVI_INDEX_SUB_2FIELD == 0x01) + ODML keyframe seek + WAVEFORMATEXTENSIBLE + `strn`/`strd` + CBR-audio validator + dmlh.dwTotalFrames + IDIT/ISMP/rcFrame/wLanguage + dwInitialFrames + r163 typed `dwChannelMask`/`Speaker`/`ChannelLayout` + r182 typed `strh.wPriority` + r203 per-stream `strh.dwStart` (32-bit DWORD at AVISTREAMHEADER byte 28; `AviDemuxer::stream_start(idx)` + `AviMuxOptions::with_stream_start(idx, start)`; default-0 → None mapping; 11 round-trip tests) |
| Blu-ray (BD-ROM) | ✅ | — | — | `oxideav-bluray` Phase 2 — UDF 2.50 mount (ECMA-167 3rd ed.) + BDMV walk (`index.bdmv`/`MovieObject.bdmv`/`.mpls`/`.clpi`) + `.m2ts` stream (192→188-byte TP_extra_header strip) + `bluray://` URI handler with auto-detect; r93 typed `Cpi { ep_map: Vec<EpMap { stream_pid, ep_stream_type, entries: Vec<EpEntry { pts_ep_start, spn_ep_start, is_angle_change_point, … }> }> }` CPI EP_map decode per BD-ROM AV §5.7 (coarse + fine two-level table folded into a flat per-PID list a seeker can binary-search); r96 keyframe-aligned `TitleSource::seek_to(pts_90k)` (PTS→clip→I-frame→SPN×192, AACS-unit-aligned); `StreamDecryptor` trait hooks `oxideav-aacs` without hard dep. + r180 multi-angle PlayItem parsing (BD-ROM Part 3 §5.4.4.1) + `open_title_with_angle` / `max_angle` per-angle title open (AV §5.2.3.3) + r188 `Disc::chapters(title)` from PlayListMark entry marks + r200 `Disc::title_streams(title) -> TrackCatalogue` deduplicating per-PlayItem STN_table entries by `(PID, kind)` (AV §5.2.3.3 / Part 3 §5.4.4.4) + mount-time `TitleInfo::languages` from audio/subtitle entries (133 tests). Lacks HDMV opcode exec, BD-J, mid-stream angle switching, cross-PlayItem STC PTS remap |
| DVD-Video | ✅ | — | — | `oxideav-dvd` Phase 3b — ISO 9660 + UDF 1.02 mount + VIDEO_TS walk + IFO body parser (VMGI/VTSI + TT_SRPT + VTS_PTT_SRPT + PGCI [+ PGC subpicture colour-LUT + pre/post/cell nav command table] + VTS_C_ADT + chapter materialiser) + VOB demux (MPEG-PS pack/PES + Nav-Pack PCI/DSI [+ PCI highlight + DSI typed sections] + DVD substream router for AC-3/DTS/LPCM/subpicture) + VOB → MKV mux (`mkv-output` feature; per-PES PTS preserved + ChapterAtom per `DvdChapter` via RFC 9559 §5.1.7) + `dvd://` URI handler + r172 typed NavInstruction VM disassembler (Phase 3c precursor: full Link family + 13-entry link-subset + Jump/Call SS + Set arithmetic + Type 4..6 classifier). + r179 Sub-Picture Unit (SPU) decoder (SPUH+DCSQT walker, 8 typed commands, 2-bit/four-form PXD RLE, 90 kHz STM-DTS conversion) + r188 SPU RGBA compositor (`composite()`: SET_COLOR/SET_CONTR → PGC palette LUT → BT.601 studio-swing YCbCr→RGB + top/bottom-field PXD interleave) + r200 Phase 3c VM execution (RegisterFile w/ SPRM defaults + RSM call/return stack + `step()/run_list()` honoring Goto/Break/Exit with step-budget; SET-arithmetic + 7 CmpOps + 12 SetOps) + r207 **Phase 3c Type 4..6 compound SET+CMP+LNK** (`SetCLnk`/`CSetCLnk`/`CmpSetLnk` extended from classifier stubs to full operand triples: `scr`/`sr1` selectors, SET source reg-or-imm, CMP RHS reg-or-imm, Type 5 independent `cmp_lhs`, 6-bit `hl_bn`, 5-bit Link subset + two-row Illegal (`SET-dir=1 + CMP-dir=1`) → `NavInstruction::Invalid`; `exec_set_clnk`/`exec_cset_clnk`/`exec_cmp_set_lnk` implement ordered SET/CMP/LINK per family; `fire_link` Nop→Continue / Rsm→Resume (pops RSM) / Invalid→Continue; Type 6 unconditional Link distinguishes from Type 5; 177 lib + 187 mkv-output tests). Lacks CSS auth (`oxideav-css`) |
| MP3       | ✅ | — | ✅ | demuxer LANDED (ID3v2/ID3v1 skip + Xing/Info VBR + CBR/VBR seek_to); r177 Decoder-trait stereo widening (independent + joint MS + intensity, planar AudioFrame) |
| IFF (EA IFF 85) | ✅ | ✅ | — | One crate for the whole `FORM/LIST/CAT` family — Amiga `8SVX` audio + `ILBM` images (1..8-plane indexed + 24-bit literal-RGB true-colour, EHB/HAM6/HAM8, ByteRun1, HasMask, GRAB, SHAM, PCHG; CRNG/CCRT/DRNG `cycle_step`) + `ANIM` (op-0 literal + op-5 vertical-delta encode/decode + r192 op-7 Short/Long Vertical Delta decode) + Apple `AIFF / AIFF-C` (FORM/COMM/SSND walker, 80-bit IEEE-extended sample-rate decode, NONE/twos/sowt/raw/fl32/FL32/fl64/FL64 PCM, codec-bearing FourCCs ima4/ulaw/alaw routed to sibling crates) + r198 §6.0 AIFF MARK chunk parsing + r203 §9 AIFF `INST` (Instrument) chunk parsing (`InstrumentChunk { baseNote / detune / low+highNote / low+highVelocity / gain / sustainLoop / releaseLoop }` + `PlayMode { NoLooping / ForwardLooping / ForwardBackwardLooping }` + `resolve_sustain_loop`/`resolve_release_loop` join against MARK with begin<end ordering guard; MIDI 0..=127 validation) + r209 **ANIM op-7 encode + AIFF COMT/AESD/APPL surfacing + MARK/INST write-side** (`anim::encode_op7_body`/`encode_anim_op7` greedy Skip/Same/Uniq per column + 64-byte pointer table + 8 op/data-lists; `Form::comments`/`aesd`/`applications` dup-rejecting accessors; `write_marker_chunk`/`write_instrument_chunk`/`write_comments_chunk`/`write_aesd_chunk`/`write_appl_chunk` complete the round-trip; +21 tests); lacks ANIM op-8 + DEEP/TVPP/RGB8/RGBN true-colour (#1368) |
| IVF       | ✅ | — | — | VP8 elementary stream container |
| AMV       | ✅ | ✅ | — | Chinese MP4 player format (RIFF-like) — r191 clean-room demuxer rebuilt from `docs/container/amv/amv-container-trace.md`: position-coded RIFF prelude + `amvh` packed `[s,m,h,0]` duration + 20-byte WAVEFORMATEX + §4 no-byte-padding chunk walker + two-stream Demuxer (video=mjpeg, audio=adpcm_amv) + `AMV_END_` trailer + r197 byte-faithful `AmvMuxer` + r203 `Demuxer::seek_to` (linear walk over `movi`; cumulative §4b PTS for audio stream; rewind-on-backwards + forward-walk-from-cursor) + r208 **lazy `build_chunk_index` + `chunk_index()` cache for O(log N) repeated seeks** (one-shot walk records every chunk's file offset/kind/pre-emit per-stream PTS; `seek_to` short-circuits disk-walking loop; build is idempotent + preserves walker cursor for mid-walk invocation; 48 tests) |
| FLV       | ✅ | ✅ | — | Flash Video — MP3/AAC/H.264 audio + VP6f/VP6a/H.264 video + Enhanced RTMP ExVideoTagHeader + AMF0 onMetaData/onXMPData/onCuePoint + Annex F encryption + E-FLV ModEx walk + multitrack body splitter + HDR colorInfo metadata + r161 injection-robustness suite + 16 MB OOM-lever guard + r182 onMetaData catch-all preserves Date/Null/StrictArray/AMF3-nested + r186 unknown-script-name argument preservation + r196 first muxer slice (audio-only) + r202 §E.4.3 / §E.4.3.1 video-tag muxer slice (write_h263_tag + write_vp6_tag + write_vp6a_tag w/ AlphaOffset + write_avc_sequence_header + write_avc_nalu_tag w/ SI24 CompositionTime + write_avc_end_of_sequence + VideoTagHeader↔byte round-trips) + r209 **Enhanced-RTMP ExVideo + ExAudio muxer slice** (`write_ex_video_tag` / `write_ex_audio_tag` + 15 per-codec writers covering av1/vp9/hvc1[CTO]/vvc1 + Opus/fLaC/ac-3/ec-3/mp3/mp4a; `Ex{Video,Audio}TagHeader::to_bytes` wire-byte inverses; multitrack OneTrack/ManyTracks/ManyTracksManyCodecs; 283 tests) |
| WebP      | ✅ | ✅ | — | RIFF/WEBP (lossy + lossless + animation; ANIM + ANMF emit) |
| TIFF      | ✅ | ✅ | — | TIFF 6.0 single-image + r177 BigTIFF write (magic 43 / 8-byte offsets / LONG8 strip+tile arrays) + r183 PhotometricInterpretation=8 1976 CIE L*a*b* decode + r185 CCITT T.4 2-D + T.6 (Group 4) fax decode (READ algorithm; tiffcp-oracle pixel-exact) + r206 §23 CIE L*a*b* **encode** (`CieLab8 { pixels }` 3-sample chunky + `CieLabL8 { pixels }` 1-sample L*-only; composes with `Predictor=2`, tiled §15 + BigTIFF; `PlanarConfiguration=2` rejected on L*-only; CCITT rejected via bilevel-only gate per §10/§11) |
| PNG / APNG| ✅ | ✅ | — | 8 + 16-bit, all color types, APNG animation + r188 gAMA/cHRM round-trip + r202 §4.2.10 zTXt compressed-textual-data round-trip (PNG3 §11.3.3.3; deflate body + compression-method=0 enforced + 166 tests); metadata lacks only iCCP/iTXt |
| GIF       | ✅ | ✅ | — | GIF87a/GIF89a, LZW, animation + NETSCAPE2.0 loop + multi-frame compositor (§23 disposal-method state machine, 4 modes) + r181 `GifImage::frames_with_palette` §21 active-table iterator + r188 §23 `has_transparency()` / `requires_user_input()` stream-level GCE flag queries — clean-room rebuilt from CompuServe spec |
| JPEG      | ✅ | ✅ | — | Still-image wrapper around the MJPEG codec |
| BMP       | ✅ | ✅ | — | Windows bitmap — DIB headers BITMAPINFOHEADER / V4 / V5, 1/4/8/16/24/32-bit + r182 BI_ALPHABITFIELDS (compression=6, V3 four-mask alpha variant); also exposes the DIB helpers used by ICO / CUR sub-images |
| Netpbm    | ✅ | ✅ | — | All seven PNM magics + PAM (P1-P7) + PFM (Pf/PF); 1/8/16-bit + IEEE-754 binary32; comment-tolerant ASCII + binary; .pbm/.pgm/.ppm/.pnm/.pam/.pfm + r183 user-defined PAM TUPLTYPE + r189 ASCII (P1/P2/P3) hot-path rewrite + r205 PFM big-endian byte-swap row helper + r210 P5/P6/P7 16-bit `read_be16_row`/`write_be16_row` autovectorizable helpers (encode P5 16-bit ≈ -4 %, P6 16-bit ≈ -2 %; same shape as the r205 PFM 32-bit helper) |
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
| **Vorbis** | 🚧 r9 (post-2026-05-20 orphan) — identification + comment + §3.2.1 codebook + Huffman tree + full §4.2.4 setup-header walker + §3.2.1/§3.3 VQ vector unpack + §8.6 residue decode (formats 0/1/2) + §7.2.3/§7.2.4 floor type 1 + §6.2.2/§6.2.3 floor type 0 LSP + §1.3.2/§4.3.1 Vorbis window + §4.3.5 inverse channel coupling + §4.3.3 nonzero-vector propagate + §4.3.6 floor×residue + §4.3.1–§4.3.8 audio-packet driver + r180 §4.3.7 IMDCT + r186 streaming overlap-add | 🚧 r2 — r195 §4.2.1+§4.2.2 identification-header WRITE + §5.2.1 comment-header WRITE + r201 §3.2.1 codebook WRITE + §9.2.2 `float32_pack` (bit-exact roundtrip across 3 length encodings × 3 lookup types; auto-picks densest sparse/ordered/dense) + r206 **§7.2.2 floor type 1 WRITE** (`write_floor1_header` bit-exact inverse of round-9 parser, proven for every legal input; 13-variant `WriteFloor1Error` invariant gate; 9 roundtrip fixtures spanning all edge cases; 409 tests); lacks §6.2.1 floor 0 WRITE + §8.6 residue WRITE + audio-packet WRITE |
| **Opus** | 🚧 ~33% — RFC 6716 range decoder + full SILK pipeline + §4.3 Table 56 CELT pre-band header + §3.1/§4.2 framing dispatch + r182 celt_band_layout + r183 §4.3.4.3 spread + r190 §4.3.4.5 TF-resolution lookup + r191 §4.3.3 LOG2_FRAC_TABLE + intensity_rsv/reserve_stereo + r193 §4.5.1 CELT redundancy + r195 §4.5.2 SILK+CELT state-reset policy (`decide_state_resets` 4 rules → `StateReset {silk, celt: None/BeforeFrame/BeforeRedundantOnly}`; full 3×3 mode × redundancy matrix + §4.5.3 Figure 18 cross-checks; 467 + 20 tests); §4.5 mode-switching now §4.5.1 + §4.5.2 complete; §4.5 mode-switching now §4.5.1 + §4.5.2 complete + r200 §4.5.1.4 redundant-frame decode params + r204 **§4.3.2.1 CELT coarse-energy Laplace-model parameter surface** (`E_PROB_MODEL: [[[u8;42];2];4]` 336 Q8 bytes via `e_prob_pair(lm, mode, band)` + typed `EnergyPredictionMode::{Inter, Intra}` + intra `α=0` / `β=4915/32768` Q15 constants; 514 lib + 20 integration tests; per-LM inter-mode `(alpha, beta)` deferred as §4.3.2.1 docs gap "depend on the frame size in use"); CELT bands still gated on #936 | 🚧 scaffold |
| **MP1 / MP2** | ✅ Layer I + Layer II decode to PCM + §2.4.3.1 CRC-16 verify + mp2 frame-level decode loop + r191 Annex D Phase-2 psy + r203 Annex D Phase-3 Step 3 LTq offset + Model 2 spreading + (allocator still pending D.1/D.3/D.4 PNG→text #1262) | 🚧 ~82% — Layer I encoder + Layer II §C.1.5.2.7 bit-allocation + r192 §C.1.3 polyphase analysis filterbank + r197 §C.1.5.1.4 Layer II per-part scalefactor extraction; lacks top-level `Mp1Encoder` Layer II switch + Table C.4 SCFSI selection (PNG→text gap) |
| **MP2** | 🚧 ~38% (post-2026-05-24 orphan) — §2.4.1.3/§2.4.2.3 Layer II header parser + §2.4.3.1 frame sizing + Annex B tables + joint-stereo + scfsi + §2.4.3.3.4 sample requantizer + r185 full LSF Layer II + r202 §C.1.5.2.5/§C.1.5.2.6 SCFSI Table C.4 encoder-side selection + r208 **§2.4.1.6 `write_audio_data` encoder bit-for-bit inverse of parser** (`write_audio_data` + `write_audio_data_with_section_bits`; emits per-(sb, ch) `nbal` allocation indices, 2-bit `scfsi`, 1/2/3 on-wire 6-bit scalefactor indices in spec order; joint-stereo `sb >= bound` writes ONE shared allocation per subband; `_with_section_bits` returns alloc/scfsi bit lengths so future CRC accumulator can index Annex B Table B.5 without re-parsing; new `AudioDataWriteError` 5-variant gate; 188 tests); lacks §2.4.3.2 polyphase synthesis + §C.1.5.2.7 iterative bit-allocator | 🚧 scaffold |
| **MP3** | ✅ ~100% — bit-exact decode + ID3v2/Xing seek + MPEG-2.5 framing; 634 tests | 🚧 ~95% — Phase-2 + r194 long + r197 pure-short + r204 mixed-block per-band threshold-in-quiet path + r207 **trait-API one-shot Annex D threshold-in-quiet factory** (`Mp3Encoder::new_with_threshold_in_quiet(bitrate_kbps, sample_rate_hz, mode)` + `codec_encoder::make_encoder_with_threshold_in_quiet`; per-channel bitrate `bitrate_kbps/nch` drives §D.1 Step 3 −12 dB offset — 192k stereo exact cutover at 96k/ch, 128k mono triggers, 128k stereo does not; 556 tests); lacks Model 1/2 psy + intensity-stereo |
| **AAC** | 🚧 Phase 1 — ADTS + raw_data_block walker + AudioSpecificConfig + program_config_element + r177 §4.4.1 GASpecificConfig extensionFlag + Table 1.15 epConfig + r192 §1.6.5 Table 1.15 trailing `syncExtensionType=0x2b7` implicit-SBR/PS/ER-BSAC probe (`AudioSpecificConfig.trailing_sbr_probe`; ext-AOT 5 reads sbrPresentFlag + optional 4-bit ext-sfi w/ 24-bit escape + secondary `0x548` sync gating psPresentFlag; ext-AOT 22 reads sbrPresentFlag + mandatory 4-bit ext-channel-config; `parse_bits_bounded` for LATM/esds carrier-bounded callers) + r194 §4.5.4.1 SWB offset tables + §4.6.13 `apply_pulse_data` reconstruction + r200 §4.6.9.4 TNS_MAX_ORDER/BANDS clamp surface + r207 **§4.4.6 Table 4.50 `ics_body` walker** (`IcsBody::parse`/`parse_with_ics_info`/`write`/`write_with_ics_info` compose global_gain + ics_info + section_data + scale_factor_data + optional pulse_data + tns_data + gain_control_data into one Table 4.50 cycle up to but not including spectral_data; covers SCE/LFE/non-shared-CPE/CPE-common_window; Table 4.50 Note 1 no-pulse-on-EIGHT_SHORT + §4.6.12 gain_control_data AOT-3-only enforced; scale_flag=true scalable AOT 6 rejected; 503 tests); decoder body still pending Huffman codebooks 1-11 + raw_data_block→ics_body wiring | 🚧 scaffold — Phase-2 writers: section_data + ics_info + pulse_data + tns_data + scale_factor_data + DPCM + r160 raw_data_block + r165 Pce::write + r183 gain_control_data SSR + r187 §4.4.2.7 extension_payload; SBR types pending QMF |
| **CELT** | 🚧 r13 (post-2026-05-20 orphan) — RFC 6716 §4.1 range decoder + §4.3 prefix + §4.3.2.1 coarse-energy scaffold + §4.3.3 bit-allocation fields + §4.3.4 tf_change/tf_select + r181 §4.3.4.3 spread + r187 §4.3.7.1 post-filter + §4.3.7.2 de-emphasis + r195 §4.3.4.5 Walsh-Hadamard primitives + r200 §4.3.3 `cache_caps50` + dynamic-band-boost decode loop + r207 **§4.3.3 initial-reservations budget walk** (`compute_initial_reservations(frame_bytes, ec_tell_frac, is_transient, lm, stereo, coded_bands) → InitialReservations` chains 4-step `total_initial`→anti-collapse→skip→intensity→dual-stereo arithmetic; `InitialReservations::gates_for_band_allocation` synthesises `BandAllocationGates` incl. trim gate; constants `RSV_BIT_8TH=8` / `RSV_INITIAL_SLACK_8TH=1`; 198 tests); blocked on docs #936 (Laplace) | 🚧 scaffold |
| **Speex** | 🚧 ~28% — Ogg stream-header + NB + WB high-band + §5.5 in-band signalling + r179 `BitWriter` + r187 encoder-side `write` + r191 22 CELP companion-table accessors + r194 NB LSP-VQ → Q10 LSP reconstruction + r200 §9.1 per-sub-frame LSP linear interpolation + r208 **NB 3-tap pitch-gain VQ reconstruction** (Manual Eq. 9.1 / CELP companion §2.2: `PitchGainTaps { taps: [i16; 3] }` β taps `(g0, g1, g2)` of LTP convolution `ea[n] = g0·e[n−T−1] + g1·e[n−T] + g2·e[n−T+1]`; resolves 5-bit/7-bit pitch-gain VQ; +32 codebook bias applied in-module; column 3 `search_aid` dropped; `NarrowbandSubFrameIndices::pitch_gain_taps(submode)` lookup; 205 tests); lacks §9.1 LSP→LPC + synthesis + UWB framing | 🚧 scaffold |
| **GSM 06.10** | 🚧 ~30% — r185 clean-room §5.3 fixed-point RPE-LTP decoder pipeline + r200 §4.4 in-band decoder-homing protocol + r200 §5.1 `norm`/`div` saturating primitives; per-container framing for `.gsm` / RTP / MS-GSM WAV still DOCS-GAP; lacks §6 conformance vectors | 🚧 r207 — §5.2.0..§5.2.3 **encoder pre-processing pipeline** (`PreProcessor` maps `sop[0..159]` to §5.2.4-input `s[0..159]`: §5.2.1 `downscale_frame` `>>3 then <<2`, §5.2.2 high-pass IIR with persisted `z1`/`L_z2` state per §4.5 Table 4.2, §5.2.3 first-order pre-emphasis `s[k] = sof[k] + mult_r(mp, -28180)`); next: §5.2.4 LPC + §5.3 segmentation |
| **G.711** (μ/A-law) | ✅ 100% | ✅ 100% |
| **G.722** | 🚧 r185 clean-room SB-ADPCM decoder bring-up against staged ITU-T G.722 Recommendation + r200 BLOCK1/QMF predictor split into shared `src/predictor.rs` + r207 **Table 19 RIL=11111 sign-anomaly fix** (5-bit Mode-2 inverse-quantizer entry now correctly in the `SIL = −1` column per printed table p.40, parallel to Table 18's `111110/111111` smallest-magnitude-negative anomaly; pure-top-bit-as-sign mis-classifies these adjacent-to-positive-half codewords; 6 new tests) | 🚧 r200 SB-ADPCM **encoder bring-up** — 24-tap transmit QMF (clause 3.1) + 60-level QUANTL + 4-level QUANTH (clause 6.2.1.1) + 64 kbit/s octet multiplexer + Tables 16/20 forward output codes + r207 Mode-2 + Mode-3 silence-envelope round-trip green (37 tests); lacks Appendix-II conformance fixtures |
| **G.723.1** | ✅ 100% | ✅ 100% — both 5.3k + 6.3k |
| **G.728** | 🚧 ~16% — clean-room decoder front-end: Annex A/B/C/D tables + block-50 Levinson-Durbin + blocks 29/31/32 + r195 blocks 30/33 backward synthesis-filter + vector-gain adapters + r201 blocks 73-77 postfilter AGC tail + r207 **block 72 short-term (spectral) postfilter** (`H_s(z) = (1 − Σ b̄_i z⁻ⁱ) / (1 − Σ ā_i z⁻ⁱ) · (1 + µ·z⁻¹)` per eq 4-2..4-5; `b̄_i = ã_i·0.65^i`, `ā_i = ã_i·0.75^i`, `µ = 0.15·k1`; order-10 Levinson byproduct on `rtmp[..=10]` for `k1 = -R(2)/R(1)`; coeffs refresh at first vector of every adaptation cycle ICOUNT=1; full block 29 → 33 → 72 → 73..77 chain now live; 96 tests); lacks postfilter block 71 (long-term/pitch) | 🚧 scaffold |
| **G.729** | 🚧 ~10% — clean-room from staged trace #859: r173-r195 tables + serial parser + LSP-quantiser codebooks + corpus harness + r201 §3.2.4 MA-predictor `fg` family + r207 **§3.2.4 LSP-frame reconstruction pipeline** (codebook sum eq 19 → twice-applied rearrangement `J=0.0012` then `J=0.0006` per fig F0013-01 → MA-prediction eq 20 via `L0`-selected predictor → 4-step stability clamp floor 0.005 / min-gap 0.0391 / ceil 3.135; `LspReconstructor` carrying 4-frame MA history initialised to `l̂_i = i·π/11`; Q13/Q15 boundary conversion; 43 tests); lacks §3.2.5 interpolation + §3.2.6 LSP→LP + gain GA/GB + postfilter + Annex B DTX | 🚧 scaffold |
| **IMA-ADPCM (AMV)** | ✅ 100% | ✅ 100% |
| **MS-ADPCM / IMA-ADPCM (WAV)** | ✅ 100% | ✅ 100% — block-aligned WAV encoder for both nibble layouts |
| **OKI / Dialogic VOX** | ✅ 100% — r186 clean-room from Dialogic app note 00-1366-001 (1988); HiFirst (VOX/MSM6295) + LoFirst (MSM6258) nibble orders, Native12 + Wide16 output | ✅ 100% — symmetric §3 closed-form encode; mono-only via registry (Dialogic hardware constraint) |
| **8SVX** | ✅ 100% | ✅ 100% |
| **iLBC** (RFC 3951) | ✅ 100% — NB 20/30 ms | ✅ 100% |
| **AC-3 / AC-4** (Dolby Digital / Dolby AC-4) | ✅ ~97% — AC-3 full decode + E-AC-3 SPX + TPNP + AHT + §7.8.2 LtRt downmix + r126 Annex D mix-level + WAVE_FORMAT_EXTENSIBLE + r172 SPX-attenuation border + r182 §7.10.1 CRC verifier + r187 §7.10.1 augmented crc2 + r193 typed `BitStreamMode` accessor for Table 5.7 + r196 §E.2.3.1.8 E-AC-3 `chanmap` routing + r202 §7.7.2.2 typed `CompressionGain` + r208 **typed xbsi2 / informational-metadata Dolby Surround EX + Dolby Headphone + A/D-converter type** (`dsurexmod`/`dheadphonmod`/`adconvtyp` enums per Tables D2.7/D2.8/D2.9 lifted from parse-and-discard; Annex-E-only `adconvtyp_ch2` for 1+1 dual-mono; spec-gated `bsid==6 && xbsi2e==1` for AC-3, `infomdate==1` + per-acmod gates for E-AC-3; 218 tests) | 🚧 AC-3 ~95% — acmod 1/2/2.1/3/6/7 + LFE + DBA + 5-fbw coupling + E-AC-3 indep+dep + per-channel PSNR gates + r95 two-stage equalise + spread-cap greedy for per-channel `fsnroffst[ch]` |
<!-- ac3 decode r129: E-AC-3 mixmdata mix-levels (ltrt/loro c/sur) now surfaced + routed through §7.8 downmix in process_eac3_frame -->
| **AC-4** (Dolby) | 🚧 ~98% — A-SPX + DRC + 60+ ETSI codebooks + 5_X/7_X ACPL_1/2/3 + cfg0/1/2/3 + LFE + SSF/SNF + SAP + Pseudocode 121 companding + IMS bitstream_version≥2 walker + r181 §5.7.7.7 Pseudocode 121 + r190 Table 126 `aspx_int_class = FIXFIX` writer width fix; lacks ETSI fixture RMS audit, object/a-joc | 🚧 IMS ~72% — v0/v2 TOC + mono/stereo/joint M/S + 5.0/5.1/7.1 SIMPLE Cfg3Five + 5_X SIMPLE/ASPX_ACPL_1/2 + ASPX_ACPL_3 + r132/r135/r139/r144 real per-band α+β for ACPL_1/2 + r193 real per-band β1/β2 for 5_X ASPX_ACPL_3 + r196 real per-band α1/α2 for 5_X ASPX_ACPL_3 + r202 real per-parameter-band α + β for 7.0/7.1 SIMPLE/ASPX_ACPL_2 + r208 **5_X SIMPLE/ASPX_ACPL_3 real per-band γ5/γ6 extraction** (§5.7.7.6.2 Pseudocode 118 step 7 centre output `z4 = 0.5·(γ5·x0in + γ6·x1in)`, then `*= √2` step 11, `x0in = (1+√2)·L` step 1 → `C ≈ K·(γ5·L + γ6·R)` with `K = 1+√(1/2)`; extractor solves 2×2 normal equations per parameter band minimising `Σ (C/K − γ5·L − γ6·R)²`; degenerate Gram keeps γ5=γ6=0; Table-208 linear quantiser with symmetric ±cb_off clamp; 813 tests); lacks γ1..γ4 (need 5.1+Ls+Rs PCM input layout) + 7_X ACPL_3 β + ASPX envelope + Table-181 SAP residual + back-pair Lb/Rb |
| **MIDI** (SMF) | ✅ ~99% — SMF Type 0/1/2 → PCM via 32-voice mixer + SF2/SFZ/DLS + FF 01..07 text-meta iterator family (10 helpers) + r208 **`smpte_offsets()` FF 54 + `FrameRate` enum** (`SmpteOffsetEvent { tick, track, hours_raw, minutes, seconds, frames, subframes }` pinned to parent-track absolute tick + stably merged per track-0-before-track-1 rule; `FrameRate::{Fps24, Fps25, Fps30DropFrame, Fps30NonDrop}` decoded from `hr` byte per RP-004/008 bits 5-6 rate + bits 0-4 hours; iterator family now 11 helpers; 376 tests); r172 cargo-fuzz (30M+ panic-free) | — synthesis only |
| **NSF** (NES) | 🚧 ~96% — full 6502 + IRQ/NMI + 5/5 2A03 APU + DMC DMA + six expansion chips + NSF v1/v2/NSFe + Dendy region + r154 Namco 163 + r185 VRC7 OPLL pipeline + r199 VRC7 register semantics + r204 **VRC7 KSR (Key Scale of RATE)** per YM2413 §III-1-2 Table III-2 (`Envelope::update_rks(block, fnum_msb)` cached RKS: KSR=0 → `block >> 1`; KSR=1 → `(block << 1) \| fnum_msb`; 4-bit per-stage R widens to 6-bit RATE = 4·R + RKS via `Envelope::effective_rate(r)` with explicit R=0→RATE=0 halt carve-out; pitch-only `$1X`/`$2X` writes trigger mid-note `refresh_rks` glide; 213 tests). + r209 **§4 KSL (Key Scale of LEVEL) formula scaffold** (`ksl_attenuation_env_levels`/`ksl_base_attenuation` + `KSL_BASE_BYTE_TABLE: [[u32; 16]; 8]` exposing `(base) >> (3 - KSL)`; block 0 bit-exact; blocks 1..=7 zero scaffold awaiting #1363; `OpllChannel.{mod_ksl,car_ksl}` capture on `load_patch`; trip-wire test; 202 tests; Rule-E scrub of `src/opll.rs:23-43` resolves #1339). Lacks §4 byte base table rows 1..=7 (#1363) + §7 per-rate env tables + rhythm mode | — synthesis only |
| **Shorten** (.shn) | 🚧 r13 (post-2026-05-18 orphan) — `ajkg` magic + v2/v3 ulong + svar(n) + per-block function dispatch + VERBATIM/QUIT + DIFF0..3 + Rice residual + per-channel carry + spec/05 §2.5 running mean + QLPC predictor + r7 `decode_stream` + r145 `Decoder` trait + r181 block-by-block + r187 streaming `Decoder` + r191 envelope encoder surface + r197 **`write_diff0_block` predictor encoder** (full `<fn=0> <energy> <residual>×bs` command per spec/03 §3.1 + spec/05 §3.1; `min_energy_for_diff0` selector; encode→decode round-trips byte-exact through `decode_stream` across DIFF0+VERBATIM splice, silent block, ±100 max-natural residuals; +15 tests = 203)+ r209 **`write_diff1_block` order-1 polynomial-difference predictor encoder** (per spec/03 §3.2 + spec/05 §1.1 + §3.1; seeds `s(t−1)` from `carry.at(0)`, writes `e₁(t) = s(t) − s(t−1)` under `svar(energy_encoded + 1)`; mean-invariant per spec/05 §2; `min_energy_for_diff1` natural-energy selector; byte-exact roundtrips via `decode_stream`; 224 tests); lacks DIFF2/3/QLPC predictor encoders + #1267 spec/04 §2 BLOCK_FN_QUIT contradiction | 🚧 scaffold |
| **TTA** (True Audio) | ✅ ~98% — TTA1 fmt=1/2 + password + ID3v1/APEv2 trailer + r187 streaming + random-access decode API + r198 streaming bench parameter-cube + r204 **`Decoder::new_with_password` brings streaming + random-access onto format=2 streams** (ECMA-182 CRC-64 digest from `spec/07` §3.2 + Stage-A `qm[0..7]` priming at every per-channel frame init per §3.5–§3.6; format=1 transparent alias via clear_priming) + r209 **`Decoder::decode_from_sample(sample_index)` + `frame_iter_from_sample(sample_index)` random-access player-API sugar** (eager + lazy `SampleSkipIter` suffix of `decode_all` from per-channel sample boundary; both reuse `seek_to_sample`'s spec/01 §4.1 arithmetic; cover format=1 + format=2; pre-existing libtta citation in `src/roundtrip_tests.rs:20-21` paraphrased per Rule E in same commit, resolves #1338; 101 lib tests) | ✅ ~96% — TTA1 fmt=1/2 + password; bit-exact self-roundtrip |
| **APE** (Monkey's Audio) | 🚧 r190 Phase 1 + r206 polish — 8-byte `MAC ` magic + decimal-coded version + 5 compression-level enum prefix parser + `Display` for `CompressionLevel`/`HeaderPrefix` (surfaces `version_raw` so unknown encoder values stay distinguishable) + 2040-input single-byte mutation harness asserting every result is `Ok` or a documented `Error` variant (18 unit + 8 integration + 1 doctest); per-version header tail + IIR coefficients + residual `k` recurrence + range-decoder bounds + channel decorrelation all DOCS-GAP | 🚧 scaffold |
| **Musepack** | 🚧 r197 — SV7 §2.5/§2.6 requantiser constants + SV7/SV8 stream-magic recognisers + SV8 packet outer-frame walker + r197 SV7 mpc_huffman tables + CNS PRNG + r201 SV7 §2.5 per-band sample-decode dispatcher (`BandDecodeCase` classifier covers all 18 spec cases; Cns=−1 / Empty=0 / HuffmanPerSample=3..=7 / PcmEscape=8..=17 live; Grouped1/2 + SV8 canonical-Huffman walk surface as DOCS-GAP via `Error::UnsupportedBandType(i8)` per #1323) + r206 **SV7 §2.6 reconstruction primitives** (`centre_pcm_level`/`centre_pcm_band` PCM-escape centring for band_types 8..=17, `dequantise_sample` covering CNS band -1 + normal 0..=17 via `centred * C / 65536`, `dequantise_band`/`dequantise_huffman_band`/`dequantise_cns_band` convenience wrappers, `pcm_escape_d` + `DEQUANT_DIVISOR=65536.0`; cross-module bit-reader→PCM-escape→centring→dequant integration test; 85 tests); lacks SV7 fixed-header field map + SV8 canonical-Huffman entropy layer + 32-band synthesis | 🚧 scaffold |
| **Cook** (RealMedia) | 🚧 r4 — flavor table + cookie parser + 8 DSP parameter tables + r194 open-time `DecodeConfig` (cookie ↔ flavor cross-check + sub-packet accounting) + r197 wire-level real-stream integration test + r203 cookie→flavor multi-match API (`iter_flavor_records` + `flavor_indices_matching_cookie(&CookCookie)` returns every record whose 4 cookie-checkable fields agree — cookie lacks `frame_bytes`/`sample_rate_hz`/`coupling_mode` so 21+22 both match on the real fixture; 41 tests); lacks bitstream decode | — |
| **WMA** | 🚧 r4 — patent-disclosed primitives (r197 mid-side stereo + run/level walker) + r203 **quantization-matrix differential coding + entropy-mode selector** (`qmatrix.rs` differential walker + `entropy_mode.rs` per-band entropy-coder choice; +818 LOC across 2 new modules); lacks codeword Huffman tables / exponent partition / LSP codebook / sign-bit layout / escape coding (`[GAP]` per docs) | — |
| **WavPack** | 🚧 ~85% (post-2026-05-18 orphan) — v4 block/metadata/decorrelation/entropy parse + LSB bit-reader + Golomb (base,add) interval + r186 `parse_block` aggregate + r191 `AdaptiveMedians` §3.2 + r194 **first PCM-producing API** `decode_packed_samples_mono` + r199 stereo per-sample decode loop + r201 `EntropyInfo→AdaptiveMedians` bridges + `from_entropy` wrappers + r206 **`WavPackBlock::decode_samples()` one-call PCM composer** chaining `parse_block` + `0x05` entropy expander + `0x0A` typed view finder + `decode_packed_samples_*_from_entropy`; mono/stereo dispatch via new `Flags::is_block_data_mono` (union of bit 2 `mono` and bit 30 `false_stereo`); returns `Vec<i32>` of `block_samples` mono OR `block_samples*2` interleaved stereo; new `UnsupportedBlockFeature` enum (Hybrid/FloatData/Int32Mode/MultichannelMember/Decorrelation/LowLatencyBlock/RobustBlock) + 3 structural errors; 295 tests; lacks hybrid 0x0B+0x0C / float / multichannel / CRC / decorrelation prediction-loop consumer / encoder | 🚧 scaffold |
| **APE** (Monkey's Audio) | 🚧 r190 Phase 1 + r206 polish — 8-byte `MAC ` magic + decimal-coded version u16 + 5 compression-level enum (1000/2000/3000/4000/5000) prefix parser + `Display` impls (label + raw u16) + 2040-input mutation harness asserting only documented `Error` variants leak from `parse()`; 18 unit + 8 integration tests + standalone-build OK; per-version header tail (sound params/frame count/seek table/embedded WAV) + IIR coefficients + residual `k` recurrence + range-decoder bounds + channel decorrelation reconstruction all DOCS-GAP | 🚧 scaffold |
| **DTS** (Core) | 🚧 ~40% — frame-sync header + 14↔16-bit pack/unpack + r192 `iter_frames_14bit` + r195 §5.4.1 ABITS/SCALES side-info + Annex D §D.5.6 12-level BHUFF + §D.5.3/§D.5.4 small-Huffman + §D.1.1 RMS_6BIT + §D.1.2 RMS_7BIT + r202 §5.3 SFREQ/AMODE/PCMR typed resolvers + r208 **§C.2.5 `PreCalCosMod()` 544-entry `raCosMod` cosine-modulation matrix** (4-block layout: Block 1 `cos((2i+1)(2k+1)π/64)` 16×16, Block 2 `cos(i(2k+1)π/32)` 16×16, Block 3 `+0.25/(2·cos((2k+1)π/128))` 16, Block 4 `−0.25/(2·sin((2k+1)π/128))` 16; per-block start constants `COS_MOD_BLOCK{1..4}_START` = 0/256/512/528; 232 lib + 217 standalone tests); lacks subframe walker + §5.4 polyphase synthesis (blocked on §D.8 raCoeff* taps #1357) + DIALNORM | — |
| **aptX** (classic + HD) | 🚧 ~70% — 4-band QMF + ADPCM; bit-exact NDA-blocked + r189 RFC 2361 §A.24 `WAVE_FORMAT_TAG_APTX = 0x0025` IANA tag + `CODEC_ID_STR = "aptx"` registry (lets RIFF containers route 0x0025 → clean NotImplemented) | — |

</details>

<details>
<summary><strong>Video</strong> (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **MJPEG** | ✅ ~97% — baseline + progressive 4:2:0/4:2:2/4:4:4/grey + 12-bit YUV (baseline + r183 SOF2 P=12 progressive) + SOF9 arithmetic + lossless SOF3 + RFC 2435 RTP/JPEG depacketization + r190 §G.1.1 SOF2 4-component CMYK / YCCK progressive at P=8 + r197 **8th cargo-fuzz target `arith_decode`** wraps fuzz bytes in SOF9 envelope to drive `ArithDecoder` Q-coder per-iteration coverage (`Initdec`/`Renorm_d`/`Byte_in`/`decode_dc_diff`/`decode_ac`/`decode_magnitude` + DRI restart + per-component stats) | ✅ ~96% — baseline + progressive + lossless SOF3 grey/RGB + DRI/RSTn + non-zero point transform Pt 0..15 + r193 public 4-component CMYK encoder |
| **FFV1** | 🚧 ~78% — RFC 9043 decoder + demux + decode_frame driver (YCbCr + RGB Y/Cb bit-exact; RGB Cr divergence open) + r179 `coder_type==2` alternative state-transition table wired through decode + encode + r208 **Golomb-Rice chroma-planes decode_frame cursor fix** (latent bug: `coder_type==0` was constructing fresh `BitReader` *inside* per-Plane loop, causing Plane 1/2/3 to silently re-read Plane 0's bytes from offset zero on `chroma_planes==true` YCbCr Slices; dormant because every prior round-trip targeted single-Plane grayscale OR RGB/line-major driver; fix: one shared `BitReader` outside loop on `coder_type==0` arm; +14 round-trip tests covering `(coder_type ∈ {0,1,2}) × (4:4:4/4:2:2/4:2:0) × (extra_plane ∈ {true, false})`; 386 tests) | 🚧 ~96% — Slice Footer + Slice Header + Golomb-Rice primitives + frame-level Golomb-Rice + YCbCr encoder + r164 range-coded SliceContent encoder + r190/r193 §4.7 RGB + RCT encode + r196 unified `encode_frame` dispatch helper + r202 §4.2 Parameters + §4.1 Quantization Table Set cascade encoder; lacks §4.2.14 tail (#904) |
| **MPEG-1 video** | 🚧 ~42% — sequence/GOP/picture/slice + macroblock walk + intra-DC + §2.4.3.7 dct_coeff walker + §2.4.4 dequantiser + r185 §A 8×8 IDCT + IEEE P1180/D2 conformance + r194 §7.3 `mpeg2_inverse_scan` + r202 §6.2.6 MPEG-2 `block(i)` driver (`mpeg2_decode_block` chains §7.2.1 DC prelude → §7.2.2 residual VLC walker w/ FIRST/NEXT alternation → §7.3 inverse scan → §7.4 inverse-quant → §A 8×8 IDCT into one bitstream→`f[y][x]` entry; 677 tests) | 🚧 scaffold |
| **MPEG-2 video** | 🚧 ~65% — §6.2.x sequence/GOP/picture/slice + macroblock walk + §7.6.3.x PMV + §7.6.4-8 forming-predictions/combine/add-saturate + r165 §7.6 driver + r179 §7.4 inverse-quantisation + r185 §A 8×8 IDCT + r192 §7.2.2 residual VLC walker + r194 §7.3 `mpeg2_inverse_scan` + r199 §7.2.1 **intra-block DC prelude** + r206 **§6.2.5/§6.2.6 macroblock-block driver** (`mpeg2_macroblock_blocks::decode_macroblock_blocks` walks `pattern_code[12]` and dispatches §6.2.6 `block(i)` per coded slot; auto-derives §6.1.1.8 block-index → component (Figures 6-10/6-11/6-12), Table 7-5 weighting-matrix `w` per `(coding, component, chroma_format)`, honours §7.2.1 non-intra DC-predictor reset; +960 LOC driver + 15 unit + 6 integration; 698 tests); next: slice-layer driver looping over MBs | 🚧 scaffold |
| **MPEG-4 Part 2** | 🚧 ~64% — I-VOP intra + inter texture + §6.2.5 video_packet_header + §7.8.7.3 GMC + r182 §7.6.2.1 half-sample bilinear + r190 §7.6.2.2 quarter-sample + Table 7-13 chroma MV reduction + r193 §7.6.9.5.2 B-VOP direct-mode MV derivation + r195 §7.6.9.5.3 B-VOP luminance prediction-block + r201 §7.6.5 chroma MV derivation `MVDCHR` + r206 **§7.6.1.6 vector padding** (`pad_macroblock_vectors` with `AllZero` (INTRA / P-VOP skipped) and `PerBlock` modes; walks verbatim `FALLBACK_CHAIN = [[1,2,3],[0,3,2],[3,0,1],[2,1,0]]` precedence against pre-padding snapshot so spec's `?:` RHS sees originals; `VectorPaddingError::AllTransparent` for fully-transparent MBs; feeds §7.6.5 luma→chroma + spatial-predictor gather + §7.6.9.5 B-VOP direct-mode co-located lookup; 553 lib + 8 doc tests); lacks B-VOP chroma MC plane + §6.2.6.2 MV-body parser wiring + encoder | 🚧 scaffold |
| **Theora** | 🚧 ~48% — §6.1–§6.4 setup-header + Appendix B.2/B.3 VP3-default tables + §6.4.x quant + DCT-token Huffman + §7.1–§7.5 frame walk + r160 §7.5 motion vectors + r179 §7.7.1 EOB Token decode + r185 §6.4.1 LFLIMS + r191 §7.7.2 Coefficient Token Decode + r195 §7.7.3 DCT Coefficient Decode driver + r201 §7.8.1 DC predictor compute + r206 **§7.8.2 Inverting DC Prediction driver** (`invert_dc_prediction` walks Y/Cb/Cr planes in raster order, resets `LASTDC[0..=2]` at plane boundaries, recomputes §7.8.1 predictor per coded block, adds residual to `COEFFS[bi][0]`, truncates via i32→i16→i32 narrowing, seeds `LASTDC[rfi]` per ref frame; 4 new error variants; 320 tests); §7.1–§7.8 frame decode now complete; lacks §7.9 reconstruction + §7.10 loop filter | 🚧 scaffold |
| **H.263** | 🚧 ~89% (post-2026-05-18 orphan) — §5.1-§5.4 baseline + §6 IDCT/MV/half-pel/INTER + Annex J §J.3 deblock + Annex I AIC + Annex D UMV + Annex F 4-MV + OBMC + §5.1.4 PLUSPTYPE + Annex K §K.2 SS + r187/r192 §I.3 AIC reconstruction pipeline + r196 §I.2/§I.3 AIC MB-grid driver wiring + r202 `decode_picture_layer` PLUSPTYPE entry-point + r208 **§5.1.4.4/§5.1.4.5 PLUSPTYPE inherited-state stream driver** (`decode_picture_layer_with_inherited(data, ref, options, inherited) -> DecodePictureOutcome { frame, inherited }` retains §5.1.4.4 OPPTYPE snapshot across pictures so a UFEP=000 PLUSPTYPE header is decodable; `InheritedExtendedState` grown to full snapshot `source_format/umv/advanced_prediction/advanced_intra/deblocking`; `from_opptype` captures from parsed `Opptype` (refused-mode bits dropped); §5.1.4.5 rule 1 UMV/AP forced off in I-pictures after inheritance; rule 3 baseline-PTYPE resets snapshot; legacy `decode_picture_layer` is thin wrapper; 385 tests); lacks Annex K driver + PB-frames + custom-format | 🚧 scaffold |
| **H.261** | ✅ ~98% — I+P QCIF/CIF + integer-pel + loop filter + BCH FEC + Annex B HRD + RFC 4587 RTP + RFC 3550 RTCP SR/RR/SDES/BYE/APP; r189 §6.2.1 SDP offer/answer negotiation + r198 3rd cargo-fuzz target `decode_bch_multiframe` + r204 **4th target `parse_rtp_payload`** (RTP §5.1 fixed header + RFC 4587 §4.1 H.261 payload header SBIT/EBIT/I/V/GOBN/MBAP/QUANT/HMVD/VMVD + multi-packet `depacketize` bit-walker; 9-buffer seed corpus + stable-CI mirror; distinct from `decode_h261`/`parse_rtcp_compound`/`decode_bch_multiframe`) | ✅ ~98% — spiral+diamond ME + GQUANT-from-bitrate + BCH framing + RTP wrap + RTCP compound build/parse; 45 dB at 64 kbit/s QCIF |
| **MS-MPEG-4** (v1/v2/v3) | 🚧 ~44% — clean-room scaffold + r202 `Macroblock4MvDecoder` 4-MV-per-MB bitstream tests (4 integration tests pin picture-corner rule-4 + within-MB candidate chaining + four-zero-MVD rigid-motion + parallel-reader cross-check against `predict_block_mv`; 80 integration tests) + r181 `GFamily` accessors + r185 Figure 7-34 MV-predictor walk + r191 1-MV predictor routed through `predict_block_mv(Block::TopLeft, …)` + r196 §7.6.5 4-MV-per-MB batch predictor + r208 **4-MV neighbour-MB bordering-cell picker** (`bordering_block_of_neighbour` + `pick_neighbour_mv_from_4mv` const-fns + `NeighbourDirection` enum close long-standing "caller picks right cell from neighbouring MB" gap in `MacroblockCandidates`; (current-block, direction) → bordering-block table from Figure 7-34's 4 sub-diagrams: TL takes all 3, TR takes Above+AboveRight, BL takes Left, BR takes nothing; 309 tests). Still lacks G0..G3 primary canonical-Huffman bit-length array + alt-MV VLC + 4-MV MCBPC. VfW-sandboxed mpg4c32.dll runs in parallel | — |
| **H.264** | 🚧 ~80% — I/P/B + 4:2:0/4:2:2/4:4:4 + CAVLC + CABAC + DPB + 44 SEI types + fuzz-hardened + r183 SEI type 46 + r187 §8.2.1 POC i64-staged + r192 §5.2.4.1.1 strict avcC parser + High-family extension trailer + r194 §7.3.5.3.1 CAVLC call-contract guards + r200 Annex G MVC SEI types 39+43 + r207 **Annex G MVC SEI type 41 `non_required_view_component`** (§G.13.1.6/§G.13.2.6: `parse_non_required_view_component` + range-bound pre-allocation caps `num_info_entries_minus1 ≤ 1022`, `view_order_index[i] ∈ 1..=1023`, per-entry `num_non_required_view_components_minus1 ≤ view_order_index − 1`, `index_delta_minus1 ≤ view_order_index − 1`; 1076 lib + 2948 integration); lacks MBAFF, SVC/3D/MVC body | 🚧 ~83% — I+P (1MV/4MV, ¼-pel) + B + CABAC at all chroma layouts + Trellis-quant RDOQ-lite (1227 tests); ffmpeg PSNR_Y 44.20 dB |
| **H.265 (HEVC)** | 🚧 ~54% — VPS+SPS+PPS bodies + scaling-list + scan + §9.3 CABAC engine + slice header through §7.3.6.3 pred_weight_table + r182 §7.3.6.2 ref_pic_lists_modification() + r190 §7.4.8 inter-RPS-prediction typed builder + r193 §7.3.2.3.1 `PpsExtensionFlags` + r195 §9.3.4.2 binarization scaffold + r200 §9.3.4.2.4 `coded_sub_block_flag` ctxInc + §9.3.4.2.2 Table 9-49 `split_cu_flag`/`cu_skip_flag` ctxInc + r207 **six §9.3.4.2/Table 9-48 closed-form ctxInc** (`split_transform_flag_ctx_inc(log2)`→`5−log2TrafoSize` bank `{0..=3}`; `cbf_luma_ctx_inc(d)`→`d==0?1:0`; shared `cbf_chroma_ctx_inc` for Cb/Cr; `inter_pred_idc_ctx_inc` bin-0 routes CtDepth + escapes to 4 on 8×4/4×8 (`nPbW+nPbH==12`); `log2_res_scale_abs_plus1_ctx_inc(bin,c)`→`4*c+bin`; `res_scale_sign_flag_ctx_inc(c)`→`c`; 265 tests); lacks `sig_coeff_flag`, `coeff_abs_level_g{1,2}` + residual/IDCT | 🚧 scaffold |
| **H.266 (VVC)** | 🚧 ~70% — 4:2:0 IDR intra + ALF/SAO/CC-ALF + P/B merge+skip + HMVP + MMVD + CIIP + BCW + BDOF + GPM + AMVR + HBD + DMVR + affine + PROF + AMVP + SbTMVP + r181 VPS + r193 §7.3.10.10 `amvr_flag`/`amvr_precision_idx` CABAC reader; 1106 lib tests | 🚧 ~90% — forward CABAC + DCT-II + SAO/ALF/cu_qp_delta + MTT BT+TT RDO + P+B + sub-pel MC + multi-ref DPB + weighted bi-pred + r190 §7.3.11.7 `encode_non_merge_inter_pre_residual` + r195 encoder-side §7.3.10.10 `amvr_enc` + r201 §7.3.10.5 `bcw_idx_enc` encoder mirror + r207 **§7.3.10.5 multi-CP-MV affine MVD encoder dispatcher** (`encode_non_merge_inter_pre_residual_affine` + `NonMergeInterPreResidualAffineDecision` generalises r190 translational dispatcher to emit `numCpMv` `mvd_coding()` calls per active list in spec order: `mvd_coding(LX,0)` → if MotionModelIdc>0: `mvd_coding(LX,1)` → if MotionModelIdc>1: `mvd_coding(LX,2)`; clamps unused CP + inactive-list per-CP MVDs to zero; §7.3.11.7 affine-excludes-SMVD precondition debug-asserted; §8.5.2.5 SMVD shortcut applies only `cpIdx==0`; 1134 lib tests; translational-degenerate bit-identical to r190) |
| **VP6** | 🚧 r17 — §13 static tables + §3 RawBitReader + §7.3 BoolCoder + r198 §13.2.1 DC arithmetic + r204 §13.3.1 **AC coefficient arithmetic decoder** (`decode_ac_token` Figure 15 walk with EOB-branch + "implicitly-1" first-decision shortcut gated on `prec == WasZero && encoded_coeffs > 1`; `decode_ac_coefficient` wrapper returning `AcOutcome::{EndOfBlock, ZeroRun, Value}`; `AcBand` (Table 30) + `AcPlane` (Table 28) + `AcPrecContext` (Table 29) with `seed_from_dc(dc: i32)` per §13.3.1 first-AC seeding; reuses r16 `decode_token_value` magnitude/sign kernel; 363 tests; +27); unblocks §13.3.3.1 zero-run-length + §13.3 per-frame `AcProbs` update | 🚧 scaffold |
| **VP8** | ✅ 100% | ✅ 100% |
| **VP9** | 🚧 ~44% — §6.2 walk + §9.2 Bool decoder + §6.3 compressed-header primitives chain complete + §6.4.24 coeff + §8.6 dequant + §8.7 inverse transforms + §8.5.1 intra pred + r199 §6.3.12 `frame_reference_mode` + r205 §6.3.16 **`mv_probs()` compressed-header outer sweep** (65/69-cell walk over 9 `mv_*_prob[]` arrays in three unconditional phases + conditional HP tail gated on `allow_high_precision_mv`; threads §6.3.17 `update_mv_prob` per-cell primitive; new `MvProbs` aggregate + `defaults()` ctor; 9 §10.5 default tables + 5 §3 MV constants verbatim-transcribed; 415 lib tests; §6.3.1→§6.3.18 primitives chain complete); lacks §6.2.5 inter-frame branch of uncompressed-header walker + §6.4.4 decode_block + §8.4 loop filter | 🚧 scaffold |
| **AV1** | 🚧 ~94% — decoder feature-complete + **standalone `decode_av1` public entry** + r203 §6.7.2 Y-only (monochrome) on the dyn pixel driver + r207 multi-SB dyn-Y dispatch up to 128×128 (1528 tests + integration roundtrips) | 🚧 ~32% encoder — pixel-space YUV→IVF driver + 14-mode intra picker + §7.13.3 forward 2D dispatcher + WHT lossless + forward quantize + r194 §7.11.5.3 UV_CFL_PRED + r196 `base_q_idx > 0` lossy quant + r197 rectangular extents + r203 monochrome encoder dyn driver + r207 **multi-super-block tiling on monochrome dyn driver** (lifts Y-only extent ceiling 64→128 via §5.11.1 `for r/c += sbSize4` walk with `sbSize=BLOCK_64X64`; each SB origin a fresh `BLOCK_64X64`-rooted `EncodeNode` tree; edge SBs swallow OOB quadrants via r234 `EncodeNode::dummy_oob` + §5.11.4 line-1 short-circuit; new `MonoYFrameMultiSb` + `EncodedFrameDynYMultiSb` + `encode_intra_frame_y_dyn_multi_sb{,_with_q}` + `MAX_DIM_Y_MULTI_SB=128`; bit-exact across 10 extents incl. partial-coverage edges 72×64/104×72 + 2×2 grids up to 128×128 + lossy q∈{1,32,200}). Lacks rectangular **TX_SIZE family** (TX_4X8/8X4/8X16/16X8) + §5.11.18 inter mode_info + RD picker + 4:2:0 YUV multi-SB |
| **Dirac / VC-2** | ✅ ~95% — VC-2 LD+HQ intra + Dirac core-syntax intra/inter + OBMC + 7 wavelets + 10/12-bit + bit-exact intra fixtures + r165 fuzz oracle + r190 Criterion bench harness + r195 `vh_synth`/`vh_analysis` row-major slice driving + r201 §12.4.4 `extended_transform_parameters` parser (VC-2 v3 streams that reduce to the §12.4.4 NOTE symmetric default decode cleanly; genuinely asymmetric v3 streams surface typed `AsymmetricTransformUnsupported`; 345 tests) | 🚧 ~96% — HQ+LD intra + Dirac core-syntax + adaptive sub-pel + 2-ref bipred + post-OBMC refinement + picture/sequence rate-control + r179 intra-encoder fuzz oracle + r193 inter-encoder fuzz oracle + r206 **§12.4.4 VC-2 v3 encoder symmetric-default sequence-header roundtrip** (`with_major_version_3` on `EncoderParams`/`LdEncoderParams` emits two §12.4.4 `read_bool()` flag bits `asym_transform_index_flag` then `asym_transform_flag` at symmetric default after `dwt_depth`; v3 streams decode pixel-identical to v2 per §12.4.4 NOTE; asymmetric emission intentionally not exposed since decoder rejects; HQ bit-exact + LD ≥35 dB PSNR; both assert byte-distinct AND pixel-identical reconstructions) |
| **AMV video** | 🚧 scaffold (orphan rebuild post-audit 2026-05-18; clean-room re-implementation pending) | 🚧 scaffold (orphan rebuild post-audit 2026-05-18; clean-room re-implementation pending) |
| **ProRes** | ✅ ~96% — RDD 36 entropy + 8/10/12-bit + 4:4:4:4 alpha + interlaced + RAW refused; ffmpeg interop 60-68 dB + cargo-fuzz + r185 `idct8x8_dc_only` fast path + r195+r201 SHA-256 lockstep pin on 9 fixtures (2 interlaced 1920×1080 + 7 progressive across all 6 profiles) + r206 SHA-256 lockstep on small in-tree 128×128 interlaced apcn (3 SHAs frame 0 / frame 1 / concatenated `f0‖f1`; per-frame `assert_ne!` catches §5.1-walker-re-decodes-frame-0-twice that whole-frame PSNR alone cannot detect; FIPS 180-4 §B.1/§B.2 self-check; 263 tests) | ✅ ~97% — RDD 36 all 6 profiles + interlaced + alpha + perceptual quant matrices + r193 ffmpeg cross-decode acceptance §5.3 MBs/slice knob (58.85-63.93 dB luma PSNR) |
| **EVC** (MPEG-5) | 🚧 ~90% — NAL + SPS/PPS/APS + §9.3 CABAC + §8 intra + DCT-II + P/B inter + RPL + HMVP + DPB + ALF + DRA + IBC §8.6 + r187 §8.9.7 `DraChromaDerived` + r193 §8.9.8 `DraJoinedScaleFlag=1` joined-chroma-scale + r195 §7.4.3.1 SPS-signalled `ChromaQpTable` + r201 `chroma_qp_table_for_sps` three-way SPS adapter + r207 **`derive_dra_chroma_state_for_sps(syntax, derived, cidx, sps) → DraChromaDerived`** SPS→§8.9.6 chroma-chain adapter dispatching §8.9.7 unjoined (`joined_scale_flag==false` → `derive_dra_chroma_state`) vs §8.9.8 joined (`joined_scale_flag==true` → `derive_dra_chroma_state_joined` with `chroma_qp_table_for_sps`); 437 tests; lacks Main-profile toolset (BTT/ADMVP/EIPD/ATS/AMVR/affine) + #1278 §8.9.8 eq 1398-1409 tableNum==0 branch ambiguity | — |
| **HuffYUV** / FFVHuff | ✅ ~97% — HFYU + FFVH FourCCs + 6 predictors + 8-bit only + interlaced field-stride=2 + fast-LUT decoder + SWAR 8-byte gradient post-pass + r181 YUY2 LEFT macropixel-step branch-free decoder + r196 cargo-fuzz `encode_roundtrip` target + r202 YUY2 Median tail-loop dead-branch strip + r208 **LEFT-helper dedup vs `predict::*`** (decoder-local `inverse_left_per_channel` was byte-for-byte duplicate of `predict::inverse_left_row`; `inverse_yuy2_left{,_range}` were thin pass-throughs into `predict::inverse_yuy2_left_macropixel` since r181 macropixel-step rewrite; YUY2 + RGB24 + RGB32 decode paths now re-pointed at predict-side helpers; decoder behaviour byte-identical; 164 tests) | ✅ ~96% — full encoder symmetry × YUY2/RGB24/RGB32 + v1.x + v2.x ClassicV2/CustomV2 + r181 YUY2 LEFT forward + r186 `forward_rgb_left_subtract_linear` + r202 encoder-side dead-branch parity |
| **Lagarith** | ✅ ~95% — all 11 wire types + modern range coder + legacy adaptive-CDF + Fibonacci-Zeckendorf prefix + JPEG-LS Median + G-pivot decorr + zero-run RLE + pair-packed 513-entry CDF + modern RGB(A) first-column predictor Rule B + r198 deeper channel-body fuzz (bit-XOR + multi-byte burst + shift sweeps layered on r192's truncation + single-byte-flip; closes the §6.1 channel-body parser surface) | 🚧 ~76% — encoder for SOLID/RGB/RGBA/YV12/YUY2/legacy-RGB + spec/02 §5 Step-A + Step-B + Step-C `freqs[]` cache + r135/r138/r141 modern + per-channel header-form selection; byte-exact vs proprietary encoder Auditor-blocked |
| **Ut Video** | ✅ ~97% — 5 native FourCCs × 4 predictors + RGB inter-plane decorrelation + LUT-accelerated canonical Huffman + slice-parallel decode (5.63× at 720p) + criterion baseline + r186 `Decoder` trait factory + r196 Gradient/Median per-row branch-hoist + r203 **row-strided None + Left predictor refactor** (single shared stride-aware row driver replaces two near-duplicate per-predictor inner loops; tests/round16_predictor_row_stride.rs covers contiguous + odd-stride + tail-partial-row equivalence vs r186 baseline; +468 test LOC; observable byte-identical) | ✅ ~96% — slice-parallel encode (3.28×) + content-fixture corpus + r161 cargo-fuzz oracle |
| **MagicYUV** | ✅ 100% | ✅ 100% — r206 `examples/profile_magicyuv.rs` samply-friendly flat profile driver (5 modes × 5 `quick_bench` archetypes; single Instant-pair per scenario isolates Huffman batch decode + modular/JPEG-LS Median + RGB decorrelation + Package-Merge + BitWriter drain hot paths) |
| **Cinepak** (CVID) | ✅ ~98% — frame header + multi-strip + V1/V4 codebooks + intra/inter + grayscale + Sega FILM demuxer + Saturn/3DO deviant + r181 codebook_chunk_apply + r192 `decode_vector_chunk` cargo-fuzz target + criterion benches + r196 `decode_multi_frame` cargo-fuzz target + r202 named seed-corpora for `codebook_chunk_apply` / `decode_vector_chunk` / `decode_deviant_frame` (27 deterministic seeds via `examples/seed_fuzz_corpora.rs` + in-memory verification test through public entry points) | ✅ ~98% — stateful encoder + rolling codebooks + RDO + LBG + 3-axis grid picker + bitrate-target rate-control + keyframe-interval (34.18 dB PSNR; decode 4.4 GiB/s, stateful GOP 13.5 ms/frame) |
| **SVQ1/SVQ3** (Sorenson) | 🚧 r11 — SVQ1 framework + r194 L=0..L=3 codebook payload + r197 L=4/L=5 ABSENCE + r203 **SVQ1 saturating-clip + bit-mask helper LUTs** (`build.rs` extension stages `clip_lut.csv` 769-row table + `MANIFEST-02.sha256` integrity; `svq1_helper_luts.rs` typed-LUT accessors for `saturating_clip` + `mask_bits`; +237 LOC LUT module + +175 LOC build extension; `tables/clip_lut.meta` binary-disassembly-tier provenance YAML only); lacks intra-vs-inter ordering + stage interleave + SVQ3 MV-VLC + #1256 svq3.c attribution scrub | — |
| **Indeo 3** (IV31/IV32) | 🚧 r14 — clean-room codec-frame header + bitstream header + spec/02 picture-layer + spec/03 macroblock-layer + spec/04 VQ codebook + spec/06 byte-level entropy + spec/07 output-reconstruction + four cell-shape kernels + spec/02 strip-context array + spec/03 per-cell sub-array wiring + r181 spec/05 §1 mc_table + r186 spec/05 §2.2/§2.3/§3.3/§3.4 packed-MV bit-layout + r196 spec/05 §5.4 cell-position decoding + r202 spec/05 §4.2 ping-pong bank-selection + r208 **spec/05 §4.1 strip pixel-buffer arena geometry typing** (`MC_ARENA_LEN = 0x8020` + `MC_ARENA_ROW_STRIDE = 0xb0` cross-check; `STRIP_PIXEL_BUFFER_ALIAS_COUNT = 6` + `StripPixelBufferAlias::{Base0..Base5}` enum + `from_index`/`as_index`/`slot_relative_byte_offset`; `strip_region_bytes(plane_height_pixels)`; `StripArenaCapacity::for_plane_height` (boundary `186`; §4.1 example height 240 flagged not-fitting); `base_pointer_aliases_equal` invariant probe; 341 tests); lacks §7.2 boundary fix-up + §7.3 reverse decomposition + MC inner loop | — |
| **Indeo 2/4/5** | 🚧 scaffold — pending clean-room workspace; Indeo 4/5 still sandboxed via `oxideav-vfw` | — |

</details>

<details>
<summary><strong>Image</strong> (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **PNG / APNG** | ✅ 100% — 5 colour types × 8/16-bit + APNG + sBIT/pHYs/tIME/bKGD/hIST/eXIf/sRGB/cICP/sPLT + r154 Criterion benches + r183 tRNS keyed transparency promotion + r196 APNG frame-scan Criterion bench + r208 **iCCP + iTXt round-trip** (`metadata::Iccp { name, profile: Vec<u8> }` W3C PNG3 §11.3.2.3 opaque zlib-compressed; single-instance; rejected on duplicate; emitted before PLTE/IDAT in §4.3 rank 2 between cICP and sRGB; `metadata::Itxt { keyword, compressed, language_tag, translated_keyword, text }` PNG3 §11.3.3.4 UTF-8 successor to tEXt with BCP47 language tag, optional zlib body, no-NUL-in-translated-keyword+text rule; multi-instance; emitted after zTXt; 264 tests; closes the metadata-chunk gap list — only HDR `mDCV`/`cLLI` remain) | ✅ 100% |
| **GIF** | ✅ 100% — 87a/89a + LZW + interlaced + animation + disposal compositor + structured Application Extensions + Plain Text Extension + lenient mode + lazy Playback + animation-timing accessors + fluent AnimationBuilder; clean-room from CompuServe spec + r153 tracked spec-derived fuzz seed corpus (5 seeds × 3 targets) | ✅ 100% — per-frame palettes + `optimize_color_tables()` GCT/LCT hoisting + §7 Required Version enforcement + `upgrade_version_if_needed()` |
| **WebP** (VP8 + VP8L) | ✅ 100% | ✅ 100% |
| **JPEG** (still) | ✅ ~95% — via MJPEG | ✅ ~90% — via MJPEG |
| **TIFF** (6.0) | ✅ ~98% — II/MM + BigTIFF read + 7 photometrics (incl. PI=4 Transparency Mask r172) + 1/4/8/16-bit + None/PackBits/LZW/Deflate/CCITT-MH/T.4-1D + FillOrder + tiles + multi-page + JPEG-in-TIFF (incl. CMYK-JPEG: Compression=7 + Photometric=5 + SamplesPerPixel=4) + PlanarConfiguration=2 (separate component planes across strips/tiles + chunky re-interleave + Predictor=2 driven per-plane) + cargo-fuzz decoder (panic-free, 7.7 M iter green) | ✅ Gray8/16/RGB24/Palette8 — None/PackBits/LZW/Deflate + Predictor=2 + PlanarConfiguration=2 separate-planes write (Rgb24 × None/PackBits/LZW/Deflate ± Predictor=2) + Bilevel CCITT-MH / T.4-1D, single+multi-page + tiled chunky write (Gray8/16/RGB24/Palette8 × None/PackBits/LZW/Deflate ± Predictor=2, §15) + tiled PlanarConfiguration=2 write (Rgb24, one grid per plane, §15) |
| **BMP** | ✅ ~97% — 1/4/8/16/24/32-bit + V3/V4/V5 + OS/2 + RLE4/RLE8 + BI_ALPHABITFIELDS + r205 V4/V5 colour-space + embedded ICC profile metadata + 3 fuzz targets + 31-test property sweep | ✅ ~97% — top-down + `biClrUsed`-trimmed palette + r205 `encode_bmp_with_icc_profile` (V5 + `PROFILE_EMBEDDED`) + r210 `encode_bmp_with_linked_icc_profile` (V5 + `PROFILE_LINKED`) for Rgba/Rgb24 |
| **Netpbm** (PBM/PGM/PPM/PNM/PAM + PFM) | ✅ ~96% — all 8 magics at 1/8/16-bit + 6 PAM TUPLTYPEs + PFM (`Pf`/`PF`) IEEE-754; r210 P5/P6/P7 16-bit `read_be16_row`/`write_be16_row` autovectorizable helpers (same shape as r205 PFM 32-bit helper; encode 16-bit ≈ -2-4 %) | ✅ ~96% |
| **ICO / CUR / ANI** | ✅ ~98% — multi-res + BMP/PNG sub-images + CUR hotspot + ICONDIRENTRY validation + 256×256 PNG round-trip + r198 standalone `read_ani_raw` RIFF/ACON parser + r198 `biBitCount` reject + r204 ANI `seq[]` step-index bounds-check + r210 **directory-vs-body sub-image dim mismatch reject** (non-zero `bWidth`/`bHeight` byte must agree with PNG IHDR / BMP `biWidth`/halved-`biHeight`; canonical `bWidth=0 → 256` carve-out preserved; closes the dim-side probe-vs-render hole left after r178 / r184 / r198 / r204 closed the range / hotspot / biBitCount / seq[] sides; 78 lib tests) | ✅ ~92% |
| **JPEG 2000** | 🚧 r15 (post-2026-05-20 orphan) — T.800 main-header + SOT/SOD + typed COC/QCC/POC/RGN/PLT/PPT + JP2 box + §B.10 tier-2 + §B.5 ResolutionLevel + §B.6 precinct + §B.7 code-block partition + Annex C §C.3 tier-1 MQ + Annex D 19 contexts + §B.12.1 5 packet-progression iterators + §B.12.2 POC + r181 Annex F.3 inverse DWT + r187 4 cargo-fuzz targets + r192 Annex E code-block→sub-band reassembly + r195 Annex G MCT primitives + r201 §G.1 DC level-shift + r208 **§F.3.1 IDWT cascade `idwt_5x3` + `idwt_9x7`** (initialises at `levels[0]` NLLL band; iterates `k = 1..=NL` folding each level's `[HL, LH, HH]` triple through `dwt::sr_2d_*` with origin `(levels[k].trx0, levels[k].try0)`; carries LL forward; returns full tile-component resolution `Interleaved2D<i32>`/`<f64>`; handles `NL = 0` per §F.3.1 "`a0LL` is output `I(x, y)`"; 388 tests); lacks §B.12 progression-walker → BlockSource adapter wiring + HTJ2K Part-15 | 🚧 scaffold |
| **JPEG XL** | 🚧 ~92% — ISO/IEC 18181-1:2024 lossless Modular path + 7 fixtures pixel-correct + VarDCT scaffold + Gaborish/EPF/AFV pure-math complete + §C.8.3 per-block HF coefficient loop + r190 `PerPassNonZerosGrids` per-pass container + r191 WP trace oracle isolating #799 divergence + r195 WP state-evolution backward bisect + r202 row-3 chain widening + r208 **§C.5.4 + §C.8.3 per-LfGroup varblock-walk driver** (`varblock_walk` module: `Varblock { x, y, transform, hf_mul }` + borrow-based `VarblockWalk` raster-order iterator over `DctSelectGrid` skipping Continuation cells; `count_varblocks`; typed per-pass per-channel `decode_varblocks_for_pass_channel` invoking `block_ctx_for_varblock` closure + threading each varblock through `PerPassNonZerosGrids::decode_block_at_for_pass_channel`; closes "per-LfGroup varblock-shape grid" gap r177/r183/r190 module notes repeatedly deferred — bridges r13 DctSelect placement with r190 per-pass per-channel NonZeros routing; 650 tests); lacks upstream WP fix (#799) + §C.7.2 histograms | — retired |
| **JPEG XS** | 🚧 ~82% — ISO/IEC 21122 Part-1 + 5/3 DWT + Annex C/D/F/G + multi-component + CAP-bit + high bit depth + r190 4:2:0 chroma at NL,y≥3 | 🚧 ~90% — Nc 1/3/4 + Sd>0 + RCT + Star-Tetrix + NL up to 8 + odd dims + vertical prediction + per-band Q + NLT + high-bit-depth Star-Tetrix lossless+lossy + r206 **per-slice `Q[p]` override** (Annex C.2 Table C.1; `encode_planar_hsl_qslice` + `EncodeConfig.q_slices: Vec<u8>`; `slice_cfg_for` clones cfg with `q = q_slices[t]` per slice, falls back to picture-level `cfg.q` when override absent so legacy output is byte-identical; `Fq` auto-selects 0 if all entries 0 else 8 per Table A.8; wire-trace verifies per-slice Q via codestream parse; 335 tests) |
| **AVIF** | 🚧 ~89% — HEIF→AV1 + grid + imir/clap/colr/pixi/pasp + HDR metadata + AV1 wrap + DoS caps + HEIF item-properties + auxC URN + rloc/lsel/iovl/grpl + `mif1` + r130 tmap §4.2.2 + r188 ISO 21496-1 Annex C.2 `GainMapMetadata` + r193 §5.2.5.3+§5.2.7 value-comparison shalls + r195 §8.2/§8.3 AVIF still-image profile-compliance audit + r201 av1-avif v1.2.0 §3 AVIS shall-level audit + r206 **§8.2/§8.3 AVIS sequence-track profile audit** (`audit_avis_profile_compliance(&AvisMeta, &BrandClass) -> Vec<AvisProfileCompliance>` — sequence-track companion to r195 still-image audit; decodes `AV1CodecConfigurationRecord` byte 1 `seq_profile (3) | seq_level_idx_0 (5)` per av1-isobmff §2.3; bounds match still-image — Baseline ≤ Main + level 5.1, Advanced ≤ High + level 6.0; `avis-` diagnostic prefix; pinned on Netflix `alpha_video.avif`; 281 + 59 tests) | — |
| **DDS** | ✅ ~99% — DDS_HEADER + DXT10 + uncompressed (10 layouts) + BC1-7 + BC6H all 14 modes + mipmap + 6-face cubemaps + DX10 arrays + volume textures + 132-entry DXGI table + daily cargo-fuzz + r162 40-case injection-robustness + r176 saturating-math + r192 Criterion benches | ✅ ~96% — uncompressed + BC1-5 + BC7 all 8 modes + BC6H_UF16/SF16 all 14 modes + box-downsample mip chains + cubemap/array + r207 **BC6H second LSQ refinement pass in 17-bit unq integer space** (closes r77 "still deferred" followup; the space `(e0*(64-w) + e1*w + 32) >> 6` decoder interpolation is linear in; pixel-space `half_to_f32`-LSQ over-weights bright-exponent pixels by their float magnitude while unq-space weights uniformly; new `target_unq_uf16` inverts `finish_uf16`, `unq_to_q_uf16` inverts `unquantize_uf16`; SSE-guarded acceptance; **+1.75 dB PSNR uplift (28.00→29.75 dB)** on mixed-dynamic-range test case inside followup's 1-2 dB target) |
| **OpenEXR** | 🚧 ~89% — magic + 8 required attrs + HALF/FLOAT/UINT + NO_COMPRESSION/ZIP/ZIPS/RLE + tiled ONE_LEVEL + sub-sampled chroma + single-part deep scanline + multi-part deep scanline + r130 single-part deep tiled + r181 multi-part deep TILED + r192 multi-part flat TILED ONE_LEVEL read + r196 multi-part flat MIPMAP_LEVELS read + r202 multi-part flat RIPMAP_LEVELS read + r208 **single-part deep tiled MIPMAP_LEVELS read** (`parse_exr_deep_tiled_mipmap` redirects MIPMAP from `parse_exr_deep_tiled` instead of rejecting; composes r130 single-part deep tiled chunk shape with r78 single-part flat MIPMAP iteration order; ROUND_DOWN only; deep ZIP rejected); PIZ blocked on docs trace | ✅ ~95% — RGBA scanline + r130 single-part deep tiled + r181 multi-part deep TILED + r196 multi-part flat MIPMAP_LEVELS + r202 multi-part flat RIPMAP_LEVELS + r208 **single-part deep tiled MIPMAP_LEVELS write** (`encode_exr_deep_tiled_mipmap` + `DeepMipmapTiledInput`/`DeepTiledMipmapLevel`; NONE/RLE/ZIPS; version field only `non_image` (0x800) bit + `tiles[tiledesc,mode=0x01]` + `type="deeptile"`; validated against exrheader "mip-map" + "deeptile" + pure-Rust 24×16-in-8×4 ZIPS pyramid round-trip) |
| **Farbfeld** | ✅ 100% — streaming reader + DoS hardening (dimension overflow + truncated payload guards) + `magick` black-box cross-validator | ✅ 100% |
| **HDR** (Radiance RGBE) | ✅ ~99% — new-RLE + old-RLE + 8 axis-flag combos + shared-exponent + multi-record EXPOSURE/COLORCORR + typed COLORCORR/PRIMARIES/VIEW + apply_exposure/apply_colorcorr + r189 luminance_lm_per_sr_per_m2 + r192 committed-fixture regression anchors + r196 uncompressed scanline R+W + r202 `HdrLimits` resource-cap surface (`parse_hdr_with_limits` + `parse_hdr_with_options_and_limits` + `HdrError::TooLarge`; default 32767×32767 + 256 MiB pixel-bytes; checked_mul width×height×12 BEFORE alloc) + cargo-fuzz harness (decode/roundtrip/headers); 81 lib tests | ✅ ~98% — new-RLE + old-RLE + auto-RLE + XYZE↔RGB + 8 tonemap ops + CRLF + r179 zero-copy `reorient_for_axis_flags` (~6% encode throughput at 1024²) |
| **QOI** | ✅ 100% | ✅ 100% |
| **TGA** | ✅ 100% — types 1/2/3/9/10/11 + TGA 2.0 extension + thumbnail + developer area + CCT + scan-line table + typed AttributesType alpha + r188 image-descriptor bit-4 right-to-left column ordering + r201 §3.3/§C.3 image-identification field round-trip (`parse_tga_image_id` borrowed slice + `splice_image_id` post-encode splice, `TGA_IMAGE_ID_MAX=255`; composes w/ extension-area entry point; 153 tests); magick cross-validated + r154 cargo-fuzz daily decode harness | ✅ 100% — all six image types + full TGA 2.0 extension + thumbnail + RGB24-input entry points |
| **ICER** (JPL) | 🚧 ~78% — Mars-rover heritage; bit-plane scan + compressed/uncompressed segments + 8 filters + IPN 42-155 §III.B context model + r192 §III.E lenient multi-segment decode (`parse_icer_lenient` / `parse_icer_lenient_with_limits` for DSN-packet-loss spaceflight scenario — `LenientDecode { image, received, missing_count }`; segment 0 required to pin canonical strip dims; missing strips reconstruct as flat 128 matching r6 ROI placeholder; trailing-drop truncates; +9 integration tests) | ✅ ~82% — quota encoding + auto wavelet selection + R-D byte-budget + r189 per-segment §III.D uncompressed fallback |
| **WBMP** | ✅ 100% — Type 0 + WbmpLimits DoS caps + adversarial fuzz sweep + r189 caller-selectable `MonoBlack`/`MonoWhite` decode polarity (`parse_wbmp_as` + `CodecParameters::pixel_format` routing) | ✅ 100% |
| **PCX** (ZSoft) | ✅ ~97% — 1/2/4/8 bpp planar + packed-bits + 24 bpp RGB planar + grayscale flag + DCX multi-page + DCX `Demuxer` + r136 fuzz-hardened + r197 Criterion bench harness (decode/encode/roundtrip across 9 scenarios: 1bpp×4 EGA / 2bpp CGA / 4bpp packed / 8bpp palette / 8bpp grayscale / 24-bit RGB + DCX multi-page; xorshift32 deterministic fills, no committed fixtures) | ✅ ~92% — 8 write paths + DCX; r185 framework `Encoder` widened to Rgba/Rgb24/Gray8 + Bgr24/Bgra/MonoBlack/MonoWhite |
| **ILBM** (Amiga IFF) | ✅ ~94% — BMHD/CMAP/CAMG/BODY + ByteRun1 RLE + EHB + HAM6/HAM8 + PBM + SHAM + PCHG + ANIM op-0/op-5 + CRNG/CCRT + DRNG (DPaint IV extended range, true-colour + register cells); lacks ANIM op-7/op-8, DEEP true-colour | ✅ ~84% — IlbmMuxer parity + masking + ANIM op-5 + CRNG/CCRT/DRNG encoder |
| **PICT** (Apple QuickDraw) | ✅ ~99% — v1 + v2 opcode walkers + drawing rasteriser + DirectBitsRect packType 0..4 + Region + clip + pen-size + Compressed/UncompressedQuickTime skip + r186 indexed PixMap variants + r199 §A-3 reserved-Apple-use v2 opcode skip + r205 **v1 (8-bit-opcode) §A-3 Table A-3 completion** (7 state/text-setup opcodes `TxFont/TxFace/TxMode/SpExtra/PnMode/TxSize/TxRatio` + 4 text-glyph opcodes `LongText/DHText/DVText/DHDVText` walked past + 20 implemented Same-shape `SameRect/SameRRect/SameOval/SameArc` reusing v2 `last_*` state slots via `opcode-8` verb-nibble routing + 10 spec-NYI Same-poly/Same-rgn zero-byte no-ops; 257→282 tests; v1 now state-machine-parity with v2 except glyph rendering); lacks text rasterisation + embedded `CompressedQuickTime 0x8200` JPEG decode | ✅ ~93% — `PictBuilder` + every v2 drawing-command family + magick cross-decode bit-exact |
| **SVG** | ✅ ~99% — full shape set + path + gradients + text + mask + clipPath + use/symbol + svgz + SMIL animate/set/animateTransform + CSS3 Selectors L3 + `@import` + `@font-face` + `@keyframes` + Media Queries L4 + viewBox + 17 filter primitives + CSS Values L4 LengthUnit + CSS Easing L2 + SVG 2 §9.6.1 pathLength + SVG 2 §16.3 `<view>` element + fragment-identifier routing (`#MyView` / `#svgView(...)` + percent-decode + spatial/temporal media-fragment fallthrough) + SVG 2 §5.7 `<switch>` conditional processing (requiredExtensions / systemLanguage) + SVG 2 §13.7.1 `<marker>` typed def capture (refX/refY geometric keywords + markerUnits/orient + verbatim round-trip) + SVG 2 §13.2 `context-fill`/`context-stroke` + SVG 2 §16.5 `<a>` hyperlink (renders as group; link target + HTML attrs preserved across round-trip) + SVG 1.1 §11.5 `display` / `visibility` property handling + SVG 2 §5.8 `<title>` / `<desc>` + §5.9 `<metadata>` capture (multilingual lang, round-trip via PreservedExtras) + r172 SVG 2 §11.10.1.1 text-anchor (start/middle/end, inherited) + §11.8.3 textPath start-offset bias | ✅ ~88% — round-trips full shape graph + PreservedExtras side-channel + `<view>` re-emit at trailing edge |
| **PDF** | ✅ ~99% — bytes → Scene via xref/xref-streams/ObjStm + `/Prev` incremental + `/Encrypt` R=2..6 + public-key + PKCS#7 + `/Sig` AcroForm + Doc-Timestamp + text extraction + Linearization + Tagged-PDF + EmbeddedFiles + §12.6 actions + 5 stream filters + §8.11 Optional Content + r194 PDF 2.0 §14.13 Associated Files + r197 6 new §12.5.6 annotation subtypes (Line/Polygon/PolyLine/Ink/Caret/Popup/FileAttachment) + r204 **§12.5.6.22 `/Watermark` (Table 190 + Table 191 `FixedPrint` six-number `/Matrix` + `/H`/`/V` percentages)** + **§12.5.6.23 `/Redact` non-destructive surface** (Table 192 `/QuadPoints` Option-typed + 3-component DeviceRGB `/IC` validated + `/RO` Form XObject as `ObjectId` + `/OverlayText` UTF-16BE-BOM + `/Repeat` + `/DA` + `/Q` clamped 0..=2); 497 + 15 tests; Movie/Sound/Screen/3D/RichMedia remain `Other` | ✅ ~99% — PDF 1.4/1.5 multi-page + paths/gradients/opacity/clip + RGBA + xref-stream + ObjStm + Linearization writer + `/Encrypt` + public-key + `/Sig` + AcroForm + annotation writer + embedded files + RFC 3161 Document Time-Stamp writer |

</details>

<details>
<summary><strong>3D scenes & assets</strong> (click to expand)</summary>

> The typed Scene3D / Mesh / Material PBR / Skin / Animation / Camera / Light / AudioEmitter model lives in `oxideav-mesh3d`, with `Mesh3DDecoder` / `Mesh3DEncoder` traits and a `Mesh3DRegistry` that's parallel to `oxideav-core::CodecRegistry`. Per-format crates register into it. `oxideav-meta::populate_mesh3d_registry(&mut Mesh3DRegistry)` walks every enabled format's `register()`. Lazy bytes flow through `AssetSource` (with a `raw_storage` pass-through hook for archive-backed sources, e.g. ZIP-stored USDZ textures + audio).

| Format | Decode | Encode |
|--------|--------|--------|
| **STL** (ASCII + binary) | ✅ ~99% — ASCII + binary + per-face attrs + 16-bit colour + multi-`solid` + topology + 8-step repair pipeline + r199 `repair_translate_to_positive_octant` + r205 **`repair_make_winding_consistent`** (per-`Triangles`-primitive BFS over manifold-edge adjacency graph; for each edge with exactly two incident faces, unvisited neighbour is flipped (swap slots 1+2) if it walks shared edge in same direction as seed; discriminant-preserving on indexed primitives U16/U32 round-trip; unindexed primitives mirror position swaps to matching-length `normals` swaps; conflicting-edges counter records non-orientable Möbius-like cases; matches `validate`'s `inconsistent_winding_edges` rule — closes diagnostic↔repair matrix for spec's mesh-wide winding invariant; 144→155 lib + 5 integ tests) | ✅ ~99% — both formats + attribute pass-through + `EncodeStats` + configurable float precision |
| **OBJ** (+ MTL) | ✅ ~98% — full Wavefront grammar + MTL (Phong + Wavefront-PBR + map_* options + typed refl) + smoothing/display attrs + free-form geometry + `xyzrgb` per-vertex colour + Bezier/B-spline/NURBS/Cardinal/Taylor `curv` + `surf` 2D-surface tessellation + r171 cargo-fuzz + r188 `curv2` 2D trimming-curve tessellation + r206 **`scrv` special-curve tessellation** (`with_curve_tessellation` resolves `scrv u0 u1 curv2d ...` (spec §"Special curve") into parameter-space `LineStrip` on synthetic mesh `"obj:scrvs"`; reuses round-201 `collect_all_curv2_polylines` pre-pass + `append_curv2_segment` first-vertex-drop-on-join; `obj:tessellated_curve` sentinel filters synthetic geometry at encode time so original `scrv` replays from `extras["obj:freeform_directives"]`); lacks surface-aware tri-edge-constrained re-meshing + multi-patch decomposition + sub-cell trim re-meshing | ✅ ~96% — symmetric + negative-index encoder + polyline rejoin |
| **glTF 2.0** (+ .glb) | ✅ ~97% — JSON + .glb + full PBR + 12 KHR_materials extensions + skin + skeletal animation + sparse accessors + morph-targets + 12 spec-MUST validators + KHR_texture_transform + r188 KHR_mesh_quantization decode + r199 **KHR_node_visibility** (Khronos ratified per-node Boolean visible flag; spec default `true`; false hides node subtree; decoder lifts into `Node::extras["KHR_node_visibility"]=Bool`; encoder rebuilds typed extension + declares in `extensionsUsed`; §3.12 validator rejects undeclared use; +10 tests); lacks KHR_audio_emitter + quantized morph-targets + KHR_materials_variants | ✅ ~92% — symmetric + sparse-encoding heuristic + signed+unsigned normalised-int quantisation + KHR_node_visibility round-trip + KHR_materials_unlit emit |
| **USDZ** (+ USDA) | ✅ ~93% — ZIP STORED walker + USDA parser + UsdGeomMesh + UsdPreviewSurface PBR + UsdUVTexture pass-through + xformOp transforms + UsdMediaSpatialAudio + variantSet + LIVRPS variant-selection composition + composition-arc round-trip + in-archive sublayer + references/payload arc composition + r180 in-layer `inherits`/`specializes` class-arc composition + r188 reader-side CRC-32/ISO-HDLC verify on `walk()` + r200 `.usdc` (Pixar Crate binary) bootstrap parser + r206 **`usdc::decode_int_array` §3b compressed-integer decoder** (2-bit-per-element LSB-first control stream of `ceil(N/4)` bytes; code 0 repeat-prev, 1 i8 delta, 2 i16 delta, 3 absolute i32 reset; variable-width payload; `Error::InvalidData` on short control/payload; test-only `encode_int_array_for_tests` companion; 71 tests); lacks §3a LZ4 wrapper, per-section semantics, FIELDS value-rep type-codes, UsdSkel*, UsdGeomSubset | ✅ ~88% — symmetric writer + zero-re-encode pass-through + variant writer + composition-arc writer |
| **FBX** | 🚧 ~92% — binary + ASCII container + object-graph + mesh + animation + deformers + Material/Texture/Video + bind pose + LayerElementMaterial/Color + Properties70 P-record grammar + multi-UV-set surfacing + r207 **Light + Camera `NodeAttribute` surfacing** (walks `Objects { NodeAttribute }` records with subtype `"Light"`/`"Camera"`, decodes inner `Properties70` via existing `PropertyMap`, binds result onto owning `Model`'s scene-graph `Node::light`/`Node::camera` via `NodeAttribute → Model` OO connection; LightType→Point/Directional/Spot (Area+Volume collapse to Point + extras tag); `Color × Intensity × 0.01` scale; `DecayType≠0` → `range=DecayStart`; Spot reads InnerAngle/OuterAngle deg → half-cone radians; CameraProjectionType + `FieldOfViewY|FieldOfView|FieldOfViewX` → `yfov` via `2·atan(tan(xfov/2)/aspect)`; `AspectWidth/Height` extras; `OrthoZoom` → vertical half-extent; 96 unit + 14 integration tests). Lacks: multi-LayerElementNormal, ASCII writer | ✅ ~58% — symmetric binary writer + opt-in zlib deflate |
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
| **`oxideav-videotoolbox`** | macOS (Apple Silicon + Intel Macs) | 🚧 H.264 + HEVC + ProRes + MJPEG + MPEG-2 + VP9 + MPEG-4 Pt 2 + AV1 (M3+) | 🚧 H.264 + HEVC + ProRes + MJPEG | r198 encoder knobs wired across H.264 / HEVC / MJPEG / ProRes: `bit_rate` → `AverageBitRate`, `options["quality"]` (`Float32 [0,1]`) → `Quality`, `options["profile"]` aliases (H.264 baseline/main/high/extended; HEVC main/main10/main4_2_2_10) → `ProfileLevel`; `make_prores_encoder` dispatches via `prores_codec_type_for_tag()` across all 6 fourCCs (apco/apcs/apcn/apch/ap4h/ap4x). PSNR_Y: MPEG-2 ~61 dB; H.264 ~51 dB; HEVC ~54 dB; ProRes ~52 dB; MJPEG ~36 dB; AV1 ≥30 dB (M3+/macOS 14+). r178 VP9 + r184 MPEG-4 Pt 2 + r190 VOL→ESDS + r205 **AV1 `av1C` extension-atom path** (`FrameSplit::Av1Whole` walks first temporal unit OBU list, finds `OBU_SEQUENCE_HEADER` per spec §6.2.2, wraps in `AV1CodecConfigurationRecord` per ISO BMFF Binding §2.3.3; `parse_av1_seq_header_fields` MSB-first bit-reader recovers av1C fields per §5.5.1/§5.5.2; `find_av1_obu` + `read_uleb128` per §5.3.2/§4.10.5; generalised `BlobDecoder::extradata` to (name, bytes) tuple supporting both `esds` and `av1C`; 11 new unit tests). |
| **`oxideav-audiotoolbox`** | macOS | 🚧 AAC LC + HE-AAC v1/v2 + AAC-LD/ELD + ALAC + iLBC + AMR-NB + AMR-WB + MP3 + **FLAC** | 🚧 AAC LC + HE-AAC v1/v2 + AAC-LD/ELD + ALAC + iLBC | r178 AAC encoder bitrate read-back; r184 iLBC; r190 AMR-NB; r193 AMR-WB; r199 MP3 decode via `kAudioFormatMPEGLayer3` (bit-exact 33×1152 PCM @ ≈89.8 dB SNR); r206 **FLAC decode via `kAudioFormatFLAC`** (RFC 9639 STREAMINFO + frame-header walker + sample-rate/block-size/channel-assignment/bps code tables + `dfLa`-boxed magic-cookie builder/parser; AT-empirical finding: magic-cookie format is Xiph `dfLa` box (8B BoxHeader + 4B FullBox + metadata chain), NOT bare `fLaC + STREAMINFO`; 256-byte cookie ceiling + `min_blocksize ≥ 192`; three-path cookie resolve; bundled mono 16-bit 44.1 kHz fixture decodes byte-exact to staged `expected.wav`; 152 tests). Roadmap: Opus. |
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
| **`oxideav-generator`** | Synthetic media source (`generate://...` URIs) + zero-input filters | ✅ audio synth (sine + chirp/FM/DTMF/multitone/ADSR/ringmod + r180 5-colour noise + r198 `pwm` PWM oscillator + r205 **`supersaw`/`saws` detuned-sawtooth stack** — `voices ∈ [1, 32]` (default 7) placed symmetrically over `[-detune, +detune]` cents around `freq`; per-voice `f_k = freq · 2^(c_k / 1200)` equal-weight averaged so peak stays in `[-amplitude, amplitude]` for every `(freq, voices, detune)` × sample rate; `voices=1` collapses to `sawtooth`, `detune=0` likewise, freq≤0/NaN errors, alias equivalence; clamping `voices=100` → 32) + image (xc/gradient/pattern/fractal/plasma/noise/label; r188 Perlin-2001) + video (testsrc/smptebars/fractal_zoom/gradient_animate/zoneplate); 164 lib + 26 integ tests |
| **`oxideav-rtmp`** | RTMP ingest + push | ✅ Server + client; AMF0/AMF3 parser/builder; Enhanced-RTMP v1 video + v2 audio + ModEx; pluggable key-verification; `rtmp://` PacketSource; symmetric teardown + client `poll_event` + r179 v2 `MultichannelConfig` audio body (24 SMPTE ST 2036-2-2008 22.2 channel positions) + r187 Enhanced-RTMP v2 Multitrack body parser+builder + r198 §E FLV file/byte-stream writer + r205 §E **`FlvReader<R: Write>` inverse-of-FlvWriter** (walks §E.2 9-byte header + §E.3 alternating PreviousTagSize/FLVTAG body, surfaces every tag as typed `FlvTag::{Audio,Video,Script,Unknown}`; every legacy + Enhanced-RTMP v1/v2 wire shape FlvWriter emits round-trips byte-for-byte; refuses bad sig/version/nonzero `PreviousTagSize0`/`DataOffset<9`, enforces §E.3 `PreviousTagSize == 11 + DataSize` + §E.4.1 `StreamID == 0`, configurable `max_tag_size`, §E.4.1 `Filter=1` Annex F encrypted refused; 243 tests = +21) |
| **`oxideav-sysaudio`** | Native audio output | ✅ Runtime-loaded backends (ALSA, PulseAudio, WASAPI, CoreAudio); no C build-time linkage. CoreAudio + WASAPI backends report **real HAL latency** — CoreAudio sums `kAudioDevicePropertyLatency` + `BufferFrameSize` + `SafetyOffset` + `kAudioStreamPropertyLatency`; WASAPI reads `IAudioClock`-derived presentation latency. Output-device enumeration (names + default flag) across WASAPI / ALSA / CoreAudio. r178 per-device routing API + r184 CoreAudio per-device routing (all 4 backends route) + r206 **`StreamRequest::buffer_frames` now honoured on every functional backend** (WASAPI: `buffer_duration_ref_time` converts via `frames × 10⁷ / sample_rate` with `i128` widening + ceil + i64 saturation, `None` keeps the 200 ms default; PulseAudio: `#[repr(C)] pa_buffer_attr` ABI struct + typed `Fn_pa_simple_new::attr` + `make_buffer_attr` filling `tlength = frames × bytes_per_frame` / `minreq` capped at tlength / other fields `u32::MAX` + worker `period_frames` aligned; ALSA + CoreAudio already wired). BT-aware; falls back to software estimate if HAL unavailable. |
| **`oxideav-pipeline`** | Pipeline composition (source → transforms → sink) | ✅ JSON transcode-graph executor; pipelined multithreaded runtime + `Executor::with_channel_caps(ChannelCaps { packets, frames })` configurable per-track depth (embedded `{1,1}` → offline `{64,32}`) + `Executor::with_max_queue_bytes(n)` orthogonal byte-ceiling on demux→worker queues + r178 `Progress::elapsed_micros` wall-clock stamp on every emission (realtime ratio + live-source drift diagnostics) + r184 `packets_skipped: u64` on `Progress` + `ExecutorStats` + r205 **`Progress::packets_read: u64` demuxer-cumulative count** (headroom = `packets_read − frames − packets_skipped` = wedged-decoder signature; demuxer-still-reading vs decode-stage-stalled now distinguishable per emission) + r205 EOF Progress retry-up-to-100×1ms ride-out for backed-up receivers (drops on saturation rather than blocking; fixes pre-existing Windows-runner flake `elapsed_micros_bounded_by_eof_value`) |
| **`oxideav-scene`** | Time-based scene / composition model | 🚧 data model for PDF pages / RTMP streaming compositor / NLE timelines + r179 per-frame `Sample` + animation-track composition helpers + r188 `RasterRenderer` (bg solid/gradient + Rect/Polygon + `ObjectKind::Vector` → RGBA via oxideav-raster) + r198 **`ObjectKind::Group` nested composition** (per-child resolution at scene time, parent affine/opacity/clip merge, cycle-break, dead-child exclusion) + r198 SVG 1.1 path-data lowering (M/L/H/V/C/S/Q/T/Z + relative) + r204 **arc (A/a) per F.6.1** (single-digit `fA`/`fS` flag grammar incl. minified `A5,5 0 0010,10`; degrees→radians; F.6.2 out-of-range: neg-radii absoluted / zero radius → `line_to` / coincident endpoints omitted; reuses `oxideav_core::PathCommand::ArcTo` + `oxideav_raster::flatten_arc_to_cubics`; `parse_bbox` extends pen-walk with `max(|rx|,|ry|)` endpoint enclosure; `SvgPathError::InvalidArcFlag`); 201 tests — SVG 1.1 path grammar complete; image/video/text ObjectKind pending |
| **`oxideav-audio-filter`** | Audio effects & conversions (streaming) | ✅ ~48 filters: classic + transient/spatial/restoration family + r106 SlewLimiter + r188 LR4 crossover + r198 `true_peak_detector` + r205 **`state_variable` Chamberlin SVF** (Chamberlin two-integrator-loop State Variable Filter; single recurrence emits LP / BP / HP / Notch from one pair of integrator states; `SvfMode` selects output tap without touching state; analog-prototype-matched `f = 2·sin(π·f_c/f_s)` + `q = 1/Q`; clamps enforce `f_c ≤ f_s/6.5` and `Q ∈ [0.5, 50.0]`; `"svf"` registry entry with JSON `mode`/`cutoff_hz`/`q` keys; modulation-friendly synth filter property — coefficient resolve is one `sin` per `set_cutoff`; 280 tests) — see crate README for the catalogue |
| **`oxideav-image-filter`** | Single-frame image effects (stateless) | ✅ 130 filter types / 178 factory names — r198 `Gabor` + r205 **`Niblack` adaptive local-statistics threshold** (Niblack 1986 textbook §5.1 page-segmentation example; per-pixel `T(x,y) = μ + k·σ` over `(2·radius+1)²` neighbourhood, default `k = -0.2`; two-pass separable box-sum via `Var(X) = E(X²) − E(X)²` identity, variance clamped to 0 before `sqrt` for FP-cancellation safety; `O(W·H)` regardless of radius; joins segmentation family at the local-stats threshold slot — complements `AdaptiveThreshold` (k=0) + `OtsuThreshold` (global); 15 unit + 3 factory smoke tests) + bundled rule-E scrub: 86 doc-comments across 78 src files retired pre-existing brand-named CLI references to neutral wording — see crate README for the catalogue |
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
