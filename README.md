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
| WAV       | ✅ | ✅ | ✅ | LIST/INFO metadata; byte-offset seek; BWF `bext` metadata (EBU 3285) |
| FLAC      | ✅ | ✅ | ✅ | VORBIS_COMMENT, streaminfo, PICTURE block; SEEKTABLE-based seek; CUESHEET round-trip (read + write per RFC 9639 §8.7) |
| Ogg       | ✅ | ✅ | ✅ | Vorbis/Opus/Theora/Speex pages + comments; page-granule bisection + page-level seek index (`open_indexed`); chained-link-aware duration (RFC 3533 §4); page-loss/hole detection via `page_sequence_number` (RFC 3533 §6, `hole_count()`) + continued-flag framing-consistency check (`framing_error_count()`) + page-sync recapture after parsing errors (RFC 3533 §3 + §6, `resync_count()`) + public page-level CRC-32 validation API (`validate_page_crc` / `compute_page_checksum` / `read_page_checksum`) |
| Matroska  | ✅ | ✅ | ✅ | MKV/MKA/MKS; DocType-aware probe; Cues seek; SeekHead emit; Chapters round-trip; Attachments + subtitle tracks; RFC 9559 §5.1.8 typed Tag decoder + RFC 9559 §5.1.4.5.5 / §10.3 opt-in block lacing on write (`MkvMuxer::with_block_lacing(LacingMode { None, Xiph, Ebml, FixedSize })`) + EBML CRC-32 validation on Top-Level masters (`crc_status()`) + TrackOperation typed decode (§5.1.4.1.30 3D plane-combine / block-join, UID→stream-index resolved) + ContentEncodings typed decode (§5.1.4.1.31) + Block-scoped Header-Stripping reversal on read (algo 3 — original frame bytes restored; lacks zlib/bzlib/lzo1x + decryption) + typed `chapters()` accessor (RFC 9559 §5.1.7 EditionEntry tree + multilingual ChapterDisplay rows + nested atoms, depth-first indexing) + ChapterFlagEnabled + Medium-Linking fields + ChapProcess (DVD-menu/Matroska-Script chapter commands, raw) + RFC 9559 §5.1.5.1.2.3 CueRelativePosition demux+mux round-trip (finer seek — direct jump to indexed block) + RFC 9559 §5.1.4.1.28 Video FlagInterlaced + FieldOrder typed decode (`VideoInterlacing` w/ spec-correct "ignored unless FlagInterlaced=1" gating) + RFC 9559 §5.1.4.1.28.8–14 Video geometry quartet (PixelCrop{T,B,L,R} + DisplayWidth/Height/Unit with spec-correct derived-default rule) |
| WebM      | ✅ | ✅ | ✅ | First-class: separate fourcc, codec whitelist (VP8/VP9/AV1/Vorbis/Opus); inherits Matroska Cues seek |
| MP4       | ✅ | ✅ | ✅ | mp4/ismv brands; faststart; iTunes ilst; fragmented demux + mux (DASH/HLS/CMAF) + sidx/mfra/tfra; AC-3/E-AC-3/DTS sample-entry FourCCs; subtitle/timed-text demux (tx3g/wvtt/stpp/sbtt/stxt/c608/c708); §8.12 protected sample-entry unwrap (sinf/frma/schm); §8.3.3 typed track references + edts/elst edit-list mux (§8.6.5–6 positive start delay) + §8.4.6 extended-language tag (elng) demux → `params.options["language"]` (BCP 47) + §8.10.4 Track Kind Box (`kind`) demux → `params.options["kind_<n>"]` (DASH role / iTunes scheme labelling); §8.6.1.4 Composition-to-Decode box (cslg) + §8.6.3 Shadow Sync Sample box (stsh) + §8.9 sample-group (sbgp/sgpd) demux + §8.6.4 sdtp Sample Dependency Type Box demux + r127 write-side §8.16.2 `styp` Segment Type emitter (DASH/CMAF, `write_fragmented_segment_with_styp` per-segment override) + r130 §8.9 sample-group muxing (sbgp+sgpd write-side, v1/v2 selection per §8.9.3.2); lacks CENC decryption (tenc/pssh/senc) |
| MOV (QuickTime) | ✅ | — | ✅ | Apple QTFF + ISO BMFF meta + HEIF/HEIC item-properties + grid/iovl/tmap + symmetric muxer + fragmented-MP4 seek + DASH sidx/styp + r147 stbl + r150 traf saiz/saio sample-aux (CENC envelope, per-fragment); ffprobe-accepted |
| AVI       | ✅ | ✅ | ✅ | OpenDML 2.0 super-index + AVIX + dmlh + vprp + 2-field interlaced + truncated-head recovery + VBR audio + LIST INFO + typed `PaletteChange`/`TextChunk`/`AvihFlags`/`Idx1Flags` + opt-in idx1↔ix## synthesise + WAVE_FORMAT_* + per-stream budget + ODML keyframe seek + top-down DIB + BI_BITFIELDS + WAVEFORMATEXTENSIBLE 0xFFFE + `strn` name + `strd` codec-driver + `avih.dwPaddingGranularity` round-trip + stream-aligned `JUNK` packet emission (`with_padding_granularity(n)`) + CBR-audio `ix##` standard-index block-alignment validator (`cbr_audio_block_alignment_violations()`) + OpenDML super-index `dwDuration` round-trip + reader-side `dmlh.dwTotalFrames` consistency cross-check + `vprp` typed frame-aspect-ratio accessor + `IDIT` digitization-date chunk (RIFF Hdrl Tags DateTimeOriginal): parse + emit + verbatim round-trip, `digitization_date()` accessor + `avi:idit` metadata + `ISMP` SMPTE-timecode chunk (RIFF Hdrl Tags TimeCode): parse + emit + round-trip, `smpte_timecode()` accessor + `avi:ismp` metadata + `strh.rcFrame` destination rectangle (parse + emit, `stream_frame_rect()` accessor + `with_stream_frame_rect` mux) + per-stream `strh.wLanguage` LANGID (parse + emit, `stream_language(n)` accessor + `with_stream_language(n, langid)` mux) |
| Blu-ray (BD-ROM) | ✅ | — | — | `oxideav-bluray` Phase 2 — UDF 2.50 mount (ECMA-167 3rd ed.) + BDMV walk (`index.bdmv`/`MovieObject.bdmv`/`.mpls`/`.clpi`) + `.m2ts` stream (192→188-byte TP_extra_header strip) + `bluray://` URI handler with auto-detect; r93 typed `Cpi { ep_map: Vec<EpMap { stream_pid, ep_stream_type, entries: Vec<EpEntry { pts_ep_start, spn_ep_start, is_angle_change_point, … }> }> }` CPI EP_map decode per BD-ROM AV §5.7 (coarse + fine two-level table folded into a flat per-PID list a seeker can binary-search); r96 keyframe-aligned `TitleSource::seek_to(pts_90k)` (PTS→clip→I-frame→SPN×192, AACS-unit-aligned); `StreamDecryptor` trait hooks `oxideav-aacs` without hard dep. Lacks HDMV opcode exec, BD-J, multi-angle EP_map seek, cross-PlayItem STC PTS remap |
| DVD-Video | ✅ | — | — | `oxideav-dvd` Phase 3b — ISO 9660 + UDF 1.02 mount + VIDEO_TS walk + IFO body parser (VMGI/VTSI + TT_SRPT + VTS_PTT_SRPT + PGCI [+ PGC subpicture colour-LUT + pre/post/cell nav command table] + VTS_C_ADT + chapter materialiser) + VOB demux (MPEG-PS pack/PES + Nav-Pack PCI/DSI [+ PCI highlight + DSI typed sections] + DVD substream router for AC-3/DTS/LPCM/subpicture) + VOB → MKV mux (`mkv-output` feature; per-PES PTS preserved + ChapterAtom per `DvdChapter` via RFC 9559 §5.1.7) + `dvd://` URI handler. Lacks VM (HDMV opcodes + SPRMs/GPRMs), CSS auth (Phase 3c + `oxideav-css`) |
| MP3       | ✅ | — | ✅ | demuxer LANDED (ID3v2/ID3v1 skip + Xing/Info VBR + CBR/VBR seek_to) |
| IFF / 8SVX| ✅ | ✅ | — | Amiga IFF with NAME/AUTH/ANNO/CHRS |
| IVF       | ✅ | — | — | VP8 elementary stream container |
| AMV       | ✅ | — | — | Chinese MP4 player format (RIFF-like) |
| FLV       | ✅ | — | — | Flash Video — MP3/AAC/H.264 audio + VP6f/VP6a/H.264 video + Enhanced RTMP ExVideoTagHeader (AV1/VP9/VP8/HEVC/VVC + AVC FourCC, SequenceStart→extradata, HEVC SI24 CTO, Multitrack) + AMF0 onMetaData/onXMPData/onCuePoint + Annex F encryption headline (v1/v2) + FrameType 5 command tags + typed E-FLV ModEx walk + Enhanced-RTMP `VideoCommand` UI8 on Ex video path (StartSeek/EndSeek per spec) + E-FLV multitrack body splitter (per-track split + default-track routing) + E-FLV VideoPacketType.Metadata HDR colorInfo → `colorinfo.*` metadata (BT.2020 / hdrCll / hdrMdcv) + E-RTMP-v2 onMetaData audio/videoTrackIdInfoMap → metadata bag; seek_to via keyframes + onMetaData `audiosamplesize` → `CodecParameters::sample_format` (legacy + ExAudio paths) |
| WebP      | ✅ | ✅ | — | RIFF/WEBP (lossy + lossless + animation; ANIM + ANMF emit) |
| TIFF      | ✅ | — | — | TIFF 6.0 single-image; magic II*\0 / MM\0* |
| PNG / APNG| ✅ | ✅ | — | 8 + 16-bit, all color types, APNG animation |
| GIF       | ✅ | ✅ | — | GIF87a/GIF89a, LZW, animation + NETSCAPE2.0 loop + multi-frame compositor (§23 disposal-method state machine, 4 modes) — clean-room rebuilt from CompuServe spec (no external decoder consulted) |
| JPEG      | ✅ | ✅ | — | Still-image wrapper around the MJPEG codec |
| BMP       | ✅ | ✅ | — | Windows bitmap — DIB headers BITMAPINFOHEADER / V4 / V5, 1/4/8/16/24/32-bit; also exposes the DIB helpers used by ICO / CUR sub-images |
| Netpbm    | ✅ | ✅ | — | All seven PNM magics + PAM (P1-P7); 1/8/16-bit; comment-tolerant ASCII + binary; .pbm/.pgm/.ppm/.pnm/.pam |
| ICO / CUR | ✅ | ✅ | — | Windows icon + cursor — multi-resolution, BMP and PNG sub-images |
| slin      | ✅ | ✅ | — | Asterisk raw-PCM: .sln/.slin/.sln8..192 |
| MOD / S3M / STM | ✅ | — | — | Tracker modules (decode-only by design; STM is structural-parse only) |

