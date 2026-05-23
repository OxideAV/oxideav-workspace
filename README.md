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
  (vector→raster rendering kernel — scanline AA, bilinear/Lanczos2,
  trapezoidal coverage, soft masks, patterns, filter primitives, ICC
  pipeline, bitmap cache keyed by `Group::cache_key`), `oxideav-ttf`
  (TrueType parser — cmap 0/4/6/12/14 incl. Variation Sequences, GSUB
  ligatures, GPOS kerning, COLR + CPAL + sbix tables, TTC subfont
  selection, AGL glyph-name→Unicode), `oxideav-otf` (CFF / Type 2 charstrings incl. CID-keyed ROS/FDArray/FDSelect + arithmetic/stack/storage/conditional ops, cubic outlines),
  `oxideav-scribe` (shaper with vector-first `Shaper::shape_to_paths`
  API — no rasterizer dep; trapezoidal horizontal AA, GPOS mark-to-mark,
  COLR/CBDT colour glyphs via raster bilinear/composer; bidi UAX #9 +
  USE still future work).
- **3D scenes & assets** — typed `oxideav-mesh3d` (Scene3D / Mesh /
  Material PBR / Skin / Animation / Camera / Light / AudioEmitter +
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
| Ogg       | ✅ | ✅ | ✅ | Vorbis/Opus/Theora/Speex pages + comments; page-granule bisection + page-level seek index (`open_indexed`); chained-link-aware duration (RFC 3533 §4); page-loss/hole detection via `page_sequence_number` (RFC 3533 §6, `hole_count()`) |
| Matroska  | ✅ | ✅ | ✅ | MKV/MKA/MKS; DocType-aware probe; Cues seek; SeekHead emit; Chapters round-trip; Attachments + subtitle tracks; RFC 9559 §5.1.8 typed Tag decoder + RFC 9559 §5.1.4.5.5 / §10.3 opt-in block lacing on write (`MkvMuxer::with_block_lacing(LacingMode { None, Xiph, Ebml, FixedSize })`) + EBML CRC-32 validation on Top-Level masters (`crc_status()`) + TrackOperation typed decode (§5.1.4.1.30 3D plane-combine / block-join, UID→stream-index resolved) |
| WebM      | ✅ | ✅ | ✅ | First-class: separate fourcc, codec whitelist (VP8/VP9/AV1/Vorbis/Opus); inherits Matroska Cues seek |
| MP4       | ✅ | ✅ | ✅ | mp4/ismv brands; faststart; iTunes ilst; fragmented demux + mux (DASH/HLS/CMAF) + sidx/mfra/tfra; AC-3/E-AC-3/DTS sample-entry FourCCs; subtitle/timed-text demux (tx3g/wvtt/stpp/sbtt/stxt/c608/c708); §8.12 protected sample-entry unwrap (sinf/frma/schm); §8.3.3 typed track references + §8.9 sample-groups (sbgp/sgpd) with typed roll/prol/rap (AAC + Opus pre-roll, open-GOP) + edts/elst edit-list mux (§8.6.5–6 positive start delay) + §8.4.6 extended-language tag (elng) demux → `params.options["language"]` (BCP 47); lacks CENC decryption (tenc/pssh/senc) |
| MOV (QuickTime) | ✅ | — | ✅ | Native `oxideav-mov` — Apple QTFF + ISO BMFF meta + HEIF/HEIC item-properties + derived images grid/iovl/iden/tmap + 29-variant BrandClass + Movie Fragment decode + symmetric muxer + fragmented-MP4 seek + r74 typed edit-list mapper + r91 non-unity `media_rate` scaling + r95 §8.10.3 Track Selection box (`tsel`) typed surface (switch_group + 14 §8.10.3.5 attribute roles + `MovDemuxer::switch_groups()` ranking) + r98 §8.6.4 `sdtp` Independent & Disposable Samples box + r102 §8.6.3 Shadow Sync Sample box (`stsh`, `shadow_sync_sample`); ffprobe-accepted |
| AVI       | ✅ | ✅ | ✅ | OpenDML 2.0 super-index + AVIX + dmlh + vprp + 2-field interlaced + truncated-head recovery + VBR audio + LIST INFO + typed `PaletteChange`/`TextChunk`/`AvihFlags`/`Idx1Flags` + opt-in idx1↔ix## synthesise + WAVE_FORMAT_* + per-stream budget + ODML keyframe seek + top-down DIB + BI_BITFIELDS + WAVEFORMATEXTENSIBLE 0xFFFE + `strn` name + `strd` codec-driver + `avih.dwPaddingGranularity` round-trip + stream-aligned `JUNK` packet emission (`with_padding_granularity(n)`) + CBR-audio `ix##` standard-index block-alignment validator (`cbr_audio_block_alignment_violations()`) + OpenDML super-index `dwDuration` round-trip + reader-side `dmlh.dwTotalFrames` consistency cross-check + `vprp` typed frame-aspect-ratio accessor |
| Blu-ray (BD-ROM) | ✅ | — | — | `oxideav-bluray` Phase 2 — UDF 2.50 mount (ECMA-167 3rd ed.) + BDMV walk (`index.bdmv`/`MovieObject.bdmv`/`.mpls`/`.clpi`) + `.m2ts` stream (192→188-byte TP_extra_header strip) + `bluray://` URI handler with auto-detect; r93 typed `Cpi { ep_map: Vec<EpMap { stream_pid, ep_stream_type, entries: Vec<EpEntry { pts_ep_start, spn_ep_start, is_angle_change_point, … }> }> }` CPI EP_map decode per BD-ROM AV §5.7 (coarse + fine two-level table folded into a flat per-PID list a seeker can binary-search); r96 keyframe-aligned `TitleSource::seek_to(pts_90k)` (PTS→clip→I-frame→SPN×192, AACS-unit-aligned); `StreamDecryptor` trait hooks `oxideav-aacs` without hard dep. Lacks HDMV opcode exec, BD-J, multi-angle EP_map seek, cross-PlayItem STC PTS remap |
| DVD-Video | ✅ | — | — | `oxideav-dvd` Phase 3b — ISO 9660 + UDF 1.02 mount + VIDEO_TS walk + IFO body parser (VMGI/VTSI + TT_SRPT + VTS_PTT_SRPT + PGCI [+ PGC subpicture colour-LUT + pre/post/cell nav command table] + VTS_C_ADT + chapter materialiser) + VOB demux (MPEG-PS pack/PES + Nav-Pack PCI/DSI [+ PCI highlight: HLI_GI/SL_COLI/BTN_IT menu buttons] + DVD substream router for AC-3/DTS/LPCM/subpicture) + VOB → MKV mux (`mkv-output` feature; per-PES PTS preserved + ChapterAtom per `DvdChapter` via RFC 9559 §5.1.7) + `dvd://` URI handler. Lacks VM (HDMV opcodes + SPRMs/GPRMs), CSS auth (Phase 3c + `oxideav-css`) |
| MP3       | ✅ | ✅ | ✅ | ID3v2/v1 tags + cover art, Xing/VBRI TOC seek (+ CBR fallback), frame sync with mid-stream resync |
| IFF / 8SVX| ✅ | ✅ | — | Amiga IFF with NAME/AUTH/ANNO/CHRS |
| IVF       | ✅ | — | — | VP8 elementary stream container |
| AMV       | ✅ | — | — | Chinese MP4 player format (RIFF-like) |
| FLV       | ✅ | — | — | Flash Video — MP3/AAC/H.264 audio + VP6f/VP6a/H.264 video + Enhanced RTMP ExVideoTagHeader (AV1/VP9/VP8/HEVC/VVC + AVC FourCC, SequenceStart→extradata, HEVC SI24 CTO, Multitrack) + AMF0 onMetaData/onXMPData/onCuePoint + Annex F encryption headline (v1/v2) + FrameType 5 command tags + typed E-FLV ModEx walk + Enhanced-RTMP `VideoCommand` UI8 on Ex video path (StartSeek/EndSeek per spec) + E-FLV multitrack body splitter (per-track split + default-track routing) + E-FLV VideoPacketType.Metadata HDR colorInfo → `colorinfo.*` metadata (BT.2020 / hdrCll / hdrMdcv); seek_to via keyframes |
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
| **FLAC** | ✅ 100% — bit-exact vs spec + RFC 9639 §8.7 CUESHEET tracks → Chapter API | ✅ 100% — bit-exact roundtrip + block-size-adaptive LPC coefficient precision (≤15-bit) |
| **Vorbis** | 🚧 r8 (post-2026-05-20 orphan) — identification + comment + §3.2.1 codebook + Huffman tree + full §4.2.4 setup-header walker + §3.2.1/§3.3 VQ vector unpack (entry → vector via multiplicands + minimum_value + delta_value + sequence_p) + §8.6 residue decode (formats 0/1/2) + §7.2.3/§7.2.4 floor type 1 packet decode + curve computation; lacks floor 0 runtime / §4.3.2 audio-packet driver / MDCT+TDAC | 🚧 scaffold |
| **Opus** | 🚧 r8 (post-2026-05-20 orphan) — RFC 6716 §3.1 TOC + §3.2 frame packing + §4.1 range decoder + §4.2.7.1–§4.2.7.5.1 SILK header + §4.2.7.4 subframe gains + §4.2.7.5.2 LSF Stage-2 residual + §4.2.7.5.3 NLSF reconstruction (Tables 23/24 cb1_Q8 + IHMW w_Q9 in [1819, 5227] + final NLSF_Q15 clamp) + §4.2.7.5.4 NLSF stabilization (RFC 8251 §7 erratum) + §4.2.7.5.5 NLSF interpolation; lacks §4.2.7.5.6 LSF→LPC + LTP + excitation + CELT | 🚧 scaffold |
| **MP1** | ✅ 100% — now CRC-verifying when `protection_bit==0` | ✅ ~97% — CBR + psy-driven VBR + opt-in CRC-16 protection (§2.4.3.1) + joint-stereo encode (M/S upper-band sharing, §2.4.2.4, ffmpeg-interop-confirmed); lacks full §C.1 psymodel |
| **MP2** | 🚧 scaffold (orphan rebuild post-audit 2026-05-24) | 🚧 scaffold — register-only stub; prior decode+encode erased (bit-allocation/synthesis tables had external-library provenance); clean-room re-build pending vs ISO/IEC 11172-3/13818-3 |
| **MP3** | ✅ ~96% — MPEG-1 Layer III M/S + free-format streams (§2.4.2.3; measure_free_format_size scan + CBR-fast seek + lazy-scan seek) + Xing/Info encoder-version surfaced as `encoder` metadata | 🚧 ~90% — CBR + VBR + M/S + intensity + Annex D Psy-1 + per-region big-value Huffman + per-granule count1 picker + demand-weighted CBR bit allocator + per-band scalefactor noise shaping (§2.4.3.4/§C.1.5.4) + scfsi reuse (§2.4.2.7) on MPEG-1 long blocks |
| **AAC** | 🚧 ~87% — LC + HE-AACv1 SBR + HE-AACv2 PS + LATM + PCE + fuzz-hardened bounds + gapless `iTunSMPB` + §4.6.18.5 SBR boundary upsample + AAC-LD (AOT 23) `er_raw_data_block` bootstrap for channelConfiguration 1 (SCE) and 2 (CPE common-window + independent-ICS) at 512/480-sample frame lengths + multi-RDB ADTS decode (1..4 raw_data_blocks/frame, CRC + no-CRC) + AAC-LD long-term prediction decode (§4.6.7 ltp_data + single-tap predictor) + non-LD AAC-LTP decoder tools (AOT 4, Table 4.49 non-LD ltp_data + long-window predictor M=0/N=2048); lacks multichannel LD, AOT 4 full SCE/CPE wiring, short-window LTP, low-overlap window, ELD, USAC | 🚧 ~84% — LC + HE-AACv1/v2 + PNS + 5.1/7.1 + Bark psy + TNS (CPE + SCE) + perceptual M/S decision §6.6.1.3 with Johnston binaural masking + PE-based VETO/PROMOTE gates |
| **CELT** | 🚧 r5 (post-2026-05-20 orphan) — RFC 6716 §4.1 range decoder + §4.3 prefix + §4.3.2.1 coarse-energy scaffold + §4.3.3 bit-allocation fields (Table 58 trim PDF + skip / intensity-uniform / dual gated decode); blocked on docs #936 (Laplace) + #943 (cache_caps50 / LOG2_FRAC_TABLE / alloc loop) | 🚧 scaffold |
| **Speex** | 🚧 r2 (post-2026-05-19 orphan) — Speex Codec Manual §7.3 Table 7.1 Ogg stream-header parser (NB/WB/UWB) + narrowband frame-header (1-bit WB flag + 4-bit mode ID) + Table 9.1 NB sub-mode budgets (CELP 0..=8 + §5.5 reserved 13/14/15) + MSB-first `BitReader`; lacks CELP frame body + LSP-VQ + pitch/innovation codebooks | 🚧 scaffold |
| **GSM 06.10** | ✅ 100% | ✅ 100% — incl. WAV-49 |
| **G.711** (μ/A-law) | ✅ 100% | ✅ 100% |
| **G.722** | ✅ 100% | ✅ 100% |
| **G.723.1** | ✅ 100% | ✅ 100% — both 5.3k + 6.3k |
| **G.728** | ✅ 100% — LD-CELP 50-order | ✅ 100% |
| **G.729** | 🚧 ~78% — §4.2 post-processing spec-exact (100 Hz output HPF + ×2 restore, L1-norm AGC, tilt 1/g_t) + predictor pipeline + LSP-VQ spec-exact; lacks bit-exact gbk1/gbk2 numerics (ITU electronic-attachment gap #859) | 🚧 ~78% — LP-analysis window now spec-exact (240-sample asymmetric per §3.2.1 + 5 ms look-ahead); gain-VQ per §3.9 eq 72; lacks bit-exact gbk1/gbk2 numerics (ITU electronic-attachment gap) |
| **IMA-ADPCM (AMV)** | ✅ 100% | ✅ 100% |
| **MS-ADPCM / IMA-ADPCM (WAV)** | ✅ 100% | ✅ 100% — block-aligned WAV encoder for both nibble layouts |
| **8SVX** | ✅ 100% | ✅ 100% |
| **iLBC** (RFC 3951) | ✅ 100% — NB 20/30 ms | ✅ 100% |
| **AC-3 / AC-4** (Dolby Digital / Dolby AC-4) | ✅ ~96% — AC-3 full decode + E-AC-3 SPX (§E.3.6 HF regen) + transient pre-noise (§E.3.7.2 TPNP) + downmix + WAVE_FORMAT_EXTENSIBLE; AC-4 ~98% decoder + IMS encoder ~65% (mono/stereo/5.0/5.1/7.1 Cfg3Five + 5_X ASPX_ACPL_3 multichannel) | 🚧 AC-3 ~95% — acmod 1/2/2.1/3/6/7 + LFE + DBA + 5-fbw coupling + E-AC-3 indep+dep + per-channel PSNR gates + r95 two-stage equalise + spread-cap greedy for per-channel `fsnroffst[ch]` (≤ ~1.5 dB spread; closes r91 cheap-mantissa runaway) |
| **AC-4** (Dolby) | 🚧 ~98% — A-SPX + DRC + 60+ ETSI codebooks + 5_X/7_X ACPL_1/2/3 + cfg0/1/2/3 + LFE + SSF/SNF + SAP + Pseudocode 121 companding + IMS bitstream_version≥2 walker + 7_X SIMPLE/Cfg3Five inner 5-ch IMDCT; lacks ETSI fixture RMS audit, object/a-joc substreams | 🚧 IMS ~65% — v0/v2 TOC + mono SIMPLE/ASF + stereo SIMPLE 2× SCE split-MDCT + joint M/S CPE + 5.0/5.1/7.1 SIMPLE Cfg3Five + 5_X SIMPLE/ASPX_ACPL_1/2 + ASPX_ACPL_3 multichannel encoder (aspx_config + acpl_config_1ch/2ch + companding + stereo_data + aspx_data + acpl_data; ACPL_1 joint-MDCT surround residual; zero-delta Huffman codewords for all 18 ASPX + 24 ACPL HCBs); lacks real QMF-domain (α,β,γ) extraction, real ASPX envelope coding |
| **MIDI** (SMF) | ✅ ~99% — SMF Type 0/1/2 → PCM via 32-voice mixer + SF2/SFZ/DLS + DLS `art1`/`art2` + SF2 EG2 + 2-pole resonant low-pass biquad on shared SamplePlayer + SFZ filter EG (`cutoff` / `resonance` / `fil_type` covering all 6 SFZ v1 shapes + `fileg_*` envelope opcodes) + MPE v1.1 + RPN 0/1/2/5/6 + CA-25 Master Tuning + MIDI Tuning Standard (per-key + scale/octave microtuning) + Universal Master Volume SysEx + Data Inc/Dec (CC 96/97, RP-018) | — synthesis only |
| **NSF** (NES) | 🚧 ~90% — full 6502 + IRQ/NMI + 5/5 2A03 APU + DMC DMA + six expansion chips + NSF v1/v2/NSFe + Dendy region + mixe per-device gain + plst/psfx playlist iteration + region-aware noise period (NTSC+PAL); VRC7 still 2-op approximation pending #861 | — synthesis only |
| **Shorten** (.shn) | 🚧 r5 (post-2026-05-18 orphan) — `ajkg` magic + v2/v3 ulong + svar(n) + per-block function dispatch + VERBATIM/QUIT + DIFF0..3 + Rice residual + per-channel carry + spec/05 §2.5 running mean estimator (sliding-window `mu_chan`; DIFF0/ZERO consumers) + QLPC quantised-LPC predictor (§3.5); lacks BLOCKSIZE/BITSHIFT + Decoder impl | 🚧 scaffold |
| **TTA** (True Audio) | ✅ ~97% — TTA1 fmt=1/2 + password + trace tape + ID3v1 / APEv2 trailer + multi-frame format=2 trace coverage closes audit/07 §6.2 (HEADER_CRC carries IEEE-802.3 CRC32; LMS_PRE digest seal per spec/07 §3.5/§3.6) | ✅ ~96% — TTA1 fmt=1/2 + password; bit-exact self-roundtrip |
| **aptX** (classic + HD) | 🚧 ~70% — 4-band QMF + ADPCM; bit-exact NDA-blocked | — |

</details>

<details>
<summary><strong>Video</strong> (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **MJPEG** | ✅ ~96% — baseline + progressive 4:2:0/4:2:2/4:4:4/grey + SOF9 arithmetic + lossless SOF3 grey P=2..16 + 3-comp RGB P=8 + RFC 2435 RTP/JPEG depacketization + packetization | ✅ ~95% — baseline + progressive + lossless SOF3 grey/RGB (all 7 Annex H predictors) + DRI/RSTn restart markers + non-zero point transform Pt 0..15 |
| **FFV1** | 🚧 r5 (post-2026-05-18 orphan) — RFC 9043 §4.2/§4.3 cfg-record + §4.6 SliceHeader + §4.7/§4.8 Slice Content scaffold + §3.8.2 Golomb-Rice (ur/sr + ESC) + §3.3 median predictor + §3.5 context model + §3.8.2.4 VLC symbol decoder + per-row `decode_line` → `Vec<i32>` sample_difference + §4.1 Quantization Table Set cascade (8/16-bit quant tables, context_count bit-exact); 90 tests; lacks pixel reconstruction + range mode + Slice Footer + encoder | 🚧 scaffold |
| **MPEG-1 video** | 🚧 clean-room rebuild (post-audit 2026-05-18) — sequence/GOP/picture/slice headers + §6.2.5 macroblock_address_increment + §6.2.5.1 macroblock_type (Annex B Tables B-2/B-3/B-4) + §6.2.5.3 coded_block_pattern (Table B-9 + 4:2:2/4:4:4 ext); motion / residual VLCs / IDCT pending | 🚧 scaffold |
| **MPEG-2 video** | 🚧 r6 (post-2026-05-18 orphan) — §6.2.2.1/§6.2.2.3/§6.2.2.6 sequence/GOP + §6.2.3 picture_header + §6.2.3.1 picture_coding_extension + §6.2.4 slice_header + §6.2.5 macroblock_address_increment (Annex B Table B-1 33-code VLC + escape chain + MPEG-1 stuffing) + §6.2.5.1 macroblock_type + §6.2.5.3 coded_block_pattern (Table B-9 + 4:2:2/4:4:4 ext); r8, 121 unit tests; lacks MV / residual VLCs / IDCT | 🚧 scaffold |
| **MPEG-4 Part 2** | 🚧 r7 (post-2026-05-18 orphan) — VS/VO/VOL + §6.2.3 + GOV + VOP header + §6.2.3.3 quant-matrix + §6.2.6 I/P-VOP MB header + §6.2.6 B-VOP MB header (modb Table B.3 + mb_type B.4 non-scalable + B.5 scalable + cbpb 4:2:0 + dbquant Table 6-33; Direct/Forward/Backward/Interpolated typed enum) + §6.2.6.2 motion_vector + §7.6.3 differential-MV (Table B.12 VLC + Table 7-9 wrap) + §7.6.5 median MV predictor; 149 tests; lacks candidate gathering (Fig 7-34) / DCT / interlaced_information / encoder | 🚧 scaffold |
| **Theora** | 🚧 r5 (post-2026-05-20 orphan) — §6.1 + §6.2 + §6.3 + §6.4 setup-header entrypoint + Appendix B.2/B.3 VP3-default LFLIMS/ACSCALE/DCSCALE tables (vp3_defaults constructor) + §6.4.2 quantization-parameters decode (ACSCALE/DCSCALE/base matrices/quant ranges); §6.4.1 LFLIMS body blocked on spec-PDF page gap (#944) | 🚧 scaffold |
| **H.263** | 🚧 r6 (post-2026-05-18 orphan) — §5.1 picture + §5.2 GOB + §5.3 MB header (full Tables 7/8/12/14) + §5.4 block data (Table 15/16 VLC + zigzag) + §6.1/§6.2.1 H.261-style inverse-quant + §6.2.3 zigzag scatter + §6.2.4 orthonormal IDCT (f64, OnceLock cosine table) + §6.3.2 [0,255] clip → `reconstruct_intra_block` + §6.1.1 MV reconstruct + §6.1.2 half-pel interp + §6.3.1 INTER summation; 117 tests; lacks MB-loop driver / deblocking / Annex T/I/G/D | 🚧 scaffold |
| **H.261** | ✅ ~98% — I+P QCIF/CIF + integer-pel + loop filter + §5.4 BCH FEC + §5.2 + Annex B HRD + RFC 4587 RTP payload format (4-byte §4.1 header + GOB-aligned packetizer + `-16` MVD guard) | ✅ ~98% — spiral+diamond ME + GQUANT-from-bitrate + BCH framing + RTP wrap + RFC 3550 §5.1 `RtpPacketizer` (M-bit + seq + ts + SSRC over GOB-aligned payloads) + RFC 3550 §6.4 RTCP SR/RR + §6.5 SDES/CNAME + §6.6 BYE + §6.1 compound packet build/parse (`sender_report()` wired from packetiser packet/octet counts); 45 dB at 64 kbit/s QCIF |
| **MS-MPEG-4** (v1/v2/v3) | 🚧 ~37% — clean-room scaffold; v3 intra 3-tier ESC + custom intra-DC VLC + G0..G3 LMAX/RMAX wired + synthetic-VLC end-to-end + v1/v2 CBPY VLC binary↔H.263 Table 8 / MPEG-4 Part 2 Table B-6 cross-check + spec/15 §3 (count_A, count_B) provenance-pinned single-source-of-truth table (322 tests); still lacks G0..G3 primary canonical-Huffman bit-length array (spec/99 §10 OPEN) + alt-MV VLC re-extract. VfW-sandboxed mpg4c32.dll runs in parallel | — |
| **H.264** | 🚧 ~80% — I/P/B + 4:2:0/4:2:2/4:4:4 + CAVLC + CABAC + DPB + B-pyramid POC + **34 SEI types** (+spare_pic §D.1.10 in r103; +3 in r99: sub_seq_info / sub_seq_layer_characteristics / sub_seq_characteristics §D.1.11–13) + fuzz-hardened slice/MC/SPS bounds + r91 strictness fixes — fuzz CI green; lacks MBAFF, SVC/3D/MVC | 🚧 ~82% — I+P (1MV/4MV, ¼-pel) + B + CABAC at all chroma layouts + Trellis-quant RDOQ-lite; ffmpeg PSNR_Y 44.20 dB |
| **H.265 (HEVC)** | 🚧 r6 (post-2026-05-18 orphan) — Annex B + §7.3.1.2 NAL + §7.3.2.1 VPS + §7.3.3 PTL + §7.3.2.2 full SPS body (prefix + PCM + §7.3.7 short-term RPS both explicit + inter-RPS-prediction forms + long-term RPS + MVP/smoothing + opaque VUI/ext tail) + §7.3.2.3.1 PPS (tiles + deblocking-control + lists-mod/parallel-merge tail, se(v)) + §7.3.6.1 slice header (independent I-slice IDR end-to-end through byte_alignment; non-IDR POC/RPS + P/B ref-list/pred-weight bodies opaque); 61 tests; lacks scaling_list_data / VUI body decode / ext bodies / CABAC (#444) | 🚧 scaffold |
| **H.266 (VVC)** | 🚧 ~64% — 4:2:0 IDR intra + ALF/SAO/CC-ALF + P/B merge+skip + HMVP + MMVD + CIIP + BCW + BDOF + GPM + AMVR + HBD + chroma 4-tap sub-pel + DMVR §8.5.3.2.4 + affine sub-block MC §8.5.5.9 + PROF §8.5.6.4 + §8.5.5.5/§8.5.5.6 affine merge candidates + §8.5.5.2 subblockMergeCandList insertion order + merge_subblock_idx pick + §8.5.5.2 steps 3-6 neighbour/corner-selection cascade (inherited-A/B scans + constructed-corner availability under parallel-merge gate) + §7.3.10.10 mvd_coding() + §9.3.3.14 limited-EGk; lacks SbTMVP record + AMVP MVP derivation + non-merge inter CU wire-up | 🚧 ~85% — forward CABAC + DCT-II + SAO/ALF/cu_qp_delta + MTT BT+TT RDO + P+B slice + sub-pel MC ½/¼-pel (luma + chroma) + multi-ref DPB + weighted bi-pred — see crate README |
| **VP6** | 🚧 r2 (post-2026-05-18 orphan) — §9 raw-bit frame-header prefix + §15 inverse-quantization (DctQMask→DC/AC dequant tables, full 64-coeff block); §7.3 BoolCoder b(n) blocked on Split-formula spec defect (#930) | 🚧 scaffold |
| **VP8** | 🚧 r8 (post-2026-05-20 orphan) — RFC 6386 §7 bool decoder + §9.1 + §19.2 + §11 KF MB mode layer + §12 intra-prediction kernels + §13 DCT-coefficient token decoder (coeff_tree walker + §13.2 EOB-skip + §13.5 default coef_probs[4][8][3][11] table + extra-bits decode through CAT6 11-bit DCTextra) + §14 dequant + inverse WHT/DCT + summation + §15 loop-filter per-segment kernels (simple/normal + §15.4 control params); 127 tests; lacks §15.1 filter geometry / §14.2 per-MB orchestration / §16 inter modes / §17 MV decode | 🚧 scaffold |
| **VP9** | 🚧 r9 (post-2026-05-20 orphan) — full §6.2 walk + §9.2 Boolean decoder + §6.3.1/§6.3.2/§6.3.3/§6.3.7/§6.3.8 compressed-header sweeps + §6.4.24 / §6.4.26 coefficient-token decoder + §8.6.1 dequant + §8.7 inverse transforms (DCT/ADST/WHT 1D + 2D driver, cos64_lookup + SINPI_*_9 spec verbatim); 120 tests; lacks §8.6.2 reconstruct driver / §8.4 partition/block tree / §8.5 prediction / inter §6.3.9+ / loop filter | 🚧 scaffold |
| **AV1** | 🚧 r9 (post-2026-05-20 orphan) — §5.3 OBU + §5.5 sequence + §5.9.2 prefix + §5.9.5–§5.9.9 frame_size + §5.9.3 allow_intrabc + §5.9.15 tile_info + §5.9.12 quantization_params + §5.9.14 segmentation_params + §5.9.17 delta_q + §5.9.18 delta_lf + §5.9.11 loop_filter_params (streaming, with CodedLossless derivation); lacks cdef / lr / tx_mode / frame_reference_mode / tile data / inter ref-frame state | 🚧 scaffold |
| **Dirac / VC-2** | ✅ ~90% — VC-2 LD + HQ intra + Dirac core-syntax intra/inter + OBMC + 7 wavelets + 10/12-bit; ffmpeg bit-exact at multiple chroma | 🚧 ~94% — HQ + LD intra + Dirac core-syntax + per-block adaptive sub-pel-vs-int-pel selection on 1-ref P-path (pre- AND post-OBMC) + 2-ref bipred path widened to strict-superset {int-pel, half-pel, sub-pel} + post-OBMC bipred mode-only refinement pass (+0.80 dB Y PSNR on camera-pan ME-only, ffmpeg cross-decode ceiling preserved) + §11.3.3 core-intra codeblock spatial-partition encoder (cumulative-quant decoder fix) + §13.4.3.3 all-zero codeblock skip (zero_flag) |
| **AMV video** | 🚧 scaffold (orphan rebuild post-audit 2026-05-18; clean-room re-implementation pending) | 🚧 scaffold (orphan rebuild post-audit 2026-05-18; clean-room re-implementation pending) |
| **ProRes** | ✅ ~96% — RDD 36 entropy + 8/10/12-bit + 4:4:4:4 alpha + interlaced + RDD 36 §6.4 + §6.1.1 "shall refuse" clause enforcement; ffmpeg interop 60-68 dB | ✅ ~93% — emits valid RDD 36 across all 6 profiles + interlaced (apcn/apch TFF+BFF ffmpeg cross-decode 64.17 dB) + alpha + perceptual quant matrices + explicit profile override (`EncoderConfig::with_profile`) + multi-frame rate-control ±5 % over 8-frame run + genuine 10-bit interlaced field-pair packing (ffmpeg cross-decode ~64 dB) + interlaced 4444+alpha + genuine-12-bit forward cross-decode (prores_ks reconstructs ap4h/ap4x field-pair + 16-bit alpha at 65 dB) + progressive 4444+alpha 12-bit forward cross-decode (single-picture ap4h/ap4x ~64.8 dB) |
| **EVC** (MPEG-5) | 🚧 ~78% — NAL + SPS/PPS/APS + §9.3 CABAC + §8 intra (Baseline) + DCT-II + P/B inter + RPL + HMVP + DPB + ALF + DRA + IBC §8.6 + §7.3.8.4 `coding_unit()` IBC branch on BOTH IDR-slice (r91) and non-IDR P/B-slice (r95) paths + §7.3.8.5 cu_qp_delta on intra + inter + both IBC transform_unit() branches; lacks full CABAC-driven inter IBC fixture (CabacEncoder::encode_bypass test-side bug) | — |
| **HuffYUV** / FFVHuff | ✅ ~96% — HFYU + FFVH FourCCs + 6 predictors + 8-bit only + interlaced field-stride=2 + fast-LUT decoder + flat overflow_entries slow path + SWAR 8-byte gradient post-pass (2.18×/2.56× M1) | ✅ ~96% — full encoder symmetry × YUY2/RGB24/RGB32 + v1.x + v2.x ClassicV2/CustomV2 + walking-stride interlaced + predictor auto-selection + r95 SWAR forward-gradient encoder + intermediate-allocation elimination (1.5-1.7× encode speedup on Left/Median 320×240 + 720p Left) + r100 fused LEFT+decorrelation residual + r103 GradientDecorr decorrelation fusion (encoder allocates no decorrelated buffer on any method) |
| **Lagarith** | ✅ ~95% — all 11 wire types + modern range coder with spec/02 §5 three-way fast path + legacy adaptive-CDF + Fibonacci-Zeckendorf prefix + JPEG-LS Median + G-pivot decorr + zero-run RLE + pair-packed 513-entry CDF (Strategy F, decode-only for proprietary type-7 streams) | 🚧 ~76% — encoder for SOLID/RGB/RGBA/YV12/YUY2/legacy-RGB + spec/02 §5 Step-A + Step-B + Step-C `freqs[]` cache (1.08× on Step-C-heavy fixtures, 244 MSym/s); byte-exact vs proprietary encoder Auditor-blocked |
| **Ut Video** | ✅ ~97% — 5 native FourCCs (ULRG/ULRA/ULY0/ULY2/ULY4) × 4 predictors + RGB inter-plane decorrelation + LUT-accelerated canonical Huffman + word-aligned bit reader + slice-parallel decode (2.87×→5.63× speedup 320×240→1280×720) + spec-pinned `Extradata::ffmpeg_for` builder (encoder_version 0x0100_00f0 + RGB source-format tag per spec/01 §5 / audit/00 §5.2) | ✅ ~96% — codec-internal encoder + slice-parallel encode (1.13×→3.28× speedup, byte-identical to serial path) + spec/01 T1 content-fixture corpus (8 patterns × 4 predictors × 5 FOURCCs at 128×96) + r96 encoder byte-stability suite (deterministic/path-invariant encode + `encode∘decode∘encode` fixed point + 1..256 slice sweep at non-divisible heights) + r101 malformed-payload decode-rejection suite (per-variant guard pinning; 118 tests) |
| **MagicYUV** | ✅ 100% — 17 v7 FOURCCs (8 + 10/12/14-bit M0/M2/M4) + Median + JPEG-LS Median (HBD) + raw-mode + interlaced + AVI 1.0/OpenDML 2.0; trace JSONL strict-jq-line-diff-equal to cleanroom Python ref; decode/encode 1.6-1.9× faster than pre-optimisation | ✅ 100% — `encode_frame` / `encode_avi` / `encode_avi_opendml` across all 17 FOURCCs + spec/04 §3 Dynamic predictor strategy + spec/05 §6.2 Auto Huffman/raw fallback |
| **Cinepak** (CVID) | ✅ ~96% — frame header + multi-strip + V1/V4 codebooks + intra + inter with skip + full selective-update family + grayscale + Sega FILM demuxer + r93 Sega Saturn / Lemmings 3DO deviant Cinepak decoder (`DeviantConfig`, `FilmDemuxer::variant()` classifier) | ✅ ~98% — stateful `CinepakEncoder` with rolling codebooks + multi-strip + skip-MB + Lagrangian RDO + LBG + luma-weighted distance + median-cut + Lloyd polish + 3-axis RD grid picker + per-strip independent (λ, luma_weight) picker + k-means++ cold-start init + r96 single-encoder bitrate-target rate control (`with_target_bitrate`, `RateStats`) over the 3-axis grid + r101 grayscale RD-grid frame-level picker (`encode_gray8_round7`) + r104 stateful inter-frame grayscale (`encode_intra_gray8`/`encode_inter_gray8` — rolling codebooks across frames, 88% wire savings on static fixtures) |
| **SVQ1/SVQ3** (Sorenson) | 🚧 r4 (orphan rebuild) — SVQ1 frame-header + framework registry (SVQ1/svqi FourCC) + SVQ3 SEQH + slice + MB-type tree (105 tests); SVQ1 pixel decode blocked on docs (§14.10/§14.11 codebook bytes #429) + SVQ3 coefficient/MV-VLC tables | — |
| **Indeo 3** (IV31/IV32) | 🚧 r5 — clean-room codec-frame header + bitstream header + spec/02 picture-layer plane-prelude parser + spec/03 macroblock-layer binary-tree walk + spec/04 VQ codebook materialisation + spec/06 byte-level entropy (mode-byte classify + jump-table/continuation + RLE escapes + per-position acceptance + FB-counter category); lacks per-cell unpack arithmetic (spec/06 §8) / MC (spec/05) / pixel reconstruction (spec/07) | — |
| **Indeo 2/4/5** | 🚧 scaffold — pending clean-room workspace; Indeo 4/5 still sandboxed via `oxideav-vfw` | — |

</details>

<details>
<summary><strong>Image</strong> (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **PNG / APNG** | ✅ 100% — 5 colour types × 8/16-bit + APNG + sBIT/pHYs/tIME/bKGD/hIST/eXIf/sRGB round-trip | ✅ 100% |
| **GIF** | ✅ 100% — 87a/89a + LZW + interlaced + animation + disposal compositor + structured Application Extensions (NETSCAPE2.0 / ANIMEXTS1.0 / XMP / ICC / Exif) + Plain Text Extension + lenient-decoder mode + lazy `Playback` + §18.c.viii pixel-aspect-ratio accessors + animation-timing accessors (`frame_delays`/`single_pass_duration`/`total_play_duration` w/ NETSCAPE2.0 loop count); clean-room from CompuServe spec + Welch 1984 | ✅ 100% — per-frame palettes + `optimize_color_tables()` GCT/LCT hoisting + §7 Required Version enforcement (rejects 89a-only blocks under Gif87a header) + `upgrade_version_if_needed()` |
| **WebP** (VP8 + VP8L) | 🚧 r8 (post-2026-05-20 orphan) — RFC 9649 §2.3-§2.7 walker + VP8X + ALPH + ANIM + ANMF + §2.3/§2.4/§2.7.1 RIFF builder + typed §2.5 `VP8 ` routing handle + typed §2.6 `VP8L` routing handle (WebP Lossless §3.4/§7.1 image-header peek: width/height/alpha_is_used/version; no oxideav-vp8/oxideav-vp8l runtime dep) + §2 LSB-first bit-reader + §4 transform-list header reader + §6.2.1 VP8L prefix-code reader + canonical decoder (simple/normal CLC, max_symbol, repeat/zero-run, Kraft completeness); 151 tests; lacks §6.2.2 meta-Huffman + §5.2 LZ77/color-cache + encoder | 🚧 scaffold |
| **JPEG** (still) | ✅ ~95% — via MJPEG | ✅ ~90% — via MJPEG |
| **TIFF** (6.0) | ✅ ~97% — II/MM + BigTIFF read + 6 photometrics + 1/4/8/16-bit + None/PackBits/LZW/Deflate/CCITT-MH/T.4-1D + FillOrder + tiles + multi-page + JPEG-in-TIFF + PlanarConfiguration=2 (separate component planes across strips/tiles + chunky re-interleave + Predictor=2 driven per-plane); lacks CCITT T.4 2-D / T.6 (#874), CMYK-JPEG, JPEG-in-TIFF + planar=2 | ✅ Gray8/16/RGB24/Palette8 — None/PackBits/LZW/Deflate + Predictor=2 + PlanarConfiguration=2 separate-planes write (Rgb24 × None/PackBits/LZW/Deflate ± Predictor=2) + Bilevel CCITT-MH / T.4-1D, single+multi-page |
| **BMP** | ✅ ~96% — 1/4/8/16/24/32-bit + V4/V5 + OS/2 BITMAPCOREHEADER + RLE4/RLE8 + top-down rows | ✅ ~96% — top-down + minimal `biClrUsed`-trimmed palette encoder |
| **Netpbm** (PBM/PGM/PPM/PNM/PAM) | ✅ ~95% — all 8 magics at 1/8/16-bit + 6 PAM TUPLTYPEs | ✅ ~95% |
| **ICO / CUR** | ✅ ~97% — multi-res + BMP/PNG sub-images + CUR hotspot + ICONDIRENTRY validation (bReserved / dwBytesInRes / overlap-with-directory / cross-entry payload-overlap / overflow / wPlanes / wBitCount / CUR hotspot-in-bounds) + `select_best_fit` / `select_largest` / `select_by_dimensions` resolution helpers + 256×256 PNG round-trip + write 1..=256 dimension guard + `.ani` RIFF/ACON detection | ✅ ~92% |
| **JPEG 2000** | 🚧 r8 (post-2026-05-20 orphan) — T.800 main-header + SOT/SOD + typed COC/QCC/POC/RGN/PLT/PPT + JP2 box + §B.10 tier-2 packet-header + §B.2/§B.3/§B.5 TileGeometry + §B.5 typed `ResolutionLevel` (per-r corners) + `SubBand { HL, LH, HH }` (per-r-per-orientation corners per Eq. B-14 / Table B.1) + §B.6 precinct + §B.7 code-block partition (Eq B-16/17/18) + §B.7/§B.9 precinct→code-block enumeration (PacketGeometry bridge); lacks §B.12 progression / tier-1 MQ / wavelet / dequant / MCT | 🚧 scaffold |
| **JPEG XL** | 🚧 ~86% — ISO/IEC 18181-1:2024 final core. 7 small lossless fixtures decode PIXEL-CORRECT. Modular path + ISOBMFF `FF 0A` strip + 1..16 bpp pack + §F.3 zero-pad single-TOC fast path; VarDCT scaffold with Annex I.2 IDCT + GetDCTQuantWeights + 17-slot dequant set + §C.7.1 HfPass + §C.8.3 PassGroup HF + **§F.3 HF dequantisation pure-math step** (Listing F.2 verbatim); lacks ANS-driven permutation + §C.7.2 histograms + per-block coefficient loop + CfL / Gaborish / EPF | — retired |
| **JPEG XS** | 🚧 ~80% — ISO/IEC 21122 Part-1 + 5/3 DWT + Annex C/D/F/G + multi-component + CAP-bit + `Cw > 0` + `Sd > 0` (CWD) cascade + Sd>0 composes with Cpih∈{1,3} per Annex F.2 Table F.1 | 🚧 ~78% — Nc 1/3/4 + Sd>0 raw-tail (Nc up to 8) + RCT + Star-Tetrix + NL up to 8 + odd dims + vertical prediction + significance coding + per-band Q + NLT + `Cw > 0` cascade + r95 `Sd > 0` ∘ `Cpih ∈ {1, 3}` (RCT on c<3; Star-Tetrix on c<4; lossless at Nc=4 + RCT and Nc=5 + Star-Tetrix) + Fs=1 separate sign sub-packet (Annex C.5.5) + multi-slice `Hsl` emission (Annex B.10, one SLH per slice) |
| **AVIF** | 🚧 ~80% — HEIF→AV1 + grid + imir/clap/colr/pixi/pasp + HDR metadata + AV1 wrap pass-through + DoS caps + HEIF item-properties (infe v2/v3 mime/uri tail + thmb/cdsc/prem iref + Exif/XMP item resolver) + auxC URN routing (Alpha / Depth / HDR-gain-map) + rloc / lsel / iovl / grpl parsers + `mif1` compliance audit; AV1 pixel decode gated on sibling rebuild | — |
| **DDS** | ✅ ~98% — DDS_HEADER + DXT10 + uncompressed (10 layouts) + BC1-5/7 + BC6H all 14 modes + mipmap + 6-face cubemaps + DX10 arrays + full 132-entry DXGI table | ✅ ~95% — uncompressed + BC1-5 + BC7 all 8 modes (0-7 incl. mode 4/5 channel-rotation; rank-3 multi-axis 30.4 dB; independent-alpha ≥30 dB-RGBA) + BC6H_UF16 all 14 modes + BC6H_SF16 all 14 modes (signed-magnitude pipeline across 1/2-subset signed) + box-downsample-then-encode mip chains + cubemap/array |
| **OpenEXR** | 🚧 ~75% — magic + 8 required attrs + HALF/FLOAT/UINT + NO_COMPRESSION/ZIP/ZIPS/RLE + tiled ONE_LEVEL + sub-sampled chroma + single-part deep scanline + multi-part deep scanline read (`parse_exr_deep_multipart` validated via `exrmultipart -combine`); exrmetrics cross-validates; PIZ blocked on docs trace; lacks B44/B44A/DWAA-B, deep-tiled | ✅ ~85% — RGBA scanline + ZIP/ZIPS/RLE + tiled-output ONE_LEVEL + tiled-output MIPMAP_LEVELS + multi-part scanline + sub-sampled channels + deep scanline write (NONE/RLE/ZIPS); exrmetrics + exrmultipart + exrinfo + exrheader cross-validate bit-exact |
| **Farbfeld** | ✅ 100% — streaming reader + DoS hardening (dimension overflow + truncated payload guards) + `magick` black-box cross-validator | ✅ 100% |
| **HDR** (Radiance RGBE) | ✅ ~98% — new-RLE + old-RLE + 8 axis-flag combos + shared-exponent + multi-record EXPOSURE/COLORCORR + typed COLORCORR / PRIMARIES / VIEW headers + apply_exposure / apply_colorcorr helpers | ✅ ~98% — new-RLE + old-RLE + auto-RLE + 8 axis combos + XYZE↔RGB + 8 tonemap ops + CRLF line endings (encode_hdr_with_options) |
| **QOI** | ✅ 100% — byte-exact vs all 8 reference fixtures | ✅ 100% — byte-exact vs reference encoder |
| **TGA** | ✅ 100% — types 1/2/3/9/10/11 + TGA 2.0 extension + thumbnail + developer area + colour-correction table (parse + 16-bit LUT application) + scan-line table + typed AttributesType; magick cross-validated | ✅ 100% — all six image types + full TGA 2.0 extension (CCT / SCT / developer tags) + thumbnail + RGB24-input entry points |
| **ICER** (JPL) | 🚧 ~75% — Mars-rover heritage; bit-plane scan + compressed/uncompressed segments + 8 filters + IPN 42-155 §III.B context model | ✅ ~80% — quota-controlled encoding (`with_byte_budget` / `with_target_bytes` / `with_rd_budget`) — MSB-down progressive truncation + r5 auto wavelet-filter selection + R-D byte-budget per-segment ranking (IPN 42-155 §IV.B; +6.09 dB checker @ 400 B vs strict-MSB, never regresses) |
| **WBMP** | ✅ 100% — Type 0 + WbmpLimits DoS caps + adversarial fuzz sweep | ✅ 100% |
| **PCX** (ZSoft) | ✅ ~97% — 1/2/4/8 bpp planar + packed-bits + 24 bpp RGB planar + grayscale flag + DCX multi-page + DCX `Demuxer` | ✅ ~97% — 8 write paths (incl. grayscale + windowed 24bpp) + DCX; framework `Encoder` accepts Rgba/Rgb24/Gray8 |
| **ILBM** (Amiga IFF) | ✅ ~94% — BMHD/CMAP/CAMG/BODY + ByteRun1 RLE + EHB + HAM6/HAM8 + PBM + SHAM + PCHG + ANIM op-0/op-5 + CRNG/CCRT + DRNG (DPaint IV extended range, true-colour + register cells); lacks ANIM op-7/op-8, DEEP true-colour | ✅ ~84% — IlbmMuxer parity + masking + ANIM op-5 + CRNG/CCRT/DRNG encoder |
| **PICT** (Apple QuickDraw) | ✅ ~97% — v1 + v2 opcode walkers + drawing rasteriser + DirectBitsRect packType 0/1/2/3/4 + Region + clip-region + pen-size aware + Compressed/UncompressedQuickTime opcode skip + monochrome stipple + **PixPat colour 8×8 type 1** + **dithered PixPat (`patType=2`)** per Inside Macintosh §A-3 / §4 Color QuickDraw (8×8 RGBA tile uniformly painted with `RGBColor`) + `probe_pict`; lacks non-8×8 PixPat tiles, text rasterisation, embedded JPEG decode | ✅ ~93% — `PictBuilder` + every v2 drawing-command family + state opcodes + mono+PixPat pattern setters + DirectBitsRect packType 1/2/3/4 + BitsRgn / PackBitsRgn; magick cross-decode bit-exact |
| **SVG** | ✅ ~99% — full shape set + path + gradients + text + mask + clipPath + use/symbol + svgz + SMIL animate/set/animateTransform + CSS3 Selectors L3 + `@import` + `@font-face` + `@keyframes` + Media Queries L4 + viewBox + 17 filter primitives + CSS Values L4 LengthUnit + CSS Easing L2 + SVG 2 §9.6.1 pathLength + SVG 2 §16.3 `<view>` element + fragment-identifier routing (`#MyView` / `#svgView(...)` + percent-decode + spatial/temporal media-fragment fallthrough) + SVG 2 §5.7 `<switch>` conditional processing (requiredExtensions / systemLanguage) + SVG 2 §13.7.1 `<marker>` typed def capture (refX/refY geometric keywords + markerUnits/orient + verbatim round-trip) + SVG 2 §13.2 `context-fill`/`context-stroke` | ✅ ~88% — round-trips full shape graph + PreservedExtras side-channel + `<view>` re-emit at trailing edge |
| **PDF** | ✅ ~99% — bytes → Scene via xref/xref-streams/ObjStm + `/Prev` incremental + `/Encrypt` R=2..6 + public-key `adbe.pkcs7.s3/s4/s5` + PKCS#7 verify + `/Sig` AcroForm verify + Doc-Timestamp `ETSI.RFC3161` reader + text extraction + Linearization + Tagged-PDF + EmbeddedFiles + §12.6 actions + indirect `/Length` + all 5 generic stream filters (Flate/LZW/ASCII85/ASCIIHex/RunLength incl. chains; `/DecodeParms /Predictor` PNG 10-15 + TIFF-2 post-filter on Flate/LZW) + §8.11 Optional Content (OCG / OCMD typed surface; default + alternate config dicts; §8.11.4.5 ON/OFF resolution; PDF 1.6 `/VE` visibility expressions w/ cycle guard; `DocumentReader::optional_content()`) | ✅ ~99% — PDF 1.4/1.5 multi-page + paths/gradients/opacity/clip + RGBA + xref-stream + ObjStm + Linearization writer + `/Encrypt` ENCODE + public-key ENCODE + `/Sig` writer §12.8.1.1 + AcroForm widget §12.7.4 + annotation writer §12.5.6 (8 kinds) + embedded file attachment §7.11 + RFC 3161 Document Time-Stamp writer §12.8.5 (TsaSigner trait; qpdf + openssl ts -verify accept) |

</details>

<details>
<summary><strong>3D scenes & assets</strong> (click to expand)</summary>

> The typed Scene3D / Mesh / Material PBR / Skin / Animation / Camera / Light / AudioEmitter model lives in `oxideav-mesh3d`, with `Mesh3DDecoder` / `Mesh3DEncoder` traits and a `Mesh3DRegistry` that's parallel to `oxideav-core::CodecRegistry`. Per-format crates register into it. `oxideav-meta::populate_mesh3d_registry(&mut Mesh3DRegistry)` walks every enabled format's `register()`. Lazy bytes flow through `AssetSource` (with a `raw_storage` pass-through hook for archive-backed sources, e.g. ZIP-stored USDZ textures + audio).

| Format | Decode | Encode |
|--------|--------|--------|
| **STL** (ASCII + binary) | ✅ ~99% — both formats + per-face attributes + 16-bit colour + multi-`solid` ASCII + topology (Euler χ) + repair pipeline (weld + degenerate-cull + zero-normal recompute + orient-from-winding + unit-normal renormalise + consistent-winding) + ASCII comment preservation | ✅ ~99% — both formats + attribute pass-through + `EncodeStats` + configurable float precision + spec-style scientific ASCII formatter (`1.23456E+789` form) |
| **OBJ** (+ MTL) | ✅ ~97% — full Wavefront grammar + MTL (Phong + Wavefront-PBR + map_* options + typed refl) + smoothing/display attrs + free-form geometry pass-through + `xyzrgb` per-vertex colour + Bezier + B-spline / NURBS / Cardinal (Catmull-Rom) / Taylor `curv` + Bezier + B-spline / NURBS `surf` 2D-surface tessellation (tensor-product de Casteljau / Cox-deBoor); lacks Cardinal / Taylor / basis-matrix `surf` surfaces, multi-patch decomposition, trim/hole loops | ✅ ~96% — symmetric + negative-index encoder + polyline rejoin |
| **glTF 2.0** (+ .glb) | ✅ ~92% — JSON + .glb + full PBR + KHR_lights_punctual + KHR_materials_unlit + KHR_materials_emissive_strength + KHR_materials_ior (decode + encode + §3.12 validator) + skin + skeletal animation (LINEAR/STEP/CUBICSPLINE) + sparse accessors + morph-targets + 11 spec-MUST validators + JSON fuzz hardening; lacks KHR_audio_emitter / KHR_materials_clearcoat/sheen/transmission/specular/volume / KHR_texture_transform (further KHR specs newly-staged at `docs/3d/gltf/extensions/`) | ✅ ~91% — symmetric + sparse-encoding heuristic + signed+unsigned normalised-int quantisation + KHR_materials_unlit emit |
| **USDZ** (+ USDA) | ✅ ~92% — ZIP STORED walker + USDA parser + UsdGeomMesh + UsdPreviewSurface PBR + UsdUVTexture pass-through + xformOp transforms + UsdMediaSpatialAudio + variantSet + LIVRPS variant-selection composition + composition-arc round-trip + in-archive sublayer + references/payload arc composition (LayerStack); lacks `.usdc` binary (#754), UsdSkel*, UsdGeomSubset | ✅ ~88% — symmetric writer + zero-re-encode pass-through + variant writer + composition-arc writer |
| **FBX** | 🚧 ~66% — binary container (32/64-bit) + object-graph + mesh + animation (TRS+DeformPercent) + deformers (Skin / Cluster / BlendShape) + Material / Texture / Video surfacing via Connections walker (embedded Video.Content R-blobs + OP typed PBR routing for DiffuseColor / NormalMap / EmissiveColor / metallic / occlusion) + bind pose (Pose/BindPose → node extras + skeleton inverse-bind refine). Lacks: ASCII FBX (#785), Properties70 P-record grammar + Light/Camera NodeAttribute | ✅ ~58% — symmetric binary writer + opt-in zlib deflate; Blender/ufbx-readable round-trip |
| **Alembic** | 🚧 0% — Sphinx API reference + Python examples staged at `docs/3d/alembic/`; on-disk Ogawa binary needs Wayback PDF recovery (Imageworks 2010-2012 manuals 404 today) or commissioned trace | — |

Cross-format integration: `oxideav-cli-convert` exposes a 3D conversion path through `oxideav_meta::populate_mesh3d_registry` — `oxideav convert in.obj out.gltf` (or `--probe` for structural inspection). `crates/oxideav-tests/tests/mesh3d_*.rs` runs the cross-format roundtrip suite. Convert verb has accumulated IM-compatible ops including `-resize` / `-thumbnail` / `-define`, USDZ encoder + 3D→raster renderer (Gouraud + Phong + `-light` / `-camera` / `-projection` / `-fov` / `-bg`), `-render normal-debug|depth-debug` + `-aa N` supersampling, and multi-size ICO via `-define icon:auto-resize`. Black-box oracles in `tests/mesh3d_{usdz_apple,blender_assimp}_oracle.rs` cross-validate against Apple `usdzconvert` + Blender + assimp.

</details>

<details>
<summary><strong>Trackers</strong> (decode-only by design) (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **MOD** | ✅ ~97% — 4-channel Paula-style mixer + full ProTracker 1.1B effect set + FT-extension `8xx` / `E8x` per-channel pan + XM E3x glissando control + Lxy set-envelope-position + E4x/E7x vibrato/tremolo waveform shapes (sine/saw/square) (FT2 §); PT-fidelity rounds for loop boundary / LED filter / extended period range / EE pattern-delay; 104 unit + 39 integration tests | — |
| **STM** (Scream Tracker v1) | ✅ ~85% — structural parse + shared-mixer playback; XM-parity effects (Gxy/Jxy/Bxy/Cxy/Exy/Hxy + volume-slide variants); hard-pan LRRL | — |
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
`oxideav-core::CodecRegistry`. Design contract in
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
| **`oxideav-generator`** | Synthetic media source (`generate://...` URIs) + zero-input filters | ✅ audio synth (sine + chirp/FM/DTMF/multitone/ADSR) + image (xc/gradient/pattern/fractal/plasma/noise/label) + video (testsrc/smptebars/fractal_zoom/gradient_animate/zoneplate); ImageMagick/sox shorthands in `convert` verb (vector text → raster via scribe + raster) |
| **`oxideav-rtmp`** | RTMP ingest + push | ✅ Server accepts incoming publishers + client pushes to remote servers; AMF0 handshake / chunk stream demux + **AMF3 wire-format parser + builder per Adobe AMF3 §3.1 / §1.3.1 / §2.2** + AMF3 `onMetaData`/data (type 15) + AMF3 command (type 17) routed end-to-end (bridged onto AMF0 so metadata surfaces through one path); Enhanced-RTMP v1 video (HEVC/AV1/VP9/AVC FourCC) + v2 audio (Opus/FLAC/AC-3/E-AC-3/MP3/AAC FourCC); pluggable key-verification hook; `rtmp://` registered as a `PacketSource` on `SourceRegistry` |
| **`oxideav-sysaudio`** | Native audio output | ✅ Runtime-loaded backends (ALSA, PulseAudio, WASAPI, CoreAudio); no C build-time linkage. CoreAudio + WASAPI backends report **real HAL latency** — CoreAudio sums `kAudioDevicePropertyLatency` + `BufferFrameSize` + `SafetyOffset` + `kAudioStreamPropertyLatency`; WASAPI reads `IAudioClock`-derived presentation latency. Output-device enumeration (names + default flag) across WASAPI / ALSA / CoreAudio. BT-aware; falls back to software estimate if HAL unavailable. |
| **`oxideav-pipeline`** | Pipeline composition (source → transforms → sink) | ✅ JSON transcode-graph executor; pipelined multithreaded runtime + `Executor::with_channel_caps(ChannelCaps { packets, frames })` configurable per-track depth (embedded `{1,1}` → offline `{64,32}`) + `Executor::with_max_queue_bytes(n)` orthogonal byte-ceiling on the demux→worker queues (composes with the count caps) |
| **`oxideav-scene`** | Time-based scene / composition model | 🚧 Scaffold — data model for PDF pages / RTMP streaming compositor / NLE timelines; renderer still stubbed |
| **`oxideav-audio-filter`** | Audio effects & conversions (streaming) | ✅ ~45 filters: classic + transient/spatial/restoration family + MidSide / EnvelopeFollower / DeEsser / Wah / OctaveDoubler / AdaptiveNoiseGate + Exciter / MultibandCompressor / StereoImager / Talkbox + TransientDesigner / Ducker / GainNormalizer / FreqShifter + r101 HardClipper (symmetric memoryless clipping distortion) — see crate README for the catalogue |
| **`oxideav-image-filter`** | Single-frame image effects (stateless) | ✅ 125 filter types / 160 factory names (r101 added Prewitt + PrewittMagnitude — 3×3 first-derivative edge operator, L1/L2; r24 added Roberts cross 2×2 edge operator; r22 added Reinhard / Hable / Drago tone-mapping + Curves with monotone-cubic interpolation + Borgefors distance transform + Cyanotype) — see crate README for the catalogue |
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
| **WebVTT**          | ✅ | ✅ | Header, STYLE ::cue(.class), REGION, inline b/i/u/c/v/lang/ruby/timestamp, cue-settings round-trip (vertical / line+position align / region) + full REGION block (id/width/lines/regionanchor/viewportanchor/scroll) |
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
| **ASS / SSA**       | ✅ | ✅ | Script Info + V4+/V4 Styles (BGR+inv-alpha) + override tags (b/i/u/s/c/fn/fs/pos/an/k/kf/ko/K/N/n/h). Typed `\pos`/`\fad`/`\fade`/`\move`/`\t`/`\frz`/`\frx`/`\fry`/`\org`/`\blur`/`\be`/`\bord`/`\xbord`/`\ybord`/`\shad`/`\xshad`/`\yshad`/`\fax`/`\fay`/`\fscx`/`\fscy`/`\clip`/`\iclip` extraction + time-evaluation via `extract_cue_animation` → `RenderState`; `[Aegisub Project Garbage]` + `[Fonts]`/`[Graphics]` round-trip via extradata |

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

`oxideav-mp3` and `oxideav-flac` containers surface the extracted
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