Cross-container remux works for any pair whose codecs don't require
rewriting (FLAC ↔ MKV, Ogg ↔ MKV, MP4 ↔ MOV, etc.).

### Content protection

| Layer | Status | Notes |
|-------|:-------|-------|
| AACS  | ✅ Common 0.953 + BD-Prerecorded 0.953 | `oxideav-aacs` clean-room — KEYDB.cfg parser, `MKB_RO.inf` / `Unit_Key_RO.inf` parsers, Subset-Difference tree walk, Device-Key → Processing-Key → Media-Key → VUK derivation, AES-128-CBC Aligned Unit decryption, Title Key unwrap + Phase B SCSI MMC drive-command wire layer (REPORT_KEY / SEND_KEY / READ_DISC_STRUCTURE typed CDBs + AGID / Drive-Cert-Challenge / Drive-Key / Host-Cert-Challenge / Host-Key / Volume-ID sub-payload codecs + `DriveCommand` trait + `MockDrive` synthetic-fixture impl) + Phase C Drive-Host AKE (clean-room ECDSA over the AACS 160-bit curve + FIPS 180-2 SHA-1 + AES-128-CMAC; `host_authenticate` §4.3 state machine + `DriveAuthState` wired into `MockDrive`; Bus Key = lsb_128 of shared ECDH x-coord; §4.4 Volume-ID transfer w/ CMAC verify). Lacks platform `DriveCommand` back-ends (Phase D), AACS 2.0 (UHD-BD) |

</details>

### Codecs

> Each row below is a current-state summary. For round-by-round history, design notes, and per-feature trade-offs, see the per-crate `README.md` and `CHANGELOG.md` in `crates/oxideav-<codec>/`.

<details>
<summary><strong>Audio</strong> (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **PCM** (s8/16/24/32/f32/f64) | ✅ 100% | ✅ 100% |
| **slin** (Asterisk raw PCM) | ✅ 100% | ✅ 100% |
| **FLAC** | ✅ 100% — bit-exact vs RFC 9639 + CUESHEET → Chapter API | ✅ 100% — bit-exact roundtrip + LPC order/window/precision search + closed-form Rice estimate (4.6-6.0× faster); r147 flamegraphs (`best_partition` ~61% of encode CPU) + r150 +3 bench scenarios (mono-S24, wasted-bits, CONSTANT); scratch-reuse refactor evaluated and rejected (~10% regression on macOS arm64) |
| **Vorbis** | 🚧 r8 (post-2026-05-20 orphan) — identification + comment + §3.2.1 codebook + Huffman tree + full §4.2.4 setup-header walker + §3.2.1/§3.3 VQ vector unpack (entry → vector via multiplicands + minimum_value + delta_value + sequence_p) + §8.6 residue decode (formats 0/1/2) + §7.2.3/§7.2.4 floor type 1 packet decode + curve computation + §6.2.2/§6.2.3 floor type 0 LSP per-packet decode + curve computation + §1.3.2/§4.3.1 Vorbis window + §4.3.5 inverse channel coupling + §4.3.3 nonzero-vector propagate + §4.3.6 floor×residue dot product + §4.3.1–§4.3.6 audio-packet driver (mode + window + per-channel floor + nonzero/coupling/dot-product; returns Err(ImdctStage) at boundary) + §4.3.8 overlap-add primitive (3/4-vs-1/4 alignment, mixed-size short→long signed-arithmetic, §1.3.2 squared-window perfect-reconstruction); **§4.3.7 IMDCT blocked — Vorbis I spec defers to external reference (clean-room barred)** | 🚧 scaffold |
| **Opus** | 🚧 r11 (post-2026-05-20 orphan) — RFC 6716 §3.1 TOC + §3.2 frame packing + §4.1 range decoder + §4.2.7.1–§4.2.7.5.1 SILK header + §4.2.7.4 subframe gains + §4.2.7.5.2 LSF Stage-2 residual + §4.2.7.5.3 NLSF reconstruction (Tables 23/24 cb1_Q8 + IHMW w_Q9 in [1819, 5227] + final NLSF_Q15 clamp) + §4.2.7.5.4 NLSF stabilization (RFC 8251 §7 erratum) + §4.2.7.5.5 NLSF interpolation + §4.2.7.5.6 NLSF→LPC core conversion (`silk_NLSF2A`: Table 27 ordering + Table 28 Q12 cosine + i64 P/Q recurrence) + §4.2.7.5.7 LPC range-limiting bandwidth expansion (silk_bwexpander_32 chirp loop + Q12 saturation) + §4.2.7.5.8 LPC prediction-gain stability + §4.2.7.6 LTP parameters + §4.2.7.7 LCG seed + §4.2.7.8 excitation reconstruction (all 6 substeps, Tables 43–53) + §4.2.7.9.2 LPC synthesis filter (carries d_LPC unclamped lpc[] across subframes, clamp to ±1) + §4.2.7.9.1 LTP synthesis filter (5-tap Q7 + Region-A/B rewhitening of out[]/lpc[] + LSF-interp split branch) + §4.2.8 SILK stereo unmixing (MS→LR) + §4.2.3 SILK header bits (VAD + LBRR) + §4.2.4 per-frame LBRR flags (Table 4 40/60 ms PDFs) + §4.2.9 SILK resampler delay budget (Table 54 NB/MB/WB = 0.538/0.692/0.706 ms; SILK internal 8/12/16 kHz; output 8/12/16/24/48 kHz); 339 tests; lacks CELT | 🚧 scaffold |
| **MP1 / MP2** | ✅ Layer I + Layer II decode to PCM + §2.4.3.1 CRC-16 verification + r150 mp2 §2.4.1.6/§2.4.3.1/§2.4.3.2 frame-level decode loop (`decode_frame()` returns 1152 PCM samples/channel; staged 31-frame stereo fixture decodes cleanly; 98 tests); lacks bit-exact PSNR validation + Layer II encoder + LSF (#1076) + Annex D psy | ✅ ~100% Layer I encoder — Annex C polyphase + scalefactor + energy allocator + Table C.3 quantizer; 1 kHz tone @ 192 kbit/s mono self-roundtrip RMS<0.01; lacks Annex D psy |
| **MP2** | 🚧 r126 step 1 (post-2026-05-24 orphan) — §2.4.1.3 / §2.4.2.3 Layer II frame-header parser (full validation: bad sync, LSF, layer code, bitrate, sample-freq, emphasis, disallowed (bitrate, mode) matrix) + §2.4.3.1 frame sizing + Annex B Table 3-B.1 scalefactors + §2.4.1.6 audio-data side info (Tables 3-B.2a..d bit-allocation + Table 3-B.4 quant classes + joint-stereo allocation sharing + scfsi + 3-granule scalefactor expansion) + §2.4.3.3.4 sample requantizer + r142 §2.4.3.1 CRC-16 (G(X)=X^16+X^15+X^2+1, 0xFFFF init; header bits 16-31 + alloc + scfsi); 72 tests; lacks §2.4.3.2 polyphase synthesis + encoder | 🚧 scaffold |
| **MP3** | ✅ ~100% — MPEG-1/2/2.5 frame walker + Layer III main-data Huffman + requantize + stereo (MS+intensity) + IMDCT + polyphase synthesis; end-to-end bitstream→PCM bit-exact vs mpg123 on test corpora; ID3v2/Xing demuxer w/ seek | 🚧 ~82% — Phase-2 encoder: PCM→MP3 + outer loop + bit-reservoir + true-VBR + CRC-16 + stereo + joint-stereo MS + auto MS/LR + r151 §2.4.3.4.10.2 forward short-block MDCT path (force-toggle for testing); lacks Annex D psy (#1048) + intensity-stereo + LSF + block-type auto-decision + mixed-block encode |
| **AAC** | 🚧 Phase 1 (post-r111 orphan-rebuild) — ADTS + raw_data_block walker + r126 AudioSpecificConfig + program_config_element + Phase-2 encode writers: section_data + ics_info + pulse_data + tns_data + r149 §4.4.6 scale_factor_data parser+writer (Table 4.53 + Table 4.A.1 codebook 12 over 19 bits; 5th syntax-element writer, 227 tests); decoder body still pending | 🚧 scaffold |
| **CELT** | 🚧 r6 (post-2026-05-20 orphan) — RFC 6716 §4.1 range decoder + §4.3 prefix + §4.3.2.1 coarse-energy scaffold + §4.3.3 bit-allocation fields + §4.3.4 tf_change/tf_select; 73 tests; blocked on docs #936 (Laplace) + #943 (cache_caps50 / LOG2_FRAC_TABLE / alloc loop) | 🚧 scaffold |
| **Speex** | 🚧 r3 (post-2026-05-19 orphan) — Ogg stream-header + narrowband frame-header + Table 9.1 NB sub-mode budgets + narrowband CELP frame-body bit-reader (raw indices, 20 fields/frame); lacks LSP-VQ + pitch/innovation codebooks (#969) + LSP→LPC + synthesis | 🚧 scaffold |
| **GSM 06.10** | 🚧 scaffold (orphan rebuild post-audit 2026-05-25 — libgsm-derived; blocked on staged ETSI GSM 06.10 docs) | 🚧 scaffold |
| **G.711** (μ/A-law) | ✅ 100% | ✅ 100% |
| **G.722** | 🚧 scaffold (orphan rebuild post-audit 2026-05-25 — ITU-reference-code-derived tables; blocked on staged G.722 docs) | 🚧 scaffold |
| **G.723.1** | ✅ 100% | ✅ 100% — both 5.3k + 6.3k |
| **G.728** | ✅ 100% — LD-CELP 50-order | ✅ 100% |
| **G.729** | 🚧 scaffold (orphan rebuild post-audit 2026-05-24) — register-only; prior decoder/encoder force-erased (LSP/gain tables had been transcribed from ITU reference C *software*, not the Recommendation text); clean-room rebuild pending (#859 trace doc + #1028 yank) | 🚧 scaffold |
| **IMA-ADPCM (AMV)** | ✅ 100% | ✅ 100% |
| **MS-ADPCM / IMA-ADPCM (WAV)** | ✅ 100% | ✅ 100% — block-aligned WAV encoder for both nibble layouts |
| **8SVX** | ✅ 100% | ✅ 100% |
| **iLBC** (RFC 3951) | ✅ 100% — NB 20/30 ms | ✅ 100% |
| **AC-3 / AC-4** (Dolby Digital / Dolby AC-4) | ✅ ~96% — AC-3 full decode + E-AC-3 SPX (§E.3.6 HF regen) + transient pre-noise (§E.3.7.2 TPNP) + multichannel fbw+LFE+coupling AHT (§3.4 Adaptive Hybrid Transform incl. LFE-channel mantissas + lfeahtinu synthesis + interleaved cplahtinu coupling-channel mantissas) + §7.8.2 LtRt matrix-encoded stereo downmix + r126 Annex D §2.3 alternate-syntax mix-level params (ltrtcmixlev/ltrtsurmixlev/lorocmixlev/lorosurmixlev via xbsi1, reserved-code resolution per Tables D2.3-D2.6) + WAVE_FORMAT_EXTENSIBLE; AC-4 ~98% decoder + IMS encoder ~65% (mono/stereo/5.0/5.1/7.1 Cfg3Five + 5_X ASPX_ACPL_3 + 7.1 3/4/0.1 SIMPLE/ASPX_ACPL_2 LFE multichannel + r126 7.0 SIMPLE/Cfg3Five) | 🚧 AC-3 ~95% — acmod 1/2/2.1/3/6/7 + LFE + DBA + 5-fbw coupling + E-AC-3 indep+dep + per-channel PSNR gates + r95 two-stage equalise + spread-cap greedy for per-channel `fsnroffst[ch]` (≤ ~1.5 dB spread; closes r91 cheap-mantissa runaway) |
<!-- ac3 decode r129: E-AC-3 mixmdata mix-levels (ltrt/loro c/sur) now surfaced + routed through §7.8 downmix in process_eac3_frame -->
| **AC-4** (Dolby) | 🚧 ~98% — A-SPX + DRC + 60+ ETSI codebooks + 5_X/7_X ACPL_1/2/3 + cfg0/1/2/3 + LFE + SSF/SNF + SAP + Pseudocode 121 companding + IMS bitstream_version≥2 walker + 7_X SIMPLE/Cfg3Five inner 5-ch IMDCT; lacks ETSI fixture RMS audit, object/a-joc substreams | 🚧 IMS ~65% — v0/v2 TOC + mono SIMPLE/ASF + stereo SIMPLE 2× SCE split-MDCT + joint M/S CPE + 5.0/5.1/7.1 SIMPLE Cfg3Five + 5_X SIMPLE/ASPX_ACPL_1/2 + ASPX_ACPL_3 multichannel encoder (aspx_config + acpl_config_1ch/2ch + companding + stereo_data + aspx_data + acpl_data; ACPL_1 joint-MDCT surround residual; zero-delta Huffman codewords for all 18 ASPX + 24 ACPL HCBs) + 7.0 SIMPLE/ASPX_ACPL_2 (first 7_X ACPL encoder path, §4.2.6.14 Table 33, round-trips to 7-ch PCM) + 7.0/7.1 SIMPLE/ASPX_ACPL_1 (joint-MDCT surround residual; LFE→slot 7) + r125 7.0 SIMPLE/Cfg3Five immersive encoder (Cfg3Five five_channel_data + additional two_channel_data Lb/Rb pair, no LFE, per-channel SNR ≥ 23 dB) + r132 real per-band β extraction in ACPL_1 5.0 encoder + r135/r139 real per-band α+β for 7_X (7.0 + 7.1-with-LFE) ASPX_ACPL_1 + r144 real per-band α+β for 5_X ASPX_ACPL_2 (§5.7.7.5 Pseudocode 116 β derivation from (L,Ls) + (R,Rs) MDCT energy ratios); lacks real β for ACPL_3 + real ASPX envelope coding |
| **MIDI** (SMF) | ✅ ~99% — SMF Type 0/1/2 → PCM via 32-voice mixer + SF2/SFZ/DLS + DLS `art1`/`art2` + SF2 EG2 + 2-pole resonant low-pass biquad on shared SamplePlayer + SFZ filter EG (`cutoff` / `resonance` / `fil_type` covering all 6 SFZ v1 shapes + `fileg_*` envelope opcodes) + MPE v1.1 + RPN 0/1/2/5/6 + CA-25 Master Tuning + MIDI Tuning Standard (per-key + scale/octave microtuning) + Universal Master Volume SysEx + Master Balance SysEx + GM2 Global Parameter Control (CA-024 reverb/chorus) + Data Inc/Dec (CC 96/97, RP-018) + `SmfFile::time_signatures()` iterator (FF 58, stable-merge across tracks) + r125 `SmfFile::tempo_map()` iterator (FF 51, BPM-cached, stable-merge across tracks) + r128 `SmfFile::key_signatures()` iterator (FF 59, circle-of-fifths label resolver) | — synthesis only |
| **NSF** (NES) | 🚧 ~90% — full 6502 + IRQ/NMI + 5/5 2A03 APU + DMC DMA + six expansion chips + NSF v1/v2/NSFe + Dendy region + mixe per-device gain + plst/psfx playlist iteration + region-aware noise period (NTSC+PAL) + FDS frequency-modulation unit + FDS volume/mod envelope ramp generators + FDS $4023 master sound-enable/waveform-halt + FDS $4090..=$4097 read-register window; VRC7 still 2-op approximation pending #861 | — synthesis only |
| **Shorten** (.shn) | 🚧 r5 (post-2026-05-18 orphan) — `ajkg` magic + v2/v3 ulong + svar(n) + per-block function dispatch + VERBATIM/QUIT + DIFF0..3 + Rice residual + per-channel carry + spec/05 §2.5 running mean estimator (sliding-window `mu_chan`; DIFF0/ZERO consumers) + QLPC quantised-LPC predictor (§3.5) + r6 BLOCKSIZE/BITSHIFT housekeeping + r7 full-stream `decode_stream` driver (header + all block commands + round-robin channel cursor + running blocksize/shift + carries + mean estimators → per-channel PCM); + r145 `oxideav_core::Decoder` trait wiring (`ShortenDecoder` packet→`AudioFrame` per spec/05 §6 file-type table for u8 / s16hl / s16lh; bit-exact vs direct chain through registered `make_decoder` factory); 107 tests; lacks encoder + 8 unpinned TR.156 file-type labels | 🚧 scaffold |
| **TTA** (True Audio) | ✅ ~97% — TTA1 fmt=1/2 + password + trace tape + ID3v1 / APEv2 trailer + multi-frame format=2 trace coverage closes audit/07 §6.2 (HEADER_CRC carries IEEE-802.3 CRC32; LMS_PRE digest seal per spec/07 §3.5/§3.6) | ✅ ~96% — TTA1 fmt=1/2 + password; bit-exact self-roundtrip |
| **WavPack** | 🚧 r8 (post-2026-05-18 orphan) — v4 block/metadata/decorrelation/entropy parse + LSB bit-reader + run-length n-decoder + Golomb (base,add) interval + per-sample value reconstruction + single-call `decode_sample` + EntropyInfo→Medians bridge + block-header accessor coverage (lossless / sample-rate sentinel / experimental / effective bit-depth / audio-block / payload-bytes) + r130 MD5 typed view + walker finders + payload-kind predicates; 103 tests; lacks median-adaptation amount (#992) / prediction loop / float+multichannel / CRC / encoder | 🚧 scaffold |
| **aptX** (classic + HD) | 🚧 ~70% — 4-band QMF + ADPCM; bit-exact NDA-blocked | — |

</details>

<details>
<summary><strong>Video</strong> (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **MJPEG** | ✅ ~96% — baseline + progressive 4:2:0/4:2:2/4:4:4/grey + 12-bit 4:2:0/4:2:2/4:4:4 YUV + SOF9 arithmetic + lossless SOF3 grey P=2..16 + 3-comp RGB P=8 + RFC 2435 RTP/JPEG depacketization (+ §4.2 cross-frame static-Q table caching) + packetization | ✅ ~95% — baseline + progressive + lossless SOF3 grey/RGB (all 7 Annex H predictors) + DRI/RSTn restart markers + non-zero point transform Pt 0..15 |
| **FFV1** | 🚧 ~70% (post-2026-05-18 orphan) — RFC 9043 §3.x decoder + §4.x demux + end-to-end decode_frame driver (YCbCr + RGB Y/Cb bit-exact; RGB Cr divergence open) + r142 §4.9 Slice Footer encoder + r146 §4.6 Slice Header encoder + r149 §3.8.2 Golomb-Rice content encoder primitives (BitWriter + put_ur/put_sr/put_vlc_symbol; 281 tests); lacks Decoder registration (#904) + RGB Cr fix + §4.8 run-mode encoder dispatch + frame-level encoder | 🚧 scaffold |
| **MPEG-1 video** | 🚧 clean-room rebuild (post-audit 2026-05-18) — sequence/GOP/picture/slice headers + macroblock walk + intra-DC + §2.4.3.7 dct_coeff walker + §2.4.4.1/§2.4.4.2 dequantiser bodies + r149 §7.6.3.6 MPEG-2 dual-prime additional arithmetic; IDCT pending (Annex A normative ref to IEEE P1180/D2 #1110) | 🚧 scaffold |
| **MPEG-2 video** | 🚧 r6 (post-2026-05-18 orphan) — §6.2.2.1/§6.2.2.3/§6.2.2.6 sequence/GOP + §6.2.3 picture_header + §6.2.3.1 picture_coding_extension + §6.2.4 slice_header + §6.2.5 macroblock_address_increment (Annex B Table B-1 33-code VLC + escape chain + MPEG-1 stuffing) + §6.2.5.1 macroblock_type + §6.2.5.3 coded_block_pattern (Table B-9 + 4:2:2/4:4:4 ext) + §6.2.5.1 macroblock_modes() motion-type/dct_type tail (Tables 6-17/6-18/6-19) + §6.2.5.2 motion_vectors() / motion_vector() (Tables B-10 motion_code + B-11 motion_residual, dual-prime + concealment) + §7.6.3.1 PMV reconstruction (wrap-around + half-pred for field-in-frame vertical + PMV reset + §7.6.3.7 chroma scaling) + §7.6.3.3 inter-vector PMV-update (Tables 7-10 frame / 7-11 field, intra ‡/◊ + non-intra §-zero-reset rows + spec-unreachable rejection); r13, 231 unit tests; lacks §7.6.3.6 dual-prime additional + residual VLCs / IDCT | 🚧 scaffold |
| **MPEG-4 Part 2** | 🚧 ~47% (post-2026-05-18 orphan) — I-VOP intra decode through §7.4.5 IDCT + Figure 7-5 predictor gathering + §6.2.5 video_packet_header (rectangular) + r150 §7.8.7.3 S(GMC)-VOP averaged-vector substitution (Nb=256 luminance-pel-wise MV); lacks Figure 7-34 MV-predictor (#1125) + inter+B-VOP + encoder | 🚧 scaffold |
| **Theora** | 🚧 r8 (post-2026-05-20 orphan) — §6.1–§6.4 setup-header + Appendix B.2/B.3 VP3-default tables + §6.4.2 quantization-parameters + §6.4.3 quant-matrix + §6.4.4 DCT-token Huffman tables (80-table binary-tree) + §7.1 frame-header decode (FrameType + qis) + §7.2 long-/short-run bit strings (Tables 7.7/7.11, RLEN=4129 fresh-BIT exception) + §7.3 coded block flags decode (intra short-circuit + inter SBPCODED/SBFCODED/per-block chain with edge-super-block step 2(g)); 159 tests; §6.4.1 LFLIMS body blocked (#944) | 🚧 scaffold |
| **H.263** | 🚧 ~80% (post-2026-05-18 orphan) — §5.1-§5.4 baseline + §6 IDCT/MV/half-pel/INTER + Annex J §J.3 deblock + Annex I §I.2/§I.3 AIC + Annex D §D.2 UMV + Annex F §F.2 4-MV candidate + Table F.1 chroma snap + Annex F §F.3 OBMC + §5.1.4 PLUSPTYPE + Annex I §I.3 INTRA-coef VLC + Annex K §K.2 Slice-Structured header + r151 Annex F INTER4V driver wiring (Figure-5 mvs4 + §F.3 per-block OBMC dispatch + Table F.1 chroma) into decode_picture (289 tests); lacks Annex K driver wiring + PB-frames + PLUSPTYPE-gated driver + AIC §I.3 absorbed-INTRADC | 🚧 scaffold |
| **H.261** | ✅ ~98% — I+P QCIF/CIF + integer-pel + loop filter + §5.4 BCH FEC + §5.2 + Annex B HRD + RFC 4587 RTP payload format (4-byte §4.1 header + GOB-aligned packetizer + `-16` MVD guard) + RFC 4587 §6.1.1/§6.2 SDP rtpmap/fmtp signalling | ✅ ~98% — spiral+diamond ME + GQUANT-from-bitrate + BCH framing + RTP wrap + RFC 3550 §5.1 `RtpPacketizer` (M-bit + seq + ts + SSRC over GOB-aligned payloads) + RFC 3550 §6.4 RTCP SR/RR + §6.5 SDES/CNAME + §6.6 BYE + §6.7 APP application-defined + §6.1 compound packet build/parse (`sender_report()` wired from packetiser packet/octet counts); 45 dB at 64 kbit/s QCIF |
| **MS-MPEG-4** (v1/v2/v3) | 🚧 ~37% — clean-room scaffold; v3 intra 3-tier ESC + custom intra-DC VLC + G0..G3 LMAX/RMAX wired + synthetic-VLC end-to-end + v1/v2 CBPY VLC binary↔H.263 Table 8 / MPEG-4 Part 2 Table B-6 cross-check + spec/15 §3 (count_A, count_B) provenance-pinned single-source-of-truth table + inter (P-frame) AC residual decode (G4 VLC → dequant → IDCT → add-to-MC) (330 tests); still lacks G0..G3 primary canonical-Huffman bit-length array (spec/99 §10 OPEN) + alt-MV VLC re-extract. VfW-sandboxed mpg4c32.dll runs in parallel | — |
| **H.264** | 🚧 ~80% — I/P/B + 4:2:0/4:2:2/4:4:4 + CAVLC + CABAC + DPB + 40 SEI types + fuzz-hardened; lacks MBAFF, SVC/3D/MVC | 🚧 ~83% — I+P (1MV/4MV, ¼-pel) + B + CABAC at all chroma layouts + Trellis-quant RDOQ-lite + r148 opt-in CABAC IDR Intra_16x16 luma AC trellis + r151 opt-in chroma AC trellis (4:2:0 + 4:4:4, skip_dc keeps Hadamard chains bit-exact; 1227 tests); ffmpeg PSNR_Y 44.20 dB |
| **H.265 (HEVC)** | 🚧 ~38% (post-2026-05-18 orphan) — VPS+SPS+PPS bodies + scaling-list + scan + §9.3 CABAC engine + slice header through r147 num_ref_idx prelude + r150 §7.3.6.1 inter mvd_l1_zero_flag/cabac_init_flag/collocated_from_l0/collocated_ref_idx block (no-RPLM path; 195 tests); lacks §9.3.4.2 binarization+ctxIdx (#444), §7.4.8 inter-RPS-prediction, residual/IDCT | 🚧 scaffold |
| **H.266 (VVC)** | 🚧 ~64% — 4:2:0 IDR intra + ALF/SAO/CC-ALF + P/B merge+skip + HMVP + MMVD + CIIP + BCW + BDOF + GPM + AMVR + HBD + chroma 4-tap sub-pel + DMVR §8.5.3.2.4 + affine sub-block MC §8.5.5.9 + PROF §8.5.6.4 + §8.5.5.5/§8.5.5.6 affine merge candidates + §8.5.5.2 subblockMergeCandList insertion order + merge_subblock_idx pick + §8.5.5.2 steps 3-6 neighbour/corner-selection cascade (inherited-A/B scans + constructed-corner availability under parallel-merge gate) + §7.3.10.10 mvd_coding() + §9.3.3.14 limited-EGk + §7.3.10.8 non-merge inter MVP-side syntax (inter_pred_idc / sym_mvd_flag / ref_idx_lX / mvp_lX_flag) + §8.5.2.8/§8.5.2.9/§8.5.2.10 AMVP luma candidate derivation (spatial A/B scan with DiffPicOrderCnt==0 gate + §8.5.2.14 AMVR round + Col gate + HMVP fill + zero-pad + mvp_lX_flag select + mvd fold) + §8.5.2.11 live temporal-Col AMVP candidate (POC-scaled, AMVR-rounded) + §8.5.5.7 affine AMVP candidate list (luma CPMV predictors: §8.5.5.5 inherited-A/B scans + §8.5.5.6 corner-constructed cascade + AMVR round + zero pad to max-num=2) + §7.3.10.5 bcw_idx CABAC reader (Table 91 TR cMax = NoBackwardPredFlag ? 4 : 2) + §7.3.10.5 bcw_idx gate evaluator + MvField fuse (BcwIdxGate seven-input conditional + inference on closed gate) + §8.5.5.3/4 SbTMVP record + availability gate + §8.5.5.3 SbTMVP CTU-walker fuse (per-8×8-sub-block mvLXSbCol/predFlag fill from collocated motion field + centre-default fallback) + §7.3.11.7 merge_subblock_flag + merge_subblock_idx CABAC readers (Table 107/108 ctx; merge-side cond gating; live-stream entry into the SbTMVP / sub-block-merge-cand-list driver) + §7.4.3.4 eq.85 MaxNumSubblockMergeCand SPS derivation (affine + sbtmvp branches, [0,5] clamp; drives the merge_subblock_idx cMax) + r146 §7.3.11.7 `merge_data()` wire-up of the merge_subblock_flag/idx readers behind the eq.85 gate (`MaxNumSubblockMergeCand > 0 && cbW ≥ 8 && cbH ≥ 8`; subblock-merge branch short-circuits regular/MMVD/CIIP/GPM per §7.4.12.7 inferences); 992 tests; lacks non-merge inter CU walk + live per-CB merge_subblock/inter_affine neighbour grid + encoder-side subblock-merge emission | 🚧 ~85% — forward CABAC + DCT-II + SAO/ALF/cu_qp_delta + MTT BT+TT RDO + P+B slice + sub-pel MC ½/¼-pel (luma + chroma) + multi-ref DPB + weighted bi-pred — see crate README |
| **VP6** | 🚧 r7 (post-2026-05-18 orphan) — §9 raw-bit frame-header prefix + §15 inverse-quantization + §16 inverse DCT + §17.1 intra-block reconstruction + §11.4 fractional-pixel interpolation filters + §17.2/§17.3/§17.4 inter-block reconstruction + §11.3 4-tap (1,-3,3,-1) deblocking filter + §11.5 UMV border extension (48-px edge-replication, horizontal-then-vertical) + §12.1 default zig-zag scan order + §14 DC prediction (per-reference-bucket Last-DC + four-row predictor table) + §10 mode tables + ModeDecisionTree builder + §13 DCT-token static tables & conversions (banks/trees/Huffman-prob + DcNodeContexts; traversal BoolCoder-gated) + §7.2 Huffman tree construction + decode walk (R(1)-orthogonal to §7.3 BoolCoder gap — VP6_CreateHuffmanTree + VP6_HuffmanDecodeSymbol); 257 tests; §7.3 BoolCoder b(n) blocked (#930) | 🚧 scaffold |
| **VP8** | ✅ 100% — RFC 6386 key+inter decode, bit-exact vs vpx/ffmpeg on 10+ multi-frame fixtures (incl. ARNR, golden refresh, SPLITMV recon) | 🚧 ~76% — Phase-2 I+P encoder + SPLITMV + GOLDEN/ALTREF + multi-partition inter + §9.7/§9.8 RefreshControls + r151 §9.4 caller-driven LoopFilterDeltas (per-ref/per-mode mb_lf_adjustments + across-frame carry; effective deltas used by §15 post-walk filter; 494 tests); lacks encoder-decoder pixel lockstep on non-zero loop_filter_level P-frames (#1140) |
| **VP9** | 🚧 ~35% (post-2026-05-20 orphan) — §6.2 walk + §9.2 Bool decoder + §6.3 compressed-header sweeps + §6.4.24 coeff + §8.6 dequant + §8.7 inverse transforms + §8.5.1 intra pred + §8.6.2 reconstruct + §6.4.3 decode_partition recursive driver + §6.4.13 read_is_inter + r149 §6.3.11 read_is_inter_probs (349 tests); lacks §6.4.4 decode_block + §6.4.11 inter driver + §6.4.16 inter_block_mode_info + §8.4 loop filter | 🚧 scaffold |
| **AV1** | 🚧 ~42% — orphan-rebuild; uncompressed header + msac symbol decoder + §9.4 default-CDF + §8.3.2 ctx selectors + §5.11.49 palette_tokens walker + §9.3 block-size + Partition_Subsize tables + r151 §5.11.4 decode_partition() recursive walker (10 partition arms × 4 edge branches + §6.10.4 MiSizes grid + §8.3.2 partition_ctx; 394 tests); lacks §5.11.5 decode_block body | 🚧 scaffold |
| **Dirac / VC-2** | ✅ ~94% — VC-2 LD+HQ intra + Dirac core-syntax intra/inter + OBMC + 7 wavelets + 10/12-bit + bit-exact intra fixtures + r128 inter-mean rounding fix | 🚧 ~95% — HQ+LD intra + Dirac core-syntax + per-block adaptive sub-pel + 2-ref bipred + post-OBMC mode refinement + r138/r141 picture/sequence rate-control (PerPicture/CBR) + r146 HQ VBV + r149 LD `Vbv { buffer_bytes }` leaky-bucket (peak-caps at target+buffer; degenerates to PerPicture at 0 and CBR at ∞) |
| **AMV video** | 🚧 scaffold (orphan rebuild post-audit 2026-05-18; clean-room re-implementation pending) | 🚧 scaffold (orphan rebuild post-audit 2026-05-18; clean-room re-implementation pending) |
| **ProRes** | ✅ ~96% — RDD 36 entropy + 8/10/12-bit + 4:4:4:4 alpha + interlaced + RAW (aprn/aprh) refused; ffmpeg interop 60-68 dB | ✅ ~95% — RDD 36 across all 6 profiles + interlaced + alpha + perceptual quant matrices + multi-frame rate-control + 12-bit cross-decode + r148 ffmpeg cross-decode acceptance for the r144 perceptual qmat preset (8 cases 58.0-63.8 dB luma PSNR) |
| **EVC** (MPEG-5) | 🚧 ~82% — NAL + SPS/PPS/APS + §9.3 CABAC + §8 intra (Baseline) + DCT-II + P/B inter + RPL + HMVP + DPB + ALF + DRA + IBC §8.6 + r148 §8.9.5 chroma DRA + r151 §7.3.6-faithful dra_data() parser + §7.4.7 derivation (InvLumaScales / DraOffsets / OutRangesL / InDraRange / DraJoinedScaleFlag; 356 tests); lacks §8.9.3 luma inverse mapping wiring + §8.9.6 chromaScale + Main-profile toolset (BTT/ADMVP/EIPD/ATS/AMVR/affine) | — |
| **HuffYUV** / FFVHuff | ✅ ~96% — HFYU + FFVH FourCCs + 6 predictors + 8-bit only + interlaced field-stride=2 + fast-LUT decoder + flat overflow_entries slow path + SWAR 8-byte gradient post-pass (2.18×/2.56× M1) | ✅ ~96% — full encoder symmetry × YUY2/RGB24/RGB32 + v1.x + v2.x ClassicV2/CustomV2 + walking-stride interlaced + predictor auto-selection + r95 SWAR forward-gradient encoder + intermediate-allocation elimination (1.5-1.7× encode speedup on Left/Median 320×240 + 720p Left) + r100 fused LEFT+decorrelation residual + r103 GradientDecorr decorrelation fusion (encoder allocates no decorrelated buffer on any method) + r115 single-pass forward-MEDIAN fusion |
| **Lagarith** | ✅ ~95% — all 11 wire types + modern range coder with spec/02 §5 three-way fast path + legacy adaptive-CDF + Fibonacci-Zeckendorf prefix + JPEG-LS Median + G-pivot decorr + zero-run RLE + pair-packed 513-entry CDF (Strategy F, decode-only for proprietary type-7 streams) + modern RGB(A) first-column predictor Rule B (spec/06 §3.2, byte-exact vs ffmpeg lagarith decoder) | 🚧 ~76% — encoder for SOLID/RGB/RGBA/YV12/YUY2/legacy-RGB + spec/02 §5 Step-A + Step-B + Step-C `freqs[]` cache (1.08× on Step-C-heavy fixtures, 244 MSym/s) + r135 modern-coder q≥1 frequency rescale (>TOP-pixel planes now encodable) + r138 per-channel header-form selection across all 8 wire forms (0x00..0x07 + 0xff; 37% smaller wire on residual profile) + r141 legacy-fork per-channel header-form selection (`encode_legacy_channel_best` + `encode_legacy_rgb_best`; never-worse defensive guarantee — bit-packed Fib layout yields zero 0x00 so RLE escape never fires, selector ties bare-Fib); byte-exact vs proprietary encoder Auditor-blocked |
| **Ut Video** | ✅ ~97% — 5 native FourCCs (ULRG/ULRA/ULY0/ULY2/ULY4) × 4 predictors + RGB inter-plane decorrelation + LUT-accelerated canonical Huffman + word-aligned bit reader + slice-parallel decode (2.87×→5.63× speedup 320×240→1280×720) + spec-pinned `Extradata::ffmpeg_for` builder (encoder_version 0x0100_00f0 + RGB source-format tag per spec/01 §5 / audit/00 §5.2) | ✅ ~96% — codec-internal encoder + slice-parallel encode (1.13×→3.28× speedup, byte-identical to serial path) + spec/01 T1 content-fixture corpus (8 patterns × 4 predictors × 5 FOURCCs at 128×96) + r96 encoder byte-stability suite (deterministic/path-invariant encode + `encode∘decode∘encode` fixed point + 1..256 slice sweep at non-divisible heights) + r101 malformed-payload decode-rejection suite (per-variant guard pinning) + r106 descriptor-mutation rejection + encoder-API-misuse + bit-pack/unpack isolation invariants + r136 daily cargo-fuzz decode harness (~22M exec/60 s, 0 crashes) (141 tests) |
| **MagicYUV** | ✅ 100% — 17 v7 FOURCCs (8 + 10/12/14-bit M0/M2/M4) + Median + JPEG-LS Median (HBD) + raw-mode + interlaced + r130 `decode_into(&mut DecodedFrame)` streaming entry point (skips 4-7 per-frame allocs); trace JSONL strict-jq-line-diff-equal to cleanroom Python ref | ✅ 100% — `encode_frame` across all 17 FOURCCs + spec/04 §3 Dynamic predictor strategy + spec/05 §6.2 Auto Huffman/raw fallback + length-limited Package-Merge Huffman (skewed histograms cap to max_length with Kraft=1) + r127 decoder primary-table packed `Vec<u32>` (5-12% per-pixel speedup, 16 KB→8 KB working set at max_len=18) + r136 daily cargo-fuzz decode harness (~980k exec/60 s, 0 crashes) |
| **Cinepak** (CVID) | ✅ ~98% — frame header + multi-strip + V1/V4 codebooks + intra/inter + grayscale + Sega FILM demuxer + Saturn/3DO deviant + r148 cargo-fuzz harness (4 panic-free targets, 2 strip-header subtract-with-overflow bugs found + fixed) | ✅ ~98% — stateful encoder w/ rolling codebooks + RDO + LBG + 3-axis grid picker + bitrate-target rate control + keyframe-interval enforcement |
| **SVQ1/SVQ3** (Sorenson) | 🚧 r5 (orphan rebuild) — SVQ1 frame-header + framework registry (SVQ1/svqi FourCC) + SVQ3 SEQH + slice + MB-type tree + residual coefficient walker (chroma DC / alt-scan / normal-scan tables); 141 tests; SVQ1 pixel decode blocked on docs (§14.10/§14.11 codebook bytes #429) + SVQ3 MV-VLC table | — |
| **Indeo 3** (IV31/IV32) | 🚧 r8 — clean-room codec-frame header + bitstream header + spec/02 picture-layer plane-prelude parser + spec/03 macroblock-layer binary-tree walk + spec/04 VQ codebook materialisation + spec/06 byte-level entropy (mode-byte classify + jump-table/continuation + RLE escapes + per-position acceptance + FB-counter category) + spec/07 output-reconstruction kernel (predictor + softSIMD dyad add) + spec/07 §2.2 four cell-shape variant inner-loop kernels (A/B/C/D) + spec/02 §4-§7 strip-context array (6-slot dispatchable bank, §4.2 informative width table, §6 per-plane decode-call signature, §7 codec-init strip-count arithmetic) + spec/04 §3.3 outer per-cell row/column loop preamble + spec/03 §5.1/§5.3/§5.5 per-cell sub-array wiring (cell-stack [+0x40..] indexing + per-site zero-disposition predicates + §5.4/§5.5 top-dispatch classifier); 168 tests; lacks cell-stack pre-population (spec/03 §6 Q4) / pixel-buffer edge fix-up loop / MC (spec/05) | — |
| **Indeo 2/4/5** | 🚧 scaffold — pending clean-room workspace; Indeo 4/5 still sandboxed via `oxideav-vfw` | — |

</details>

<details>
<summary><strong>Image</strong> (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **PNG / APNG** | ✅ 100% — 5 colour types × 8/16-bit + APNG + sBIT/pHYs/tIME/bKGD/hIST/eXIf/sRGB/cICP/sPLT round-trip | ✅ 100% |
| **GIF** | ✅ 100% — 87a/89a + LZW + interlaced + animation + disposal compositor + structured Application Extensions (NETSCAPE2.0 / ANIMEXTS1.0 / XMP / ICC / Exif) + Plain Text Extension + lenient-decoder mode + lazy `Playback` + §18.c.viii pixel-aspect-ratio accessors + animation-timing accessors (`frame_delays`/`single_pass_duration`/`total_play_duration` w/ NETSCAPE2.0 loop count) + fluent `AnimationBuilder` (per-frame GCE + NETSCAPE2.0 looping in one chain); clean-room from CompuServe spec + Welch 1984 | ✅ 100% — per-frame palettes + `optimize_color_tables()` GCT/LCT hoisting + §7 Required Version enforcement (rejects 89a-only blocks under Gif87a header) + `upgrade_version_if_needed()` |
| **WebP** (VP8 + VP8L) | ✅ ~95% — RFC 9649 container + VP8L lossless + ALPH + animated; VP8-lossy via sibling crate; `oxideav_core::Decoder` registered | 🚧 ~70% (post-r144 Hat-3 rebuild) — VP8L lossless encoder w/ LZ77 + §4.1 spatial-predictor + §3.5.2/§4.2 color-transform + §5.2.3 color-cache sweep + §3.7.2.1.1 simple code-length + §4.4 color-indexing + r151 §6.2.2 multi-meta-prefix encoder (spec-conformant; chooser keeps single-group baseline on current fixtures, structural plumbing for future per-region clustering); lacks VP8-lossy encode + smarter histogram-distance clustering + 6 r131-r144 pieces (#1120) |
| **JPEG** (still) | ✅ ~95% — via MJPEG | ✅ ~90% — via MJPEG |
| **TIFF** (6.0) | ✅ ~97% — II/MM + BigTIFF read + 6 photometrics + 1/4/8/16-bit + None/PackBits/LZW/Deflate/CCITT-MH/T.4-1D + FillOrder + tiles + multi-page + JPEG-in-TIFF (incl. CMYK-JPEG: Compression=7 + Photometric=5 + SamplesPerPixel=4) + PlanarConfiguration=2 (separate component planes across strips/tiles + chunky re-interleave + Predictor=2 driven per-plane) + cargo-fuzz decoder (panic-free, 7.7 M iter green); lacks CCITT T.4 2-D / T.6 (#874), JPEG-in-TIFF + planar=2 | ✅ Gray8/16/RGB24/Palette8 — None/PackBits/LZW/Deflate + Predictor=2 + PlanarConfiguration=2 separate-planes write (Rgb24 × None/PackBits/LZW/Deflate ± Predictor=2) + Bilevel CCITT-MH / T.4-1D, single+multi-page + tiled chunky write (Gray8/16/RGB24/Palette8 × None/PackBits/LZW/Deflate ± Predictor=2, §15) + tiled PlanarConfiguration=2 write (Rgb24, one grid per plane, §15) |
| **BMP** | ✅ ~96% — 1/4/8/16/24/32-bit + V4/V5 + OS/2 BITMAPCOREHEADER + RLE4/RLE8 + top-down rows + daily fuzz CI (6.5M runs, 0 crashes) | ✅ ~96% — top-down + minimal `biClrUsed`-trimmed palette encoder |
| **Netpbm** (PBM/PGM/PPM/PNM/PAM) | ✅ ~95% — all 8 magics at 1/8/16-bit + 6 PAM TUPLTYPEs | ✅ ~95% |
| **ICO / CUR** | ✅ ~97% — multi-res + BMP/PNG sub-images + CUR hotspot + ICONDIRENTRY validation (bReserved / dwBytesInRes / overlap-with-directory / cross-entry payload-overlap / overflow / wPlanes / wBitCount / CUR hotspot-in-bounds) + `select_best_fit` / `select_largest` / `select_by_dimensions` resolution helpers + 256×256 PNG round-trip + write 1..=256 dimension guard + `.ani` RIFF/ACON detection | ✅ ~92% |
| **JPEG 2000** | 🚧 r12 (post-2026-05-20 orphan) — T.800 main-header + SOT/SOD + typed COC/QCC/POC/RGN/PLT/PPT + JP2 box + §B.10 tier-2 packet-header + §B.2/§B.3/§B.5 TileGeometry + §B.5 typed `ResolutionLevel` (per-r corners) + `SubBand { HL, LH, HH }` (per-r-per-orientation corners per Eq. B-14 / Table B.1) + §B.6 precinct + §B.7 code-block partition (Eq B-16/17/18) + §B.7/§B.9 precinct→code-block enumeration (PacketGeometry bridge) + Annex C §C.3 tier-1 MQ arithmetic decoder (MqDecoder/INITDEC/DECODE/RENORMD/BYTEIN, Table C.2 Qe + Table D.7 contexts) + Annex D §D.3.1 significance-propagation pass + §D.3.2 sign subroutine (t1::CodeBlock, §D.1 stripe scan, Table D.1 contexts per orientation, Table D.2/D.3 sign) + §D.3.3 magnitude-refinement pass (Table D.4 ctx 14-16) + §D.3.4 cleanup pass (all 19 Annex D contexts driven) + §D.3 bit-plane sequencer chaining the three passes per code-block from MSB toward LSB + §B.12.1 all five packet-progression iterators (LRCP/RLCP/RPCL/PCRL/CPRL); + §B.12.2 POC progression-order volume iteration (Eq B-21 + per-(component, resolution, precinct) next-layer cursor across chained volumes); lacks §F wavelet / dequant / MCT | 🚧 scaffold |
| **JPEG XL** | 🚧 ~88% — ISO/IEC 18181-1:2024 lossless Modular path + 7 fixtures pixel-correct + VarDCT scaffold + r141 Gaborish + r144 EPF + r147 AFV basis + r150 §I.2.3.8 Listing I.13 Inverse AFV wired into idct_for_transform (full Table I.4 transform family now pure-math complete; 538 lib tests); lacks §C.7.2 histograms + per-block coeff loop + per-frame Gaborish/EPF/CfL wiring (#799/#1077) | — retired |
| **JPEG XS** | 🚧 ~80% — ISO/IEC 21122 Part-1 + 5/3 DWT + Annex C/D/F/G + multi-component + CAP-bit + Cw>0 + Sd>0 cascade + high bit depth B[i]∈8..16 + r143 Annex A profile/level/sublevel conformance | 🚧 ~80% — Nc 1/3/4 + Sd>0 + RCT + Star-Tetrix + NL up to 8 + odd dims + vertical prediction + significance coding + per-band Q + NLT + Cw>0 + Sd>0∘Cpih cascade + Fs=1 sign sub-packet + multi-slice Hsl + Qpih=1 uniform quantizer + R[p]>0 precinct refinement + high-bit-depth lossless + lossy + r151 4:2:2/4:2:0 sub-sampling at B[i]∈9..16 (288 tests); lacks 4:2:0 chroma at NL,y≥2 (#1139) + Star-Tetrix high-bit-depth + NLT pre-distortion high-bit-depth |
| **AVIF** | 🚧 ~80% — HEIF→AV1 + grid + imir/clap/colr/pixi/pasp + HDR metadata + AV1 wrap pass-through + DoS caps + HEIF item-properties (infe v2/v3 mime/uri tail + thmb/cdsc/prem iref + Exif/XMP item resolver) + auxC URN routing (Alpha / Depth / HDR-gain-map) + rloc / lsel / iovl / grpl parsers + `mif1` compliance audit + a1op/a1lx AV1 layered-image properties + essential-property enforcement + r130 tmap av1-avif §4.2.2 file-shape audit (paired altr group + hidden inputs compliance checker); AV1 pixel decode gated on sibling rebuild | — |
| **DDS** | ✅ ~98% — DDS_HEADER + DXT10 + uncompressed (10 layouts) + BC1-5/7 + BC6H all 14 modes + mipmap + 6-face cubemaps + DX10 arrays + volume (3D) textures + full 132-entry DXGI table | ✅ ~95% — uncompressed + BC1-5 + BC7 all 8 modes (0-7 incl. mode 4/5 channel-rotation; rank-3 multi-axis 30.4 dB; independent-alpha ≥30 dB-RGBA) + BC6H_UF16 all 14 modes + BC6H_SF16 all 14 modes (signed-magnitude pipeline across 1/2-subset signed) + box-downsample-then-encode mip chains + cubemap/array |
| **OpenEXR** | 🚧 ~76% — magic + 8 required attrs + HALF/FLOAT/UINT + NO_COMPRESSION/ZIP/ZIPS/RLE + tiled ONE_LEVEL + sub-sampled chroma + single-part deep scanline + multi-part deep scanline read (`parse_exr_deep_multipart` validated via `exrmultipart -combine`) + r130 single-part deep tiled read (type="deeptile", NONE/RLE/ZIPS); exrmetrics cross-validates; PIZ blocked on docs trace; lacks B44/B44A/DWAA-B | ✅ ~86% — RGBA scanline + ZIP/ZIPS/RLE + tiled-output ONE_LEVEL + tiled-output MIPMAP_LEVELS + tiled-output RIPMAP_LEVELS (2-D reduction grid, NONE/ZIP/ZIPS/RLE) + multi-part scanline + sub-sampled channels + deep scanline write (NONE/RLE/ZIPS) + r130 single-part deep tiled write (type="deeptile"); exrmetrics + exrmultipart + exrinfo + exrheader + exrmaketiled -r cross-validate bit-exact |
| **Farbfeld** | ✅ 100% — streaming reader + DoS hardening (dimension overflow + truncated payload guards) + `magick` black-box cross-validator | ✅ 100% |
| **HDR** (Radiance RGBE) | ✅ ~98% — new-RLE + old-RLE + 8 axis-flag combos + shared-exponent + multi-record EXPOSURE/COLORCORR + typed COLORCORR / PRIMARIES / VIEW headers + apply_exposure / apply_colorcorr helpers | ✅ ~98% — new-RLE + old-RLE + auto-RLE + 8 axis combos + XYZE↔RGB + 8 tonemap ops + CRLF line endings (encode_hdr_with_options) |
| **QOI** | ✅ 100% — byte-exact vs all 8 reference fixtures | ✅ 100% — byte-exact vs reference encoder |
| **TGA** | ✅ 100% — types 1/2/3/9/10/11 + TGA 2.0 extension + thumbnail + developer area + colour-correction table (parse + 16-bit LUT application) + scan-line table + typed AttributesType alpha interpretation (un-premultiply + undefined→opaque); magick cross-validated | ✅ 100% — all six image types + full TGA 2.0 extension (CCT / SCT / developer tags) + thumbnail + RGB24-input entry points |
| **ICER** (JPL) | 🚧 ~75% — Mars-rover heritage; bit-plane scan + compressed/uncompressed segments + 8 filters + IPN 42-155 §III.B context model | ✅ ~80% — quota-controlled encoding (`with_byte_budget` / `with_target_bytes` / `with_rd_budget`) — MSB-down progressive truncation + r5 auto wavelet-filter selection + R-D byte-budget per-segment ranking (IPN 42-155 §IV.B; +6.09 dB checker @ 400 B vs strict-MSB, never regresses) |
| **WBMP** | ✅ 100% — Type 0 + WbmpLimits DoS caps + adversarial fuzz sweep | ✅ 100% |
| **PCX** (ZSoft) | ✅ ~97% — 1/2/4/8 bpp planar + packed-bits + 24 bpp RGB planar + grayscale flag + DCX multi-page + DCX `Demuxer` + r136 fuzz-hardened (40M exec/0 crashes; fixed 398 GB decompression-bomb OOM + width/height integer-underflow) | ✅ ~97% — 8 write paths (incl. grayscale + windowed 24bpp) + DCX; framework `Encoder` accepts Rgba/Rgb24/Gray8 |
| **ILBM** (Amiga IFF) | ✅ ~94% — BMHD/CMAP/CAMG/BODY + ByteRun1 RLE + EHB + HAM6/HAM8 + PBM + SHAM + PCHG + ANIM op-0/op-5 + CRNG/CCRT + DRNG (DPaint IV extended range, true-colour + register cells); lacks ANIM op-7/op-8, DEEP true-colour | ✅ ~84% — IlbmMuxer parity + masking + ANIM op-5 + CRNG/CCRT/DRNG encoder |
| **PICT** (Apple QuickDraw) | ✅ ~97% — v1 + v2 opcode walkers + drawing rasteriser + DirectBitsRect packType 0/1/2/3/4 + Region + clip-region + pen-size aware + Compressed/UncompressedQuickTime opcode skip + monochrome stipple + **PixPat colour 8×8 type 1** + **dithered PixPat (`patType=2`)** per Inside Macintosh §A-3 / §4 Color QuickDraw (8×8 RGBA tile uniformly painted with `RGBColor`) + `probe_pict`; lacks non-8×8 PixPat tiles, text rasterisation, embedded JPEG decode | ✅ ~93% — `PictBuilder` + every v2 drawing-command family + state opcodes + mono+PixPat pattern setters + DirectBitsRect packType 1/2/3/4 + BitsRgn / PackBitsRgn; magick cross-decode bit-exact |
| **SVG** | ✅ ~99% — full shape set + path + gradients + text + mask + clipPath + use/symbol + svgz + SMIL animate/set/animateTransform + CSS3 Selectors L3 + `@import` + `@font-face` + `@keyframes` + Media Queries L4 + viewBox + 17 filter primitives + CSS Values L4 LengthUnit + CSS Easing L2 + SVG 2 §9.6.1 pathLength + SVG 2 §16.3 `<view>` element + fragment-identifier routing (`#MyView` / `#svgView(...)` + percent-decode + spatial/temporal media-fragment fallthrough) + SVG 2 §5.7 `<switch>` conditional processing (requiredExtensions / systemLanguage) + SVG 2 §13.7.1 `<marker>` typed def capture (refX/refY geometric keywords + markerUnits/orient + verbatim round-trip) + SVG 2 §13.2 `context-fill`/`context-stroke` + SVG 2 §16.5 `<a>` hyperlink (renders as group; link target + HTML attrs preserved across round-trip) + SVG 1.1 §11.5 `display` / `visibility` property handling + SVG 2 §5.8 `<title>` / `<desc>` + §5.9 `<metadata>` capture (multilingual lang, round-trip via PreservedExtras) | ✅ ~88% — round-trips full shape graph + PreservedExtras side-channel + `<view>` re-emit at trailing edge |
| **PDF** | ✅ ~99% — bytes → Scene via xref/xref-streams/ObjStm + `/Prev` incremental + `/Encrypt` R=2..6 + public-key + PKCS#7 + `/Sig` AcroForm + Doc-Timestamp + text extraction + Linearization + Tagged-PDF + EmbeddedFiles + §12.6 actions + 5 stream filters + §8.11 Optional Content + content-stream cs/CS + §7.5.8.4 hybrid-reference + r145 cargo-fuzz + r148 criterion benches + r151 §7.5.7 ObjStm resolver cache (3.10 → 54.6 MiB/s, 17.6×) | ✅ ~99% — PDF 1.4/1.5 multi-page + paths/gradients/opacity/clip + RGBA + xref-stream + ObjStm + Linearization writer + `/Encrypt` + public-key + `/Sig` + AcroForm + annotation writer + embedded files + RFC 3161 Document Time-Stamp writer |

</details>

<details>
<summary><strong>3D scenes & assets</strong> (click to expand)</summary>

> The typed Scene3D / Mesh / Material PBR / Skin / Animation / Camera / Light / AudioEmitter model lives in `oxideav-mesh3d`, with `Mesh3DDecoder` / `Mesh3DEncoder` traits and a `Mesh3DRegistry` that's parallel to `oxideav-core::CodecRegistry`. Per-format crates register into it. `oxideav-meta::populate_mesh3d_registry(&mut Mesh3DRegistry)` walks every enabled format's `register()`. Lazy bytes flow through `AssetSource` (with a `raw_storage` pass-through hook for archive-backed sources, e.g. ZIP-stored USDZ textures + audio).

| Format | Decode | Encode |
|--------|--------|--------|
| **STL** (ASCII + binary) | ✅ ~99% — both formats + per-face attributes + 16-bit colour + multi-`solid` ASCII + topology (Euler χ) + repair pipeline (weld + degenerate-cull + zero-normal recompute + orient-from-winding + unit-normal renormalise + consistent-winding + ascending-z facet sort) + ASCII comment preservation | ✅ ~99% — both formats + attribute pass-through + `EncodeStats` + configurable float precision + spec-style scientific ASCII formatter (`1.23456E+789` form) |
| **OBJ** (+ MTL) | ✅ ~97% — full Wavefront grammar + MTL (Phong + Wavefront-PBR + map_* options + typed refl) + smoothing/display attrs + free-form geometry pass-through + `xyzrgb` per-vertex colour + Bezier + B-spline / NURBS / Cardinal (Catmull-Rom) / Taylor `curv` + Bezier + B-spline / NURBS / Cardinal (Catmull-Rom) `surf` 2D-surface tessellation (tensor-product de Casteljau / Cox-deBoor); lacks Taylor / basis-matrix `surf` surfaces, multi-patch decomposition, trim/hole loops | ✅ ~96% — symmetric + negative-index encoder + polyline rejoin |
| **glTF 2.0** (+ .glb) | ✅ ~92% — JSON + .glb + full PBR + KHR_lights_punctual + KHR_materials_unlit + KHR_materials_emissive_strength + KHR_materials_ior + KHR_materials_specular + KHR_materials_clearcoat + KHR_materials_sheen + KHR_materials_transmission (decode + encode + §3.12 validator) + KHR_materials_volume (decode + encode + thickness/attenuation §3.8 validator) + KHR_materials_iridescence (decode + encode + §3.12 validator) + skin + skeletal animation (LINEAR/STEP/CUBICSPLINE) + sparse accessors + morph-targets + 12 spec-MUST validators + KHR_texture_transform (per-textureInfo UV affine, decode + encode + §3.12 validator) + JSON fuzz hardening; lacks KHR_audio_emitter / KHR_materials_anisotropy/dispersion | ✅ ~91% — symmetric + sparse-encoding heuristic + signed+unsigned normalised-int quantisation + KHR_materials_unlit emit |
| **USDZ** (+ USDA) | ✅ ~92% — ZIP STORED walker + USDA parser + UsdGeomMesh + UsdPreviewSurface PBR + UsdUVTexture pass-through + xformOp transforms + UsdMediaSpatialAudio + variantSet + LIVRPS variant-selection composition + composition-arc round-trip + in-archive sublayer + references/payload arc composition (LayerStack); lacks `.usdc` binary (#754), UsdSkel*, UsdGeomSubset | ✅ ~88% — symmetric writer + zero-re-encode pass-through + variant writer + composition-arc writer |
| **FBX** | 🚧 ~66% — binary container (32/64-bit) + object-graph + mesh + animation (TRS+DeformPercent) + deformers (Skin / Cluster / BlendShape) + Material / Texture / Video surfacing via Connections walker (embedded Video.Content R-blobs + OP typed PBR routing for DiffuseColor / NormalMap / EmissiveColor / metallic / occlusion) + bind pose (Pose/BindPose → node extras + skeleton inverse-bind refine). Lacks: ASCII FBX (#785), Properties70 P-record grammar + Light/Camera NodeAttribute | ✅ ~58% — symmetric binary writer + opt-in zlib deflate; Blender/ufbx-readable round-trip |
| **Alembic** | 🚧 0% — Sphinx API reference + Python examples staged at `docs/3d/alembic/`; on-disk Ogawa binary needs Wayback PDF recovery (Imageworks 2010-2012 manuals 404 today) or commissioned trace | — |

Cross-format integration: `oxideav-cli-convert` exposes a 3D conversion path through `oxideav_meta::populate_mesh3d_registry` — `oxideav convert in.obj out.gltf` (or `--probe` for structural inspection). `crates/oxideav-tests/tests/mesh3d_*.rs` runs the cross-format roundtrip suite. Convert verb has accumulated IM-compatible ops including `-resize` / `-thumbnail` / `-define`, USDZ encoder + 3D→raster renderer (Gouraud + Phong + `-light` / `-camera` / `-projection` / `-fov` / `-bg`), `-render normal-debug|depth-debug` + `-aa N` supersampling, and multi-size ICO via `-define icon:auto-resize`. Black-box oracles in `tests/mesh3d_{usdz_apple,blender_assimp}_oracle.rs` cross-validate against Apple `usdzconvert` + Blender + assimp.

</details>

<details>
<summary><strong>Trackers</strong> (decode-only by design) (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **MOD** | ✅ ~97% — 4-channel Paula-style mixer + full ProTracker 1.1B effect set + FT-extension `8xx` / `E8x` per-channel pan + XM E3x glissando control + Lxy set-envelope-position + E4x/E7x vibrato/tremolo waveform shapes (sine/saw/square) (FT2 §); PT-fidelity rounds for loop boundary / LED filter / extended period range / EE pattern-delay + 9xx out-of-range no-note quirk; 107 unit + 39 integration tests | — |
| **STM** (Scream Tracker v1) | ✅ ~85% — structural parse + shared-mixer playback; XM-parity effects (Gxy/Jxy/Bxy/Cxy/Exy/Hxy + 7xy tremolo + volume-slide variants); hard-pan LRRL | — |
| **XM** (FastTracker 2) | ✅ ~90% — structural parse + full playback; envelopes + fadeout + key-off; vibrato + tone porta + pattern jumps + fine/extra-fine porta + Exy/Kxy subcommands + volume-column slides | — |
| **S3M** | ✅ ~94% — stereo + full ST3 v3.20 effect set + per-channel effect memory ("%") for D/E/F/H/I/J/K/L/O/Q/R/S/U + Dxy multimedia.cx case matrix (DFF fine-up, D0F/DF0 every-tick) + S3x/S4x bit-2 waveform retention + Qxy persistent-counter retrigger (cross-row cadence, tick-0 capable, exact TwoThirds ×2/3) + Cxx row-≥64 ignore + Kxy/Lxy continue running vibrato/porta from H/G effect-memory (`H00`/`G00 + Dxy` per multimedia.cx); lacks AdLib FM synth | — |

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
| **`oxideav-videotoolbox`** | macOS (Apple Silicon + Intel Macs) | 🚧 H.264 + HEVC + ProRes + MJPEG + MPEG-2 | 🚧 H.264 + HEVC + ProRes + MJPEG | Roadmap: VP9 / AV1 / MPEG-4 Pt 2 (round 5). MPEG-2 decode-only ~61 dB PSNR-Y. H.264 ~51 dB PSNR-Y, HEVC ~54 dB, ProRes ~52 dB, MJPEG ~36 dB. AV1 hardware needs M3+. |
| **`oxideav-audiotoolbox`** | macOS | 🚧 AAC LC + HE-AAC v1/v2 + AAC-LD/ELD + ALAC | 🚧 AAC LC + HE-AAC v1/v2 + AAC-LD/ELD + ALAC | AAC LC 36.7 dB @ 128 kbit/s; HE-AAC v1 ~11 dB @ 64 kbit/s stereo; HE-AAC v2 ~10 dB @ 32 kbit/s stereo (PS requires stereo); ALAC bit-exact 190,464 / 192,000 samples. Roadmap: FLAC, Opus, AMR-NB/WB, iLBC. |
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
| **`oxideav-source`** | URI resolution + file reader + prefetching BufferedSource | ✅ `file://` + `mem://` + `data:` (RFC 2397 inline base64/percent) + `concat:` (`|`-separated `file://` segments → one seekable stream) drivers + `FileScope` allow-list policy; generic `SourceRegistry` for pluggable schemes |
| **`oxideav-http`** | HTTP / HTTPS source driver | ✅ `http://` + `https://` via pure-Rust `ureq` + `rustls` + `webpki-roots`; Range-request seeking; `HttpConfig` policy layer (timeouts, redirect cap, custom headers) |
| **`oxideav-generator`** | Synthetic media source (`generate://...` URIs) + zero-input filters | ✅ audio synth (sine + chirp/FM/DTMF/multitone/ADSR/ringmod) + image (xc/gradient/pattern/fractal/plasma/noise/label) + video (testsrc/smptebars/fractal_zoom/gradient_animate/zoneplate); ImageMagick/sox shorthands in `convert` verb (vector text → raster via scribe + raster) |
| **`oxideav-rtmp`** | RTMP ingest + push | ✅ Server accepts incoming publishers + client pushes to remote servers; AMF0 handshake / chunk stream demux + **AMF3 wire-format parser + builder per Adobe AMF3 §3.1 / §1.3.1 / §2.2** + AMF3 `onMetaData`/data (type 15) + AMF3 command (type 17) routed end-to-end (bridged onto AMF0 so metadata surfaces through one path); Enhanced-RTMP v1 video (HEVC/AV1/VP9/AVC FourCC) + v2 audio (Opus/FLAC/AC-3/E-AC-3/MP3/AAC FourCC) + E-RTMP v2 ModEx packet-type prelude (audio + video); pluggable key-verification hook; `rtmp://` registered as a `PacketSource` on `SourceRegistry` |
| **`oxideav-sysaudio`** | Native audio output | ✅ Runtime-loaded backends (ALSA, PulseAudio, WASAPI, CoreAudio); no C build-time linkage. CoreAudio + WASAPI backends report **real HAL latency** — CoreAudio sums `kAudioDevicePropertyLatency` + `BufferFrameSize` + `SafetyOffset` + `kAudioStreamPropertyLatency`; WASAPI reads `IAudioClock`-derived presentation latency. Output-device enumeration (names + default flag) across WASAPI / ALSA / CoreAudio. BT-aware; falls back to software estimate if HAL unavailable. |
| **`oxideav-pipeline`** | Pipeline composition (source → transforms → sink) | ✅ JSON transcode-graph executor; pipelined multithreaded runtime + `Executor::with_channel_caps(ChannelCaps { packets, frames })` configurable per-track depth (embedded `{1,1}` → offline `{64,32}`) + `Executor::with_max_queue_bytes(n)` orthogonal byte-ceiling on the demux→worker queues (composes with the count caps) |
| **`oxideav-scene`** | Time-based scene / composition model | 🚧 Scaffold — data model for PDF pages / RTMP streaming compositor / NLE timelines; renderer still stubbed |
| **`oxideav-audio-filter`** | Audio effects & conversions (streaming) | ✅ ~46 filters: classic + transient/spatial/restoration family + MidSide / EnvelopeFollower / DeEsser / Wah / OctaveDoubler / AdaptiveNoiseGate + Exciter / MultibandCompressor / StereoImager / Talkbox + TransientDesigner / Ducker / GainNormalizer / FreqShifter + HardClipper + r106 SlewLimiter (per-sample rate-of-change cap) — see crate README for the catalogue |
| **`oxideav-image-filter`** | Single-frame image effects (stateless) | ✅ 126 filter types / 161 factory names (r105 added Scharr 3×3 first-derivative edge operator — `±3 ±10 ±3` weights, lowest orientation error of the 3×3 family; r101 added Prewitt + PrewittMagnitude L1/L2; r24 added Roberts cross 2×2; r22 added Reinhard / Hable / Drago tone-mapping + Curves with monotone-cubic interpolation + Borgefors distance transform + Cyanotype) — see crate README for the catalogue |
| **`oxideav-pixfmt`** | Pixel-format conversion + palette + dither | ✅ YUV↔RGB matrices (BT.601 / BT.709 / BT.2020 / BT.2100), chroma subsampling, palette quantisation (median-cut / k-means), Floyd-Steinberg dither, PQ + HLG + BT.1886 transfer functions |

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
| **TTML**            | ✅ | ✅ | W3C Timed Text, `<tt>/<head>/<styling>/<style>/<p>/<span>/<br/>`, tts:* styling |
| **SAMI**            | ✅ | ✅ | Microsoft, `<SYNC Start=ms>` + `<STYLE>` CSS classes |
| **EBU STL**         | ✅ | ✅ | ISO/IEC 18041 binary GSI+TTI (text mode only; bitmap + colour variants deferred) |

**Advanced text (own crate)** — `oxideav-ass`:

| Format              | Decode | Encode | Notes |
|---------------------|:------:|:------:|-------|
| **ASS / SSA**       | ✅ | ✅ | Script Info + V4+/V4 Styles (BGR+inv-alpha) + override tags (b/i/u/s/c/fn/fs/pos/an/k/kf/ko/K/N/n/h). Typed `\pos`/`\fad`/`\fade`/`\move`/`\t`/`\frz`/`\frx`/`\fry`/`\org`/`\blur`/`\be`/`\bord`/`\xbord`/`\ybord`/`\shad`/`\xshad`/`\yshad`/`\fax`/`\fay`/`\fscx`/`\fscy`/`\clip`/`\iclip`/`\an`/`\a`/`\k`/`\kf`/`\ko` (numpad + legacy line-alignment + karaoke timing) extraction + time-evaluation via `extract_cue_animation` → `RenderState`; `[Aegisub Project Garbage]` + `[Fonts]`/`[Graphics]` round-trip via extradata |

**Bitmap-native (own crate)** — `oxideav-sub-image`:

| Format              | Decode | Encode | Notes |
|---------------------|:------:|:------:|-------|
| **PGS / HDMV** (`.sup`) | ✅ | — | Blu-ray subtitle stream; PCS/WDS/PDS/ODS + RLE + YCbCr palette → RGBA |
| **DVB subtitles**   | ✅ | — | ETSI EN 300 743 segments + 2/4/8-bit pixel-coded objects |
| **VobSub** (`.idx`+`.sub`) | ✅ | — | DVD SPU with control commands + RLE + 16-colour palette |

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
and per-frame unsync, extended header, v2.4 data-length indicator,
encrypted/compressed frames recorded as `Unknown`) plus the legacy
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
