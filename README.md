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
| WAV       | ✅ | ✅ | ✅ | LIST/INFO metadata; byte-offset seek; BWF `bext` metadata (EBU 3285); r180 smpl + inst (Sampler/Instrument) chunks (SMPTE HH:MM:SS:FF + signed-i8 FineTune/Gain) |
| FLAC      | ✅ | ✅ | ✅ | VORBIS_COMMENT, streaminfo, PICTURE block; SEEKTABLE-based seek; CUESHEET round-trip (read + write per RFC 9639 §8.7); r182 in-place symmetric-pair Levinson-Durbin update (encoder, eliminates up to 36 Vec allocs/subframe, bit-exact regression-pinned) |
| Ogg       | ✅ | ✅ | ✅ | Vorbis/Opus/Theora/Speex pages + comments; page-granule bisection + page-level seek index + chained-link-aware duration + page-loss/hole detection + page-sync recapture + public CRC-32 validation API + r172 Criterion bench harness + r183 streaming CRC + r185 Skeleton 3.0/4.0 + r192 slice-by-4 CRC-32 + branch-free `compute_page_checksum` (3-segment dispatch drops 65535 branches from max-size page; **page/parse/max 493 MiB/s → 1.3 GiB/s = 2.5-3× over r172**; `crc::continue_checksum` chunked accumulator) |
| Matroska  | ✅ | ✅ | ✅ | MKV/MKA/MKS; Cues seek; SeekHead/Chapters/Attachments/subtitles; opt-in block lacing on write; EBML CRC-32 validation (r186 per-Cluster CRC-32 now validated on advance() + Cue-driven seek, dedup via HashSet, RFC 8794 §11.3.1 + RFC 9559 §6.2); typed Tag/TrackOperation/ContentEncodings/chapters() decode; typed Video FlagInterlaced/FieldOrder + geometry quartet + Colour master + SMPTE 2086 MasteringMetadata + StereoMode + r177 Projection + r183 AlphaMode / AspectRatioType / UncompressedFourCC typed decode + 16-test injection-robustness pin |
| WebM      | ✅ | ✅ | ✅ | First-class: separate fourcc, codec whitelist (VP8/VP9/AV1/Vorbis/Opus); inherits Matroska Cues seek |
| MP4       | ✅ | ✅ | ✅ | mp4/ismv; faststart; iTunes ilst; fragmented demux+mux (DASH/HLS/CMAF) + sidx/mfra/tfra/styp; AC-3/E-AC-3/DTS sample entries; subtitle/timed-text; protected sample-entry unwrap; typed track refs + edts/elst mux + elng + kind + cslg + stsh + sdtp + sample-group sbgp/sgpd + §8.16.5 prft demux + r162 atom-walker robustness + r182 sidx-driven seek fast-path + r189 `read_box_header` `start + total_size` checked_add overflow reject (largesize fuzz crash mirroring r187 mov); lacks CENC decryption (tenc/pssh/senc) |
| MOV (QuickTime) | ✅ | — | ✅ | Apple QTFF + ISO BMFF meta + HEIF/HEIC item-properties + grid/iovl/tmap + symmetric muxer + fragmented-MP4 seek + DASH sidx/styp + stbl + traf saiz/saio sample-aux + r157 pnot preview-poster preflight + r182 ISO 14496-12 §4.2/§11.1 `uuid` User-Type Box parser + r187 `read_atom_header` size+start checked_add overflow reject (fuzz crash on `largesize = u64::MAX`); ffprobe-accepted |
| AVI       | ✅ | ✅ | ✅ | AVI 1.0 + OpenDML 2.0 demux/mux; AVIX/dmlh/vprp + 2-field interlaced + VBR audio + LIST INFO + typed PaletteChange/TextChunk/AvihFlags/Idx1Flags + ODML keyframe seek + WAVEFORMATEXTENSIBLE + `strn`/`strd` + CBR-audio validator + dmlh.dwTotalFrames + IDIT/ISMP/rcFrame/wLanguage + dwInitialFrames + r163 typed `dwChannelMask`/`Speaker`/`ChannelLayout` + r182 typed `strh.wPriority` (selection-hint u16 at byte 12) |
| Blu-ray (BD-ROM) | ✅ | — | — | `oxideav-bluray` Phase 2 — UDF 2.50 mount (ECMA-167 3rd ed.) + BDMV walk (`index.bdmv`/`MovieObject.bdmv`/`.mpls`/`.clpi`) + `.m2ts` stream (192→188-byte TP_extra_header strip) + `bluray://` URI handler with auto-detect; r93 typed `Cpi { ep_map: Vec<EpMap { stream_pid, ep_stream_type, entries: Vec<EpEntry { pts_ep_start, spn_ep_start, is_angle_change_point, … }> }> }` CPI EP_map decode per BD-ROM AV §5.7 (coarse + fine two-level table folded into a flat per-PID list a seeker can binary-search); r96 keyframe-aligned `TitleSource::seek_to(pts_90k)` (PTS→clip→I-frame→SPN×192, AACS-unit-aligned); `StreamDecryptor` trait hooks `oxideav-aacs` without hard dep. + r180 multi-angle PlayItem parsing (BD-ROM Part 3 §5.4.4.1) + `open_title_with_angle` / `max_angle` per-angle title open (AV §5.2.3.3) + r188 `Disc::chapters(title)` from PlayListMark entry marks (§5.4.5 mark_type 0x01 → title-relative seekable PTS). Lacks HDMV opcode exec, BD-J, STN_table per-stream parse (no byte-syntax in staged docs), mid-stream angle switching, cross-PlayItem STC PTS remap |
| DVD-Video | ✅ | — | — | `oxideav-dvd` Phase 3b — ISO 9660 + UDF 1.02 mount + VIDEO_TS walk + IFO body parser (VMGI/VTSI + TT_SRPT + VTS_PTT_SRPT + PGCI [+ PGC subpicture colour-LUT + pre/post/cell nav command table] + VTS_C_ADT + chapter materialiser) + VOB demux (MPEG-PS pack/PES + Nav-Pack PCI/DSI [+ PCI highlight + DSI typed sections] + DVD substream router for AC-3/DTS/LPCM/subpicture) + VOB → MKV mux (`mkv-output` feature; per-PES PTS preserved + ChapterAtom per `DvdChapter` via RFC 9559 §5.1.7) + `dvd://` URI handler + r172 typed NavInstruction VM disassembler (Phase 3c precursor: full Link family + 13-entry link-subset + Jump/Call SS + Set arithmetic + Type 4..6 classifier). + r179 Sub-Picture Unit (SPU) decoder (SPUH+DCSQT walker, 8 typed commands, 2-bit/four-form PXD RLE, 90 kHz STM-DTS conversion) + r188 SPU RGBA compositor (`composite()`: SET_COLOR/SET_CONTR → PGC palette LUT → BT.601 studio-swing YCbCr→RGB + top/bottom-field PXD interleave). Lacks VM execution (interpreter over SPRMs/GPRMs/RSM/PC), CSS auth (Phase 3c + `oxideav-css`) |
| MP3       | ✅ | — | ✅ | demuxer LANDED (ID3v2/ID3v1 skip + Xing/Info VBR + CBR/VBR seek_to); r177 Decoder-trait stereo widening (independent + joint MS + intensity, planar AudioFrame) |
| IFF / 8SVX| ✅ | ✅ | — | Amiga IFF with NAME/AUTH/ANNO/CHRS; r178 ILBM 24-bit literal-RGB true-colour + r184 CRNG/CCRT/DRNG `cycle_step` + per-line PCHG + r192 ANIM op-7 Short/Long Vertical Delta decode (Skip/Uniq/Same opcode classes via `ANHD.bits` bit-0 selector; row-major within column); lacks ANIM op-7 encode + op-8, DEEP true-colour |
| AIFF / AIFF-C | ✅ | — | — | r185 Phase 1 bootstrap (new crate) — EA IFF 85 ChunkIter + 80-bit IEEE 754 extended sample-rate decode (8 k..192 k Hz roundtrip) + COMM v1.3 / AIFF-C with compressionType FourCC + FORM walker + PCM readers (NONE/twos/sowt/raw/fl32/FL32/fl64/FL64) + compressionType→codec-id routing for ima4/ulaw/alaw; 74 tests, standalone-build OK |
| IVF       | ✅ | — | — | VP8 elementary stream container |
| AMV       | ✅ | — | — | Chinese MP4 player format (RIFF-like) — r191 clean-room demuxer rebuilt from `docs/container/amv/amv-container-trace.md`: position-coded RIFF prelude + `amvh` packed `[s,m,h,0]` duration + 20-byte WAVEFORMATEX + §4 no-byte-padding chunk walker (`advance = 8 + size` even for odd) + two-stream Demuxer (video=mjpeg@1/fps, audio=adpcm_amv@1/22050) + `AMV_END_` 8-byte trailer termination; fixture-validated against staged 3.4MB `comedian.amv` (1116 video + 1116 audio chunks @ 128×96 @ 12 fps @ 1:33 duration); 20 tests |
| FLV       | ✅ | — | — | Flash Video — MP3/AAC/H.264 audio + VP6f/VP6a/H.264 video + Enhanced RTMP ExVideoTagHeader + AMF0 onMetaData/onXMPData/onCuePoint + Annex F encryption + E-FLV ModEx walk + multitrack body splitter + HDR colorInfo metadata + r161 injection-robustness suite + 16 MB OOM-lever guard + r182 onMetaData catch-all preserves Date/Null/StrictArray/AMF3-nested + r186 unknown-script-name argument preservation via flatten_amf_value (Enhanced-RTMP-v2 producer-defined SCRIPTDATA no longer silently dropped); lacks muxer |
| WebP      | ✅ | ✅ | — | RIFF/WEBP (lossy + lossless + animation; ANIM + ANMF emit) |
| TIFF      | ✅ | ✅ | — | TIFF 6.0 single-image + r177 BigTIFF write (magic 43 / 8-byte offsets / LONG8 strip+tile arrays) + r183 PhotometricInterpretation=8 1976 CIE L*a*b* decode + r185 CCITT T.4 2-D + T.6 (Group 4) fax decode (READ algorithm; tiffcp-oracle pixel-exact) |
| PNG / APNG| ✅ | ✅ | — | 8 + 16-bit, all color types, APNG animation + r188 gAMA/cHRM colour-management chunk round-trip (PNG3 §4.3 priority-ordered emit); metadata lacks only iCCP/zTXt/iTXt |
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
| AACS  | ✅ Common 0.953 + BD-Prerecorded 0.953 | `oxideav-aacs` clean-room — KEYDB.cfg parser, `MKB_RO.inf` / `Unit_Key_RO.inf` parsers, Subset-Difference tree walk, Device-Key → Processing-Key → Media-Key → VUK derivation, AES-128-CBC Aligned Unit decryption, Title Key unwrap + Phase B SCSI MMC drive-command wire layer (REPORT_KEY / SEND_KEY / READ_DISC_STRUCTURE typed CDBs + AGID / Drive-Cert-Challenge / Drive-Key / Host-Cert-Challenge / Host-Key / Volume-ID sub-payload codecs + `DriveCommand` trait + `MockDrive` synthetic-fixture impl) + Phase C Drive-Host AKE (clean-room ECDSA over the AACS 160-bit curve + FIPS 180-2 SHA-1 + AES-128-CMAC; `host_authenticate` §4.3 state machine + `DriveAuthState` wired into `MockDrive`; Bus Key = lsb_128 of shared ECDH x-coord; §4.4 Volume-ID transfer w/ CMAC verify). + r177 READ_DISC_STRUCTURE Format 0x81 / 0x82 / 0x83 typed sub-payloads (PMSN, Media-ID, MKB-pack body up to 32 KiB; CMAC verify per §4.5/§4.6/§4.14.3.4; MockDrive serves Format 0x81/0x82). + r183 MKB ECDSA verify §3.2.5.1.2/.3/.8 (host/drive revocation list + end-of-block signature; caller-supplied AACS LA pubkey) + r188 BD-Prerecorded §2.3 Content Hash Table (per-Hash-Unit `[SHA-1]_lsb_64` verify over encrypted on-disc bytes, keyless; HASH_UNIT_SIZE 196608 B). Lacks platform `DriveCommand` back-ends (Phase D), signed Content Certificate Table 2-1 verify, AACS 2.0 (UHD-BD) |

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
| **Vorbis** | 🚧 r9 (post-2026-05-20 orphan) — identification + comment + §3.2.1 codebook + Huffman tree + full §4.2.4 setup-header walker + §3.2.1/§3.3 VQ vector unpack + §8.6 residue decode (formats 0/1/2) + §7.2.3/§7.2.4 floor type 1 + §6.2.2/§6.2.3 floor type 0 LSP + §1.3.2/§4.3.1 Vorbis window + §4.3.5 inverse channel coupling + §4.3.3 nonzero-vector propagate + §4.3.6 floor×residue + §4.3.1–§4.3.8 audio-packet driver + r180 §4.3.7 IMDCT + §4.3.6 window + r186 `StreamingDecoder` overlap-add across packets (`Primed`/`Pcm` states, per-channel `OverlapAdd`, `reset()`+`finish()` for seek/drain; ~92→~94/0, full §4.3 reachable from packet sequence; `imdct_scale` knob deferred per #1051) | 🚧 scaffold |
| **Opus** | 🚧 ~30% — RFC 6716 range decoder + full SILK pipeline + §4.3 Table 56 CELT pre-band header + §3.1/§4.2 framing dispatch + r162 §3.4 R1..R7 malformed-input audit + r182 celt_band_layout 21-band MDCT + r183 §4.3.4.3 spread + r190 §4.3.4.5 TF-resolution adjustment lookup + r191 (CELT side) §4.3.3 `LOG2_FRAC_TABLE` + `intensity_rsv` + `reserve_stereo` budget-gate helpers (158 CELT tests, closes intensity-stereo half of #943); remainder of #943 (`cache_caps50` init + full band-boost) + #936 Laplace algorithm gated | 🚧 scaffold |
| **MP1 / MP2** | ✅ Layer I + Layer II decode to PCM + §2.4.3.1 CRC-16 verify + mp2 frame-level decode loop + r191 Annex D Phase-2 psy building blocks (Tables D.2a-f + Step 6 `av_tm`/`av_nm` + 4-piece `vf(dz, X)` + composite `LT_tm`/`LT_nm` + Step 7 `LTg` + Table D.5; 222 tests; allocator pending D.1/D.3/D.4 PNG→text #1262) | 🚧 ~80% — Layer I encoder + Layer II §C.1.5.2.7 bit-allocation + Table C.5 SNR + r160 §2.4.2.3 mp2 frame-header writer + r185 §2.4.1.6/§2.4.3.3.4 Layer II SAMPLES region writer + r192 §C.1.3 polyphase analysis filterbank (all 512 Table C.1 `C[i]` coefficients OCR-transcribed from ISO PDF; precomputed `M_ik = cos[(2i+1)(k-16)π/64]`; oracled via spec-paired `D[i] == 32·C[i]` identity with Annex B Table 3-B.3 within 1 ULP across all 512 entries; +22 tests 139→161); lacks Layer II ALLOC/SCFSI writers + scalefactor extraction + iterative bit-allocator |
| **MP2** | 🚧 ~35% (post-2026-05-24 orphan) — §2.4.1.3/§2.4.2.3 Layer II header parser + §2.4.3.1 frame sizing + Annex B Table 3-B.1/3-B.2a..d/3-B.4 + joint-stereo allocation + scfsi + §2.4.3.3.4 sample requantizer + §2.4.3.1 CRC-16 + r162 malformed-input property suite + r185 full LSF Layer II wiring (ID==0 decode, 8-160 kbit/s LSF bitrate ladder, 16/22.05/24 kHz, Annex B Table B.1 sblimit=30/Σnbal=75; 159 tests); lacks §2.4.3.2 polyphase synthesis + encoder | 🚧 scaffold |
| **MP3** | ✅ ~100% — bit-exact vs mpg123; ID3v2/Xing seek + MPEG-2.5 framing + r183 Decoder-trait MPEG-2 LSF widening + r185 LAME-extension gapless playback; 634 tests | 🚧 ~93% — Phase-2 + cross-channel-MS agreement + r165 §2.4.3.4.10 attack-detector DEFAULT_AMBIENT_LEAK empirical-corpus calibration + r192 `DEFAULT_ATTACK_THRESHOLD = 10.0` empirical sweep over 8-row synthetic corpus at fixed leak=0.5 (aggregate err 1.0→179, 5.0→2, 10.0→0 argmin, plateau [10,100]; value unchanged — calibration validates existing default); lacks Annex D psy + intensity-stereo + LSF audio-chain |
| **AAC** | 🚧 Phase 1 — ADTS + raw_data_block walker + AudioSpecificConfig + program_config_element + r177 §4.4.1 GASpecificConfig extensionFlag + Table 1.15 epConfig + r192 §1.6.5 Table 1.15 trailing `syncExtensionType=0x2b7` implicit-SBR/PS/ER-BSAC probe (`AudioSpecificConfig.trailing_sbr_probe`; ext-AOT 5 reads sbrPresentFlag + optional 4-bit ext-sfi w/ 24-bit escape + secondary `0x548` sync gating psPresentFlag; ext-AOT 22 reads sbrPresentFlag + mandatory 4-bit ext-channel-config; `parse_bits_bounded` for LATM/esds carrier-bounded callers; 409 tests); decoder body still pending | 🚧 scaffold — Phase-2 writers: section_data + ics_info + pulse_data + tns_data + scale_factor_data + DPCM + r160 raw_data_block + r165 Pce::write + r183 gain_control_data SSR + r187 §4.4.2.7 extension_payload; SBR types pending QMF |
| **CELT** | 🚧 r9 (post-2026-05-20 orphan) — RFC 6716 §4.1 range decoder + §4.3 prefix + §4.3.2.1 coarse-energy scaffold + §4.3.3 bit-allocation fields + §4.3.4 tf_change/tf_select + r181 §4.3.4.3 spread parameter + Table 56/59 f_r + r187 §4.3.7.1 post-filter (taps F32+Q15, per-sample + slice apply, prev_output carry) + §4.3.7.2 de-emphasis (α_p=0.8500061035, F32+Q15; empirical: RFC 0.2170410156 quantizes to Q15=7112 not 7113); 142 tests; blocked on docs #936 (Laplace) + #943 (cache_caps50 / LOG2_FRAC_TABLE / alloc loop) | 🚧 scaffold |
| **Speex** | 🚧 r9 — Ogg stream-header + NB + WB high-band + §5.5 in-band signalling + r165 typed packet → frame iterator + r179 `BitWriter` + r187 encoder-side `write` symmetric to `parse` + r191 22 CELP companion-table accessors (5 NB LSP-VQ stages + pitch-gain VQ 5b/7b + 6 NB innovation codebooks + 2 WB high-band MSVQ stages + 2 WB innovation + Q15 LPC analysis window + autocorr lag + QMF h0; OnceLock-backed `&'static [Row]`; closes #969); 156 tests; lacks innovation dispatcher + LSP→LPC + synthesis + UWB framing | 🚧 scaffold |
| **GSM 06.10** | 🚧 ~25% — r185 clean-room §5.3 fixed-point RPE-LTP decoder pipeline (§5.1 saturating primitives + §5.4 Tables 5.1–5.6 + §1.7 Table 1.1 76-param 260-bit unpack + LAR/LTP/RPE synthesis + de-emphasis + §5.3.7 output shape; 34 tests; per-container framing for `.gsm` / RTP / MS-GSM WAV still DOCS-GAP); lacks §6 conformance vectors + encoder | 🚧 scaffold |
| **G.711** (μ/A-law) | ✅ 100% | ✅ 100% |
| **G.722** | 🚧 r185 clean-room SB-ADPCM decoder bring-up against staged ITU-T G.722 Recommendation (Table-14 column tables Q6/QQ6/etc. from the spec, not C reference) | 🚧 scaffold |
| **G.723.1** | ✅ 100% | ✅ 100% — both 5.3k + 6.3k |
| **G.728** | 🚧 r189 ~5% — clean-room rebuild advance from scaffold to decoder front-end: Table 1 const inventory + Annex A.1/A.2/A.3 (105/34/60 Q15) + Annex B 128×5 Q11 shape codebook + GQ/GB/G2/GSQ gain codebooks + Annex C FACV/FACGPV/WPCFV/WZCFV/SPFPCFV/SPFZCFV (51+11×5 Q14) + Annex D 1 kHz lowpass + block-50 Levinson-Durbin + blocks 29/31/32 (codebook + gain + 50th-order all-pole synthesis w/ §5.13 saturation); 48 tests; lacks blocks 30/33 backward adapters + postfilter | 🚧 scaffold |
| **G.729** | 🚧 ~5% — clean-room from staged trace #859: r173 numeric tables + r189 7 more tables (LPC Hamming 240 + lag windows + LSF cos 61 + pitch interp + MA gain Q13) + r191 ITU serial bitstream parser + conformance-corpus harness (`SYNC=0x6B21` / 80-bit frame / `BIT_ZERO=0x007F` / `BIT_ONE=0x0081` / `BIT_ERASED=0x0000` empirically observed from staged `.bit` corpus; `serial::parse_frame → FrameKind::{Active([bool;80]), Erased}`; pins ERASURE 60/300, OVERFLOW 1/384, PARITY 0/300 identical on Annex A); 45 tests; lacks LSP L1/L2 codebooks + gain GA/GB + postfilter + Annex B DTX | 🚧 scaffold |
| **IMA-ADPCM (AMV)** | ✅ 100% | ✅ 100% |
| **MS-ADPCM / IMA-ADPCM (WAV)** | ✅ 100% | ✅ 100% — block-aligned WAV encoder for both nibble layouts |
| **OKI / Dialogic VOX** | ✅ 100% — r186 clean-room from Dialogic app note 00-1366-001 (1988); HiFirst (VOX/MSM6295) + LoFirst (MSM6258) nibble orders, Native12 + Wide16 output | ✅ 100% — symmetric §3 closed-form encode; mono-only via registry (Dialogic hardware constraint) |
| **8SVX** | ✅ 100% | ✅ 100% |
| **iLBC** (RFC 3951) | ✅ 100% — NB 20/30 ms | ✅ 100% |
| **AC-3 / AC-4** (Dolby Digital / Dolby AC-4) | ✅ ~97% — AC-3 full decode + E-AC-3 SPX (§E.3.6 HF regen) + transient pre-noise (§E.3.7.2 TPNP) + multichannel fbw+LFE+coupling AHT (§3.4 Adaptive Hybrid Transform) + §7.8.2 LtRt matrix downmix + r126 Annex D §2.3 alternate-syntax mix-level params + WAVE_FORMAT_EXTENSIBLE + r172 §3.6.4.2.3 SPX-attenuation border notch + r182 §7.10.1 opt-in decoder CRC verifier + r187 §7.10.1 augmented-form encoder crc2 (residue-zero on self-decode; eq. closure of r182 deferral); AC-4 ~98% decoder + IMS encoder ~65% + r187 ACPL_1 residual/α-β-writer combined-input desync pinned via 4 characterization tests (drift surface upstream of pair0 in write_acpl_1_residual_layer or write_two_channel_data, triggered only on simultaneous L+Ls non-trivial) | 🚧 AC-3 ~95% — acmod 1/2/2.1/3/6/7 + LFE + DBA + 5-fbw coupling + E-AC-3 indep+dep + per-channel PSNR gates + r95 two-stage equalise + spread-cap greedy for per-channel `fsnroffst[ch]` |
<!-- ac3 decode r129: E-AC-3 mixmdata mix-levels (ltrt/loro c/sur) now surfaced + routed through §7.8 downmix in process_eac3_frame -->
| **AC-4** (Dolby) | 🚧 ~98% — A-SPX + DRC + 60+ ETSI codebooks + 5_X/7_X ACPL_1/2/3 + cfg0/1/2/3 + LFE + SSF/SNF + SAP + Pseudocode 121 companding + IMS bitstream_version≥2 walker + r181 §5.7.7.7 Pseudocode 121 + §4.2.12.4 Table 52 SIGNAL band-count fix + r190 Table 126 `aspx_int_class = FIXFIX` writer width fix (was emitted as 2-bit `0b11` → VarFix prefix → 6-bit drift swallowed when α/β quantised to 0; closes r187 ACPL_1 desync at root cause via BitWriter/BitReader bit-position probes; 657 lib tests); lacks ETSI fixture RMS audit, object/a-joc | 🚧 IMS ~65% — v0/v2 TOC + mono/stereo/joint M/S CPE + 5.0/5.1/7.1 SIMPLE Cfg3Five + 5_X SIMPLE/ASPX_ACPL_1/2 + ASPX_ACPL_3 + r125 Cfg3Five immersive + r132/r135/r139/r144 real per-band α+β for ACPL_1 (5.0/7.0/7.1) + ACPL_2 (5_X); lacks ACPL_3 real β + ASPX envelope coding |
| **MIDI** (SMF) | ✅ ~99% — SMF Type 0/1/2 → PCM via 32-voice mixer + SF2/SFZ/DLS + r186 `cue_points()` FF 07 + r192 `SmfFile::track_names()` FF 03 (`TrackNameEvent { tick, track, text }` w/ same stable-sort cross-track merge as the other 6 text-meta helpers; filters only FF 03 excluding 01-07; `text_bytes()` raw + `text_lossy() → Cow<str>`; surfaces every occurrence not just per-track-at-tick-0); `SmfFile::{cue_points,markers,lyrics,tempo_map,time_signatures,key_signatures,track_names}` 7-iterator family; r172 cargo-fuzz (30M+ panic-free) | — synthesis only |
| **NSF** (NES) | 🚧 ~95% — full 6502 + IRQ/NMI + 5/5 2A03 APU + DMC DMA + six expansion chips + NSF v1/v2/NSFe + Dendy region + per-device gain + plst/psfx playlist + region-aware noise + FDS modulation/envelope + r154 Namco 163 + r182 VRC7 internal patch ROM + r185 VRC7 OPLL operator pipeline (§3 MUL + §5 FB + andete logsin/exp ROMs, 6-channel engine at 49.7163 kHz; §6 row-256 ground truth matches ±1 LSB; 170 tests). Lacks §4 KSL + §7 per-rate env tables (provenance-pending) + rhythm mode | — synthesis only |
| **Shorten** (.shn) | 🚧 r12 (post-2026-05-18 orphan) — `ajkg` magic + v2/v3 ulong + svar(n) + per-block function dispatch + VERBATIM/QUIT + DIFF0..3 + Rice residual + per-channel carry + spec/05 §2.5 running mean + QLPC predictor + r7 `decode_stream` + r145 `Decoder` trait + r181 block-by-block + r187 streaming `Decoder` + r191 first encoder-side surface (`BitWriter` MSB-first + `encode_envelope_stream` driver; syntactically-valid envelope-only file round-trips through r7 decoder; 9 integration tests); 189 tests; lacks DIFFn/QLPC predictor encoders + 8 unpinned TR.156 file-type labels (#1267 spec/04 §2 BLOCK_FN_QUIT encoding contradiction) | 🚧 scaffold |
| **TTA** (True Audio) | ✅ ~98% — TTA1 fmt=1/2 + password + ID3v1/APEv2 trailer + r156 9-class malformed-input property tests + r187 streaming + random-access decode API + r190 `streaming_decode` cargo-fuzz target (cross-API agreement vs eager `decode_all`; 10-byte seed-pack for target_frame/target_sample/start_index; 9 corpus seeds) | ✅ ~96% — TTA1 fmt=1/2 + password; bit-exact self-roundtrip |
| **APE** (Monkey's Audio) | 🚧 r190 Phase 1 — 8-byte `MAC ` magic + decimal-coded version + 5 compression-level enum prefix parser; per-version header tail + IIR coefficients + residual `k` recurrence + range-decoder bounds + channel decorrelation all DOCS-GAP | 🚧 scaffold |
| **Musepack** | 🚧 r191 — SV7 §2.5/§2.6 requantiser constants wired (RES_BITS[18] + Dc[19] + Cc[19] + SCF step ratio from `docs/audio/musepack/tables/`; `C·(2D+1)=65536` property bit-exact across all 18 non-CNS entries; 12 tests); lacks header parser + frame driver + Huffman codebooks (staged) + CNS generator (staged) + 32-band synthesis (#1263 header field-map observer trace pending) | 🚧 scaffold |
| **Cook** (RealMedia) | 🚧 r2 — flavor table + cookie parser + 8 DSP parameter tables (pow2_exponent 127 f32 bit-exact to `2^k`, sqrt2_scale_ladder 127 f32, gain_step_2pow_half + gain_bias_ramp 7 each, category_level_count `{13,9,6,4,3,2,1}`, reciprocal_1_over_n 11, category_index_lut 51, mdct_windows {3,7,15,31,64} validated via Princen-Bradley TDAC `w[k]²+w[N-1-k]²=1`); 22 tests; lacks MDCT inverse + spectral VLC + per-band dequant; multichannel `0x02000000` cookie shape DOCS-GAP | — |
| **WavPack** | 🚧 r14 (post-2026-05-18 orphan) — v4 block/metadata/decorrelation/entropy parse + LSB bit-reader + Golomb (base,add) interval + per-sample value reconstruction + r172 per-term `decorrelation_sample_count` + r179 `PackedSamples` view of 0x0A + r186 `parse_block` aggregate + r191 `AdaptiveMedians` §3 + §3.2 median-adaptation from staged spec (3 `u32` GET_MED state + `inc_median`/`dec_median` formulas + `Zone0/1/2/Overflow` typed `adapt(zone)`; saturating semantics; 4-step hand-computed sequence pin); 221 tests; lacks stereo per-term sample count / prediction loop / float+multichannel / CRC / encoder | 🚧 scaffold |
| **APE** (Monkey's Audio) | 🚧 r190 Phase 1 bootstrap (new crate) — 8-byte `MAC ` magic + decimal-coded version u16 + 5 compression-level enum (1000/2000/3000/4000/5000) prefix parser; 14 unit + 6 integration tests + standalone-build OK; per-version header tail (sound params/frame count/seek table/embedded WAV) + IIR coefficients + residual `k` recurrence + range-decoder bounds + channel decorrelation reconstruction all DOCS-GAP | 🚧 scaffold |
| **DTS** (Core) | 🚧 ~32% — frame-sync header + 14↔16-bit pack/unpack + r165 find_next_sync + r179 `iter_syncs` lazy iterator + r189 `frame_size_container_bytes` r192 `iter_frames_14bit(bytes) → FrameIterator14` walking 14-bit-packed container streams directly via `parse_frame_header_14bit` + container-byte advance + `FrameView14` container-domain semantics + `Error::UnsupportedRaw16Bit` symmetric to existing UnsupportedFourteenBit (10 unit + 2 integration repackaging bundled 5-frame fixture into 14b BE/LE; 204 default / 189 standalone tests); lacks §5.3.1 alloc/scfac + §5.4 polyphase filterbank | — |
| **aptX** (classic + HD) | 🚧 ~70% — 4-band QMF + ADPCM; bit-exact NDA-blocked + r189 RFC 2361 §A.24 `WAVE_FORMAT_TAG_APTX = 0x0025` IANA tag + `CODEC_ID_STR = "aptx"` registry (lets RIFF containers route 0x0025 → clean NotImplemented) | — |

</details>

<details>
<summary><strong>Video</strong> (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **MJPEG** | ✅ ~97% — baseline + progressive 4:2:0/4:2:2/4:4:4/grey + 12-bit YUV (baseline + r183 SOF2 P=12 progressive) + SOF9 arithmetic + lossless SOF3 P∈{2..16} 3-component decode + RFC 2435 RTP/JPEG depacketization + 6 cargo-fuzz targets + r190 §G.1.1 SOF2 4-component CMYK / YCCK progressive at P=8 (Adobe APP14 colour-transform honoured; tighter `Nf=4 & P=12` reject) | ✅ ~95% — baseline + progressive + lossless SOF3 grey/RGB + DRI/RSTn + non-zero point transform Pt 0..15 |
| **FFV1** | 🚧 ~73% — RFC 9043 decoder + demux + decode_frame driver (YCbCr + RGB Y/Cb bit-exact; RGB Cr divergence open) + r179 `coder_type==2` alternative state-transition table wired through decode + encode | 🚧 ~40% — Slice Footer + Slice Header + Golomb-Rice primitives + frame-level Golomb-Rice + YCbCr encoder + r164 range-coded SliceContent encoder + r179 derived-table encode path + r190 §4.7 `encode_frame_rgb` symmetric inverse for `coder_type ∈ {1,2}` (§3.7.1 forward RCT + §3.7.2.1 exception + §3.7.2 Cb/Cr offset + §4.9.3 CRC-by-construction; 357 tests); lacks Golomb-Rice RGB encode + RGB Cr fix |
| **MPEG-1 video** | 🚧 ~40% — sequence/GOP/picture/slice + macroblock walk + intra-DC + §2.4.3.7 dct_coeff walker + §2.4.4 dequantiser + r185 §A 8×8 IDCT + IEEE P1180/D2 conformance harness (PMSE/OMSE/PME/OME + deterministic checks all green) | 🚧 scaffold |
| **MPEG-2 video** | 🚧 ~58% — §6.2.x sequence/GOP/picture/slice + macroblock_type + cbp + macroblock_modes + motion_vectors + §7.6.3.x PMV + §7.6.4 forming-predictions pel reader + §7.6.7 combine + §7.6.8 add-and-saturate + r165 §7.6 driver + r179 §7.4 inverse-quantisation + r185 §A 8×8 IDCT + r192 §7.2.2 residual VLC walker (Tables B-14/B-15/B-16 + `TableSelection::from_context(intra_vlc_format, macroblock_intra)` per §7.2.2.1 Table 7-3 + `DctCoeffStep::parse(br, table, position)` with §7.2.2.2 NOTE 2/3 FIRST/NEXT alternates + table-dependent EoB + B-16 escape w/ forbidden wire words `0x000`/`0x800` rejected; 532 tests); lacks §7.2.1 intra-DC prelude + §7.3 alternate_scan | 🚧 scaffold |
| **MPEG-4 Part 2** | 🚧 ~58% — I-VOP intra + inter texture + §6.2.5 video_packet_header + §7.8.7.3 GMC + §6.2.2 VisualObject() + r163 §6.2.7 / §7.3 inter block driver + r182 §7.6.2.1 half-sample bilinear Figure 7-29 + r190 §7.6.2.2 quarter-sample interpolation Figs 7-31/7-32 + Table 7-13 chroma MV reduction (8-tap FIR `C=[160,-48,24,-8]`; arithmetic-shift handling for negative MVs; 472 tests); lacks B-VOP + encoder | 🚧 scaffold |
| **Theora** | 🚧 ~43% — §6.1–§6.4 setup-header + Appendix B.2/B.3 VP3-default tables + §6.4.x quant + DCT-token Huffman + §7.1–§7.5 frame walk + r160 §7.5 motion vectors + r165 §7.6 block-level qi decode + r179 §7.7.1 EOB Token decode + r185 §6.4.1 LFLIMS table decode + r191 §7.7.2 Coefficient Token Decode (Table 7.38 non-EOB 7..=31 + SIGN/MAG/RLEN 0-11 extra-bit + typed `CoefficientTokenKind::{ZeroRun, Single, RunPlusOne}` + MUST-NOT-overflow-64 fail-closed; 271 tests); lacks §7.7.3 driver + §7.8 DC prediction | 🚧 scaffold |
| **H.263** | 🚧 ~88% (post-2026-05-18 orphan) — §5.1-§5.4 baseline + §6 IDCT/MV/half-pel/INTER + Annex J §J.3 deblock + Annex I AIC + Annex D UMV + Annex F 4-MV + OBMC + §5.1.4 PLUSPTYPE + Annex K §K.2 Slice-Structured + r151 INTER4V driver + r181 absorbed-INTRADC + r187 §I.3 AIC reconstruction primitive + r192 §I.3 end-to-end INTRA-block reconstruction pipeline (`aic_intra_reconstruct_coefficients` composes modified inverse-quant + Figure-I.2 scan-selection scatter + DC/AC prediction reconstruction w/ clipAC/oddifyclipDC into one call — output also `Neighbour::Available` payload; `aic_intra_reconstruct_samples` runs §6.2.4 IDCT + §6.3.2 sample clip; closes 4 deferred §I.3 steps as pure-fn primitives; 358 tests); lacks MB-grid driver wiring + Annex K driver + PB-frames | 🚧 scaffold |
| **H.261** | ✅ ~98% — I+P QCIF/CIF + integer-pel + loop filter + BCH FEC + Annex B HRD + RFC 4587 RTP + RFC 3550 RTCP SR/RR/SDES/BYE/APP; r160 two cargo-fuzz targets + r189 §6.2.1 SDP offer/answer negotiation (`sdp::negotiate_answer`: picture-size intersection + `MPI = max(offer, our)` + D=1 iff both, RFC 2032 QCIF=1 fallback) | ✅ ~98% — spiral+diamond ME + GQUANT-from-bitrate + BCH framing + RTP wrap + RTCP compound build/parse; 45 dB at 64 kbit/s QCIF |
| **MS-MPEG-4** (v1/v2/v3) | 🚧 ~42% — clean-room scaffold; v3 intra 3-tier ESC + custom intra-DC VLC + G0..G3 LMAX/RMAX wired + v1/v2 CBPY VLC + spec/15 §3 provenance-pinned table + inter AC residual decode + r181 `GFamily` accessors + r185 Figure 7-34 MV-predictor walk + r191 1-MV predictor routed through `predict_block_mv(Block::TopLeft, …)` (closes r185 CHANGELOG queue; behavioural delta at §7.6.5 rule-3 corner where exactly one neighbour valid: predictor → that lone valid MV, not `median(only, 0, 0) = 0`; 291 + 113 tests). Still lacks G0..G3 primary canonical-Huffman bit-length array + alt-MV VLC + 4-MV MCBPC. VfW-sandboxed mpg4c32.dll runs in parallel | — |
| **H.264** | 🚧 ~80% — I/P/B + 4:2:0/4:2:2/4:4:4 + CAVLC + CABAC + DPB + 41 SEI types + fuzz-hardened + r183 SEI type 46 + r187 §8.2.1 POC i64-staged + `PocError::Overflow` + r192 ISO/IEC 14496-15 §5.2.4.1.1 strict avcC parser + High-family extension trailer (rejects `lengthSizeMinusOne == 2` up front; parses `chroma_format` + `bit_depth_luma_minus8` + `bit_depth_chroma_minus8` + SPS-Ext list for `profile_idc ∈ {100,110,122,144,244}`; §7.4.2.1.1 `bit_depth_*_minus8 ≤ 6` cap; 5 new avcc accessors; tolerant of muxers eliding the trailer; 1055 tests); lacks MBAFF, SVC/3D/MVC | 🚧 ~83% — I+P (1MV/4MV, ¼-pel) + B + CABAC at all chroma layouts + Trellis-quant RDOQ-lite (1227 tests); ffmpeg PSNR_Y 44.20 dB |
| **H.265 (HEVC)** | 🚧 ~48% — VPS+SPS+PPS bodies + scaling-list + scan + §9.3 CABAC engine + slice header through §7.3.6.3 pred_weight_table() in-place + r182 §7.3.6.2 ref_pic_lists_modification() + r190 §7.4.8 inter-RPS-prediction typed builder + slice wiring (eq 7-59 RefRpsIdx + eq 7-60/7-61/7-62 sign-gated cumulative recurrence; every conformant non-IDR P/B slice now parses end-to-end; 215 tests); lacks §9.3.4.2 binarization+ctxIdx (#444) + residual/IDCT | 🚧 scaffold |
| **H.266 (VVC)** | 🚧 ~68% — 4:2:0 IDR intra + ALF/SAO/CC-ALF + P/B merge+skip + HMVP + MMVD + CIIP + BCW + BDOF + GPM + AMVR + HBD + DMVR + affine sub-block MC + PROF + AMVP + SbTMVP + merge_subblock + §7.3.11.7 non-merge affine-syntax dispatcher + r181 bitstream VPS §7.3.2.3 (~1057 tests); lacks non-merge inter CU walker call-site + multi-layer VPS | 🚧 ~88% — forward CABAC + DCT-II + SAO/ALF/cu_qp_delta + MTT BT+TT RDO + P+B + sub-pel MC + multi-ref DPB + weighted bi-pred + r177 affine + r183 MVP-side syntax + r190 §7.3.11.7 `encode_non_merge_inter_pre_residual` composite walker (folds r177 affine + r183 MVP + r187 MVD-coding; SMVD `MvdL1 = -MvdL0` per §8.5.2.5; 1082 lib tests); lacks numCpMv>1 + amvr_flag + bcw_idx |
| **VP6** | 🚧 r15 — §9 raw-bit frame-header prefix + §15/§16 IDCT + §17.x intra/inter reconstruction + §11 deblock/interp + §14 DC prediction + §10 mode tables + §13 DCT-token static tables + r179 §13.3.3 AC zero-run + r186 §3 R(x) RawBitReader + r191 §7.3 BoolCoder per errata #35 (`Split = 1+(((Range-1)·P)>>7)` w/ renorm + `b(1)`/`b(n)` fixed-prob-128 MSB-first; closes the §7.3 BoolCoder gap; 319 tests); unblocks §10 mode-tree + §11 MV + §13 DCT-token walks | 🚧 scaffold |
| **VP8** | ✅ 100% | ✅ 100% |
| **VP9** | 🚧 ~40% — §6.2 walk + §9.2 Bool decoder + §6.3 compressed-header sweeps + §6.4.24 coeff + §8.6 dequant + §8.7 inverse transforms + §8.5.1 intra pred + §8.6.2 reconstruct + §6.4.3 decode_partition + §6.4.13 read_is_inter + §6.3.9-14 prob sweeps + r183 §6.3.17 update_mv_prob primitive (402 tests); lacks §6.3.12 frame_reference_mode + §6.3.16 mv_probs outer driver + §6.4.4 decode_block + §8.4 loop filter | 🚧 scaffold |
| **AV1** | 🚧 ~94% — decoder feature-complete + **standalone `decode_av1` public entry** (1489 tests + integration roundtrips) | 🚧 ~24% encoder — pixel-space YUV→IVF driver (4:2:0 intra at 16/32/64 frame sizes) with **14-mode intra picker (luma + chroma incl. UV_CFL_PRED §7.11.5.3)** + full §7.13.3 forward 2D dispatcher (DCT/ADST/FLIPADST/IDTX all square sizes 4..64) + WHT lossless arm + forward quantize + §5.11.4 partition + §5.11.36 transform_tree + §5.11.39 coefficient encode. **Encoder→`decode_av1`→pixels lossless roundtrip bit-exact at 16×16 / 32×32 / 64×64**. Lacks rectangular TX sizes + §5.11.18 inter mode_info + RD |
| **Dirac / VC-2** | ✅ ~95% — VC-2 LD+HQ intra + Dirac core-syntax intra/inter + OBMC + 7 wavelets + 10/12-bit + bit-exact intra fixtures + r165 fuzz oracle + 4 robustness fixes + r190 Criterion bench harness (decode / encode / roundtrip; 316 tests) | 🚧 ~95% — HQ+LD intra + Dirac core-syntax + adaptive sub-pel + 2-ref bipred + post-OBMC refinement + picture/sequence rate-control + r159 VbvHysteresis drain-cap + r179 rate-control fuzz oracle (13 tests) |
| **AMV video** | 🚧 scaffold (orphan rebuild post-audit 2026-05-18; clean-room re-implementation pending) | 🚧 scaffold (orphan rebuild post-audit 2026-05-18; clean-room re-implementation pending) |
| **ProRes** | ✅ ~96% — RDD 36 entropy + 8/10/12-bit + 4:4:4:4 alpha + interlaced + RAW refused; ffmpeg interop 60-68 dB + cargo-fuzz harness (3 panic-free targets) + r161 profiling + r185 `idct8x8_dc_only` decoder fast path | ✅ ~96% — RDD 36 all 6 profiles + interlaced + alpha + perceptual quant matrices + multi-frame rate-control + 12-bit cross-decode + ffmpeg acceptance + r187 `fdct8x8_constant` encoder fast path (RDD 36 §7.4 + §7.5.1 pad-block closed-form `DC=8*v, AC=0`; bit-identical to general path; fires on sample-replication pads + flat regions) |
| **EVC** (MPEG-5) | 🚧 ~85% — NAL + SPS/PPS/APS + §9.3 CABAC + §8 intra (Baseline) + DCT-II + P/B inter + RPL + HMVP + DPB + ALF + DRA + IBC §8.6 + r148 §8.9.5 chroma DRA + r151 §7.3.6-faithful dra_data() parser + §7.4.7 derivation + r187 §8.9.7 chroma DRA derived-state (`DraChromaDerived` w/ out_ranges/scales/offsets) + §8.9.6 chromaScale entry on `DraJoinedScaleFlag=0` unjoined branch (§8.9.8 eq. 1394); 389 tests; lacks §8.9.8 joined branch (eq. 1395-1419 + ScaleQP/QpScale tables) + Main-profile toolset (BTT/ADMVP/EIPD/ATS/AMVR/affine) | — |
| **HuffYUV** / FFVHuff | ✅ ~96% — HFYU + FFVH FourCCs + 6 predictors + 8-bit only + interlaced field-stride=2 + fast-LUT decoder + SWAR 8-byte gradient post-pass + r181 YUY2 LEFT macropixel-step branch-free decoder | ✅ ~96% — full encoder symmetry × YUY2/RGB24/RGB32 + v1.x + v2.x ClassicV2/CustomV2 + walking-stride interlaced + predictor auto-selection + r95 SWAR forward-gradient encoder + r100 fused LEFT+decorrelation residual + r103 GradientDecorr fusion + r115 single-pass forward-MEDIAN fusion + r181 YUY2 LEFT forward branch-free encoder + r186 `forward_rgb_left_subtract_linear` single-stride RGB24/RGB32 LEFT-residual walk (~20× M1 across 320×240..1280×720; LLVM autovectorises to NEON `vsubq_u8` / SSE2 `psubb`) |
| **Lagarith** | ✅ ~95% — all 11 wire types + modern range coder with spec/02 §5 three-way fast path + legacy adaptive-CDF + Fibonacci-Zeckendorf prefix + JPEG-LS Median + G-pivot decorr + zero-run RLE + pair-packed 513-entry CDF (Strategy F, decode-only for proprietary type-7 streams) + modern RGB(A) first-column predictor Rule B (spec/06 §3.2, byte-exact vs ffmpeg lagarith decoder) | 🚧 ~76% — encoder for SOLID/RGB/RGBA/YV12/YUY2/legacy-RGB + spec/02 §5 Step-A + Step-B + Step-C `freqs[]` cache (1.08× on Step-C-heavy fixtures, 244 MSym/s) + r135 modern-coder q≥1 frequency rescale (>TOP-pixel planes now encodable) + r138 per-channel header-form selection across all 8 wire forms (0x00..0x07 + 0xff; 37% smaller wire on residual profile) + r141 legacy-fork per-channel header-form selection (`encode_legacy_channel_best` + `encode_legacy_rgb_best`; never-worse defensive guarantee — bit-packed Fib layout yields zero 0x00 so RLE escape never fires, selector ties bare-Fib); byte-exact vs proprietary encoder Auditor-blocked |
| **Ut Video** | ✅ ~97% — 5 native FourCCs × 4 predictors + RGB inter-plane decorrelation + LUT-accelerated canonical Huffman + slice-parallel decode (5.63× at 720p) + criterion baseline + r186 `Decoder` trait factory reads `CodecParameters::tag`/`extradata`/`width`/`height` (malformed → InvalidData at construction; legacy `configure()` path preserved) | ✅ ~96% — slice-parallel encode (3.28×) + content-fixture corpus + r161 cargo-fuzz oracle |
| **MagicYUV** | ✅ 100% — 17 v7 FOURCCs + Median + JPEG-LS Median (HBD) + raw-mode + interlaced + r130 `decode_into(&mut DecodedFrame)` streaming entry point + r186 `HuffmanTable::build` opt-9 (HashMap→direct-indexed `Vec<i32>` for two-level path; `core::mem::take` for `start`; observable table byte-identical); trace JSONL strict-jq-line-diff-equal to cleanroom Python ref | ✅ 100% — `encode_frame` across all 17 FOURCCs + spec/04 §3 Dynamic + spec/05 §6.2 Auto Huffman/raw + length-limited Package-Merge Huffman + r127 decoder packed `Vec<u32>` + r136 daily cargo-fuzz (~980k exec/60 s, 0 crashes) |
| **Cinepak** (CVID) | ✅ ~98% — frame header + multi-strip + V1/V4 codebooks + intra/inter + grayscale + Sega FILM demuxer + Saturn/3DO deviant + r181 codebook_chunk_apply + r192 `decode_vector_chunk` cargo-fuzz target + criterion benches | ✅ ~98% — stateful encoder + rolling codebooks + RDO + LBG + 3-axis grid picker + bitrate-target rate-control + keyframe-interval (34.18 dB PSNR; decode 4.4 GiB/s, stateful GOP 13.5 ms/frame) |
| **SVQ1/SVQ3** (Sorenson) | 🚧 r8 (orphan rebuild) — SVQ1 frame-header + framework registry + SVQ3 SEQH + slice + MB-type tree + residual walker + r179 SVQ3 P/B-frame inter-MB motion-vector precision selector + r191 SVQ1 block-tree subdivision walker (`Svq1Level::{L0..L5}` + `read_block_decision` + L=4/L=5 0-bit-reject per newly-staged §14.10/§14.11 RESOLVED-absent); 181 tests; lacks L=0..L=3 internal codebook layout + SVQ3 MV-VLC + #1256 svq3.c attribution scrub | — |
| **Indeo 3** (IV31/IV32) | 🚧 r13 — clean-room codec-frame header + bitstream header + spec/02 picture-layer + spec/03 macroblock-layer + spec/04 VQ codebook + spec/06 byte-level entropy + spec/07 output-reconstruction + four cell-shape kernels + spec/02 strip-context array + spec/03 per-cell sub-array wiring + r181 spec/05 §1 mc_table layout + dispatch/index-fetch/index-validity + r186 spec/05 §2.2/§2.3/§3.3/§3.4 packed-MV bit-layout (`PackedMv::from_raw`/`pixel_offset`/`mode`/`source_address`, `McDispatchMode` four-way fork with per-variant RVA; 232 tests); lacks cell-stack pre-population (spec/03 §6 Q4) + pixel-buffer edge fix-up + MC inner loop (spec/05 §5.x) | — |
| **Indeo 2/4/5** | 🚧 scaffold — pending clean-room workspace; Indeo 4/5 still sandboxed via `oxideav-vfw` | — |

</details>

<details>
<summary><strong>Image</strong> (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **PNG / APNG** | ✅ 100% — 5 colour types × 8/16-bit + APNG + sBIT/pHYs/tIME/bKGD/hIST/eXIf/sRGB/cICP/sPLT + r154 Criterion benches + r183 tRNS keyed transparency promotion for ct=0/2 (8+16-bit) per RFC 2083 §4.2.9 + structural rejection of prohibited tRNS on ct=4/ct=6 | ✅ 100% |
| **GIF** | ✅ 100% — 87a/89a + LZW + interlaced + animation + disposal compositor + structured Application Extensions + Plain Text Extension + lenient mode + lazy Playback + animation-timing accessors + fluent AnimationBuilder; clean-room from CompuServe spec + r153 tracked spec-derived fuzz seed corpus (5 seeds × 3 targets) | ✅ 100% — per-frame palettes + `optimize_color_tables()` GCT/LCT hoisting + §7 Required Version enforcement + `upgrade_version_if_needed()` |
| **WebP** (VP8 + VP8L) | ✅ 100% | ✅ 100% |
| **JPEG** (still) | ✅ ~95% — via MJPEG | ✅ ~90% — via MJPEG |
| **TIFF** (6.0) | ✅ ~97% — II/MM + BigTIFF read + 7 photometrics (incl. PI=4 Transparency Mask r172) + 1/4/8/16-bit + None/PackBits/LZW/Deflate/CCITT-MH/T.4-1D + FillOrder + tiles + multi-page + JPEG-in-TIFF (incl. CMYK-JPEG: Compression=7 + Photometric=5 + SamplesPerPixel=4) + PlanarConfiguration=2 (separate component planes across strips/tiles + chunky re-interleave + Predictor=2 driven per-plane) + cargo-fuzz decoder (panic-free, 7.7 M iter green); lacks CCITT T.4 2-D / T.6 (#874), JPEG-in-TIFF + planar=2 | ✅ Gray8/16/RGB24/Palette8 — None/PackBits/LZW/Deflate + Predictor=2 + PlanarConfiguration=2 separate-planes write (Rgb24 × None/PackBits/LZW/Deflate ± Predictor=2) + Bilevel CCITT-MH / T.4-1D, single+multi-page + tiled chunky write (Gray8/16/RGB24/Palette8 × None/PackBits/LZW/Deflate ± Predictor=2, §15) + tiled PlanarConfiguration=2 write (Rgb24, one grid per plane, §15) |
| **BMP** | ✅ ~96% — 1/4/8/16/24/32-bit + V4/V5 + OS/2 BITMAPCOREHEADER + RLE4/RLE8 + top-down + daily fuzz CI (2 targets: decode + r162 `rle_stream` RLE-state-machine focus at ~72k execs/sec) + 31-test property-test sweep | ✅ ~96% — top-down + minimal `biClrUsed`-trimmed palette encoder |
| **Netpbm** (PBM/PGM/PPM/PNM/PAM) | ✅ ~95% — all 8 magics at 1/8/16-bit + 6 PAM TUPLTYPEs + r171 cargo-fuzz harness + decoder pre-allocation OOM hardening | ✅ ~95% |
| **ICO / CUR** | ✅ ~97% — multi-res + BMP/PNG sub-images + CUR hotspot + ICONDIRENTRY validation (bReserved / dwBytesInRes / overlap-with-directory / cross-entry payload-overlap / overflow / wPlanes / wBitCount / CUR hotspot-in-bounds) + `select_best_fit` / `select_largest` / `select_by_dimensions` resolution helpers + 256×256 PNG round-trip + write 1..=256 dimension guard + `.ani` RIFF/ACON detection | ✅ ~92% |
| **JPEG 2000** | 🚧 r14 (post-2026-05-20 orphan) — T.800 main-header + SOT/SOD + typed COC/QCC/POC/RGN/PLT/PPT + JP2 box + §B.10 tier-2 + §B.5 ResolutionLevel + §B.6 precinct + §B.7 code-block partition + Annex C §C.3 tier-1 MQ + Annex D 19 contexts + §B.12.1 5 packet-progression iterators + §B.12.2 POC + r181 Annex F.3 inverse DWT + r187 4 cargo-fuzz targets + r192 §B.7/§B.9 + Annex E code-block→sub-band reassembly (`reassemble_subband_5x3` reversible scatter + `reconstruct_reversible` truncate-toward-zero i32; `reassemble_subband_9x7` irreversible Equation E-6 via `reconstruct_irreversible` f64; `SubBandQuantization::resolve` Eqs E-2+E-4 one-shot; `BlockSource` trait + per-resolution helpers ready for `dwt::sr_2d_*`; 352 tests); lacks MCT (Annex G) + per-coefficient Nb + per-resolution cascade + HTJ2K Part-15 | 🚧 scaffold |
| **JPEG XL** | 🚧 ~92% — ISO/IEC 18181-1:2024 lossless Modular path + 7 fixtures pixel-correct + VarDCT scaffold + Gaborish/EPF/AFV pure-math complete + §C.8.3 per-block HF coefficient loop + r164 typed TransformType + r177 NonZeros grid + r183 per-channel + r190 `PerPassNonZerosGrids` per-pass container + r191 WP trace oracle isolating #799 noise-64×64 divergence to UPSTREAM WP state evolution (production `wp_predict` matches staged trace at sample 194 EXACTLY with spec-conformant inputs: subpred `[1248, 747, 420, 559]` + prediction 709 + max_error 737; bisect roadmap pin: `Δ te_w = +21`, `Δ te_nw = -21` symmetric pair + `Δ err_sum_0 = 0`; ~641 tests); lacks upstream WP state-evolution fix + §C.7.2 histograms + per-frame Gaborish/EPF/CfL wiring | — retired |
| **JPEG XS** | 🚧 ~82% — ISO/IEC 21122 Part-1 + 5/3 DWT + Annex C/D/F/G + multi-component + CAP-bit + Cw>0 + Sd>0 cascade + high bit depth B[i]∈8..16 + r143 Annex A conformance + r190 4:2:0 chroma at NL,y≥3 picture-β common-slot enumeration (`picture_beta_to_local_beta` per Annex B.3 Figure B.2 NOTE 3 — chroma missing slots at `β = NL,x + {2,3} − NL,y`; closes corrupt-LH/HH triple at higher decomposition levels) | 🚧 ~82% — Nc 1/3/4 + Sd>0 + RCT + Star-Tetrix + NL up to 8 + odd dims + vertical prediction + significance coding + per-band Q + NLT + r151 4:2:2/4:2:0 sub-sampling at B[i]∈9..16 + r181 Annex G.4 high-bit-depth NLT quadratic (308 tests); lacks Star-Tetrix high-bit-depth + NLT extended high-bit-depth |
| **AVIF** | 🚧 ~85% — HEIF→AV1 + grid + imir/clap/colr/pixi/pasp + HDR metadata + AV1 wrap pass-through + DoS caps + HEIF item-properties + auxC URN routing + rloc / lsel / iovl / grpl + `mif1` compliance + a1op/a1lx + r130 tmap §4.2.2 + r172 §7 grid + r182 av1-avif §2.1 SH OBU + r188 ISO 21496-1 Annex C.2 `GainMapMetadata` parser + r190 `gain_map_metadata(file, tmap_id)` one-shot accessor folding iloc + parse (54 + 217 tests both feature sets); AV1 pixel decode + gain-map composition gated on sibling rebuild | — |
| **DDS** | ✅ ~99% — DDS_HEADER + DXT10 + uncompressed (10 layouts) + BC1-7 + BC6H all 14 modes + mipmap + 6-face cubemaps + DX10 arrays + volume textures + 132-entry DXGI table + daily cargo-fuzz + r162 40-case injection-robustness + r176 saturating-math + r192 Criterion benches (decode BC1-5 @ 512² + BC6H/BC7 @ 256², encode BC1-5 @ 256² + BC6H/BC7 mode-pickers @ 128², roundtrip A8R8G8B8 single+9-mip + DXT10 R8G8B8A8_UNORM + L8 separating container vs per-block hot path; xorshift-seeded synthetic inputs no binary fixtures) | ✅ ~95% — uncompressed + BC1-5 + BC7 all 8 modes + BC6H_UF16/SF16 all 14 modes + box-downsample mip chains + cubemap/array |
| **OpenEXR** | 🚧 ~85% — magic + 8 required attrs + HALF/FLOAT/UINT + NO_COMPRESSION/ZIP/ZIPS/RLE + tiled ONE_LEVEL + sub-sampled chroma + single-part deep scanline + multi-part deep scanline + r130 single-part deep tiled + r181 multi-part deep TILED + r192 multi-part flat (non-deep) TILED read (`parse_exr_multipart_tiled` linear-scan robust to zero-filled offset tables; `parse_exr_multipart` routes `type="tiledimage"` parts to new entry; ONE_LEVEL + NONE/ZIP/ZIPS/RLE; edge-tile aware); PIZ blocked on docs trace | ✅ ~93% — RGBA scanline + ZIP/ZIPS/RLE + tiled-output ONE_LEVEL/MIPMAP/RIPMAP + multi-part scanline + deep scanline + r130 single-part deep tiled + r181 multi-part deep TILED + r192 multi-part flat TILED write (`encode_exr_multipart_tiled` + `MultipartTiledPart` w/ version-bit `0x1000` no `single_tile` + per-part `tiles[tiledesc]` + chunk `i32 part_number` prefix; validated via exrheader + exrmultipart -separate pixel-exact) |
| **Farbfeld** | ✅ 100% — streaming reader + DoS hardening (dimension overflow + truncated payload guards) + `magick` black-box cross-validator | ✅ 100% |
| **HDR** (Radiance RGBE) | ✅ ~99% — new-RLE + old-RLE + 8 axis-flag combos + shared-exponent + multi-record EXPOSURE/COLORCORR + typed COLORCORR/PRIMARIES/VIEW + apply_exposure/apply_colorcorr + r189 luminance_lm_per_sr_per_m2 + r192 committed-fixture regression anchors (3 clean-room synthetic `.hdr` ~4.6 KiB: new-RLE+`\n` baseline, old-RLE+every typed slot, CRLF+`+Y H +X W`+PIXASPECT+untyped extra; decode + re-encode byte-identity per fixture; `examples/gen_fixtures.rs` regenerator) | ✅ ~98% — new-RLE + old-RLE + auto-RLE + XYZE↔RGB + 8 tonemap ops + CRLF + r179 zero-copy `reorient_for_axis_flags` (~6% encode throughput at 1024²) |
| **QOI** | ✅ 100% — byte-exact vs all 8 reference fixtures + criterion decode bench (540 MiB/s gradient, 1.55 GiB/s solid-RUN) + r162 second cargo-fuzz target encode_roundtrip (5 seeds, 33k local iters clean) | ✅ 100% — byte-exact vs reference encoder + criterion encode bench (640 MiB/s gradient, 2.13 GiB/s solid-RUN) |
| **TGA** | ✅ 100% — types 1/2/3/9/10/11 + TGA 2.0 extension + thumbnail + developer area + CCT + scan-line table + typed AttributesType alpha + r188 image-descriptor bit-4 right-to-left column ordering (composes w/ bit-5 row flip → 180°); magick cross-validated + r154 cargo-fuzz daily decode harness | ✅ 100% — all six image types + full TGA 2.0 extension + thumbnail + RGB24-input entry points |
| **ICER** (JPL) | 🚧 ~78% — Mars-rover heritage; bit-plane scan + compressed/uncompressed segments + 8 filters + IPN 42-155 §III.B context model + r192 §III.E lenient multi-segment decode (`parse_icer_lenient` / `parse_icer_lenient_with_limits` for DSN-packet-loss spaceflight scenario — `LenientDecode { image, received, missing_count }`; segment 0 required to pin canonical strip dims; missing strips reconstruct as flat 128 matching r6 ROI placeholder; trailing-drop truncates; +9 integration tests) | ✅ ~82% — quota encoding + auto wavelet selection + R-D byte-budget + r189 per-segment §III.D uncompressed fallback |
| **WBMP** | ✅ 100% — Type 0 + WbmpLimits DoS caps + adversarial fuzz sweep + r189 caller-selectable `MonoBlack`/`MonoWhite` decode polarity (`parse_wbmp_as` + `CodecParameters::pixel_format` routing) | ✅ 100% |
| **PCX** (ZSoft) | ✅ ~97% — 1/2/4/8 bpp planar + packed-bits + 24 bpp RGB planar + grayscale flag + DCX multi-page + DCX `Demuxer` + r136 fuzz-hardened (40M exec/0 crashes) | ✅ ~92% — 8 write paths + DCX; r185 framework `Encoder` widened to Rgba/Rgb24/Gray8 + Bgr24/Bgra/MonoBlack/MonoWhite (byte-swap + alpha-drop for BGR; MSB-first 1-bit unpack with MonoWhite polarity inversion per §4.1) |
| **ILBM** (Amiga IFF) | ✅ ~94% — BMHD/CMAP/CAMG/BODY + ByteRun1 RLE + EHB + HAM6/HAM8 + PBM + SHAM + PCHG + ANIM op-0/op-5 + CRNG/CCRT + DRNG (DPaint IV extended range, true-colour + register cells); lacks ANIM op-7/op-8, DEEP true-colour | ✅ ~84% — IlbmMuxer parity + masking + ANIM op-5 + CRNG/CCRT/DRNG encoder |
| **PICT** (Apple QuickDraw) | ✅ ~98% — v1 + v2 opcode walkers + drawing rasteriser + DirectBitsRect packType 0/1/2/3/4 + Region + clip-region + pen-size aware + Compressed/UncompressedQuickTime opcode skip + monochrome stipple + PixPat colour 8×8 type 1/2 + r186 indexed PixMap variant of `BitsRect/BitsRgn/PackBitsRect/PackBitsRgn` (0x0090/91/98/99 rowBytes-high-bit dispatch per §A-3 footnote §; 1/2/4/8 bpp + ColorTable + Region trailers; oob-palette → BLACK per §4); lacks text rasterisation + embedded JPEG decode | ✅ ~93% — `PictBuilder` + every v2 drawing-command family + state opcodes + mono+PixPat pattern setters + DirectBitsRect packType 1/2/3/4 + BitsRgn / PackBitsRgn; magick cross-decode bit-exact |
| **SVG** | ✅ ~99% — full shape set + path + gradients + text + mask + clipPath + use/symbol + svgz + SMIL animate/set/animateTransform + CSS3 Selectors L3 + `@import` + `@font-face` + `@keyframes` + Media Queries L4 + viewBox + 17 filter primitives + CSS Values L4 LengthUnit + CSS Easing L2 + SVG 2 §9.6.1 pathLength + SVG 2 §16.3 `<view>` element + fragment-identifier routing (`#MyView` / `#svgView(...)` + percent-decode + spatial/temporal media-fragment fallthrough) + SVG 2 §5.7 `<switch>` conditional processing (requiredExtensions / systemLanguage) + SVG 2 §13.7.1 `<marker>` typed def capture (refX/refY geometric keywords + markerUnits/orient + verbatim round-trip) + SVG 2 §13.2 `context-fill`/`context-stroke` + SVG 2 §16.5 `<a>` hyperlink (renders as group; link target + HTML attrs preserved across round-trip) + SVG 1.1 §11.5 `display` / `visibility` property handling + SVG 2 §5.8 `<title>` / `<desc>` + §5.9 `<metadata>` capture (multilingual lang, round-trip via PreservedExtras) + r172 SVG 2 §11.10.1.1 text-anchor (start/middle/end, inherited) + §11.8.3 textPath start-offset bias | ✅ ~88% — round-trips full shape graph + PreservedExtras side-channel + `<view>` re-emit at trailing edge |
| **PDF** | ✅ ~99% — bytes → Scene via xref/xref-streams/ObjStm + `/Prev` incremental + `/Encrypt` R=2..6 + public-key + PKCS#7 + `/Sig` AcroForm + Doc-Timestamp + text extraction + Linearization + Tagged-PDF + EmbeddedFiles + §12.6 actions + 5 stream filters + §8.11 Optional Content + content-stream cs/CS + §7.5.8.4 hybrid-reference + r145 cargo-fuzz + r148 criterion benches + r151 §7.5.7 ObjStm resolver cache (3.10 → 54.6 MiB/s, 17.6×) | ✅ ~99% — PDF 1.4/1.5 multi-page + paths/gradients/opacity/clip + RGBA + xref-stream + ObjStm + Linearization writer + `/Encrypt` + public-key + `/Sig` + AcroForm + annotation writer + embedded files + RFC 3161 Document Time-Stamp writer |

</details>

<details>
<summary><strong>3D scenes & assets</strong> (click to expand)</summary>

> The typed Scene3D / Mesh / Material PBR / Skin / Animation / Camera / Light / AudioEmitter model lives in `oxideav-mesh3d`, with `Mesh3DDecoder` / `Mesh3DEncoder` traits and a `Mesh3DRegistry` that's parallel to `oxideav-core::CodecRegistry`. Per-format crates register into it. `oxideav-meta::populate_mesh3d_registry(&mut Mesh3DRegistry)` walks every enabled format's `register()`. Lazy bytes flow through `AssetSource` (with a `raw_storage` pass-through hook for archive-backed sources, e.g. ZIP-stored USDZ textures + audio).

| Format | Decode | Encode |
|--------|--------|--------|
| **STL** (ASCII + binary) | ✅ ~99% — ASCII + binary + per-face attrs + 16-bit colour + multi-`solid` + topology + 7-step repair pipeline + ASCII comment preservation + daily cargo-fuzz + r161 Criterion + r189 `chunks_exact(50)` + `unpack_triangle_record` (binary decode 7.71 GiB/s @100K tris, +2% vs r175 encoder pack-record symmetry) | ✅ ~99% — both formats + attribute pass-through + `EncodeStats` + configurable float precision |
| **OBJ** (+ MTL) | ✅ ~98% — full Wavefront grammar + MTL (Phong + Wavefront-PBR + map_* options + typed refl) + smoothing/display attrs + free-form geometry pass-through + `xyzrgb` per-vertex colour + Bezier/B-spline/NURBS/Cardinal/Taylor `curv` + `surf` 2D-surface tessellation + r171 cargo-fuzz harness + r188 `curv2` 2D trimming-curve tessellation (trim/hole/scrv parameter-space curves → LineStrip via existing evaluators; rational weights + parm-u windows + negative indices); lacks surface-clip-against-trim-loops, multi-patch decomposition | ✅ ~96% — symmetric + negative-index encoder + polyline rejoin |
| **glTF 2.0** (+ .glb) | ✅ ~96% — JSON + .glb + full PBR + 12 KHR_materials extensions (unlit/emissive_strength/ior/specular/clearcoat/sheen/transmission/volume/iridescence/anisotropy/dispersion + r164 diffuse_transmission) + skin + skeletal animation + sparse accessors + morph-targets + 12 spec-MUST validators + KHR_texture_transform + JSON fuzz hardening + r188 KHR_mesh_quantization decode (BYTE/UBYTE/SHORT/USHORT POSITION/NORMAL/TANGENT/TEXCOORD_n dequant gated on extensionsUsed + allowed combo; FLOAT-only byte-identical); lacks KHR_audio_emitter + quantized morph-targets + encoder emission | ✅ ~91% — symmetric + sparse-encoding heuristic + signed+unsigned normalised-int quantisation + KHR_materials_unlit emit |
| **USDZ** (+ USDA) | ✅ ~93% — ZIP STORED walker + USDA parser + UsdGeomMesh + UsdPreviewSurface PBR + UsdUVTexture pass-through + xformOp transforms + UsdMediaSpatialAudio + variantSet + LIVRPS variant-selection composition + composition-arc round-trip + in-archive sublayer + references/payload arc composition + r180 in-layer `inherits`/`specializes` class-arc composition + r188 reader-side CRC-32/ISO-HDLC verify on `walk()` (corrupt payload → clear InvalidData not garbled downstream parse); lacks `.usdc` binary (#754), UsdSkel*, UsdGeomSubset | ✅ ~88% — symmetric writer + zero-re-encode pass-through + variant writer + composition-arc writer |
| **FBX** | 🚧 ~72% — binary container (32/64-bit) + object-graph + mesh + animation + deformers + Material/Texture/Video surfacing + bind pose + r178 LayerElementMaterial + r184 LayerElementColor + r191 Properties70 P-record grammar (typed `PropertyMap` per `fbx-binary-properties70.md` §4: Compound/scalar (int/enum/double/Number/KTime/ULongLong/KString/bool)/vec3 (ColorRGB/Vector3D/Lcl Translation/etc.); `apply_properties70` PBR mapper: DiffuseColor × DiffuseFactor → base_color, Opacity → alpha + AlphaMode::Blend, Shininess → roughness via Blinn-Phong→GGX `sqrt(2/(n+2))`, ReflectionFactor → metallic, ShadingModel → extras; +16 tests, 71+17 total). Lacks: ASCII FBX (#785), Light/Camera NodeAttribute | ✅ ~58% — symmetric binary writer + opt-in zlib deflate |
| **Alembic** | 🚧 0% — Sphinx API reference + Python examples staged at `docs/3d/alembic/`; on-disk Ogawa binary needs Wayback PDF recovery (Imageworks 2010-2012 manuals 404 today) or commissioned trace | — |

Cross-format integration: `oxideav-cli-convert` exposes a 3D conversion path through `oxideav_meta::populate_mesh3d_registry` — `oxideav convert in.obj out.gltf` (or `--probe` for structural inspection). `crates/oxideav-tests/tests/mesh3d_*.rs` runs the cross-format roundtrip suite. Convert verb has accumulated IM-compatible ops including `-resize` / `-thumbnail` / `-define` / r178 `-extent WxH±X±Y` (canvas re-window w/ source-order `-background` colour) / r184 `-monochrome` (gray + 2 colors + Floyd-Steinberg shorthand), USDZ encoder + 3D→raster renderer (Gouraud + Phong + `-light` / `-camera` / `-projection` / `-fov` / `-bg`), `-render normal-debug|depth-debug` + `-aa N` supersampling, and multi-size ICO via `-define icon:auto-resize`. Black-box oracles in `tests/mesh3d_{usdz_apple,blender_assimp}_oracle.rs` cross-validate against Apple `usdzconvert` + Blender + assimp.

</details>

<details>
<summary><strong>Trackers</strong> (decode-only by design) (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **MOD** | ✅ ~97% — 4-channel Paula-style mixer + full ProTracker 1.1B effect set + FT-extension `8xx` / `E8x` per-channel pan + XM E3x glissando control + Lxy set-envelope-position + E4x/E7x vibrato/tremolo waveform shapes (sine/saw/square) (FT2 §); PT-fidelity rounds for loop boundary / LED filter / extended period range / EE pattern-delay + 9xx out-of-range no-note quirk; 119 unit + 39 integration tests + r171 cargo-fuzz harness (MOD/STM/XM parsers) caught + fixed an xm::parse_patterns slice-index panic on hostile header_length | — |
| **STM** (Scream Tracker v1) | ✅ ~85% — structural parse + shared-mixer playback; XM-parity effects (Gxy/Jxy/Bxy/Cxy/Exy/Hxy + 7xy tremolo + volume-slide variants); hard-pan LRRL | — |
| **XM** (FastTracker 2) | ✅ ~90% — structural parse + full playback; envelopes + fadeout + key-off; vibrato + tone porta + pattern jumps + fine/extra-fine porta + Exy/Kxy subcommands + volume-column slides | — |
| **S3M** | ✅ ~95% — stereo + full ST3 v3.20 effect set + per-channel effect memory + Dxy multimedia.cx case matrix + S3x/S4x bit-2 retention + Qxy persistent-counter retrigger + Cxx row-≥64 ignore + Kxy/Lxy continue + r171 +128 channel-mute + r183 spec-correct default-pan resolution (bit-5-clear stereo → 3/C bank fallback, mono → centre 7, FireLight §2.8.1 override); lacks AdLib FM synth | — |

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
| **`oxideav-videotoolbox`** | macOS (Apple Silicon + Intel Macs) | 🚧 H.264 + HEVC + ProRes + MJPEG + MPEG-2 + VP9 + MPEG-4 Pt 2 | 🚧 H.264 + HEVC + ProRes + MJPEG | Roadmap: AV1. MPEG-2 ~61 dB; H.264 ~51 dB; HEVC ~54 dB; ProRes ~52 dB; MJPEG ~36 dB. r178 VP9 wired. r184 MPEG-4 Pt 2 decode + r190 VOL→ESDS extension-atom path (`extract_mpeg4_part_two_vol` + `build_mpeg4_part_two_esds` shape `0x03→0x04(OTI=0x20)→0x05→0x06`; `BlobDecoder` supplies via `SampleDescriptionExtensionAtoms`; integration test 72.8 dB PSNR_Y over 10/10 frames vs ffmpeg). AV1 hardware needs M3+. |
| **`oxideav-audiotoolbox`** | macOS | 🚧 AAC LC + HE-AAC v1/v2 + AAC-LD/ELD + ALAC + iLBC + AMR-NB | 🚧 AAC LC + HE-AAC v1/v2 + AAC-LD/ELD + ALAC + iLBC | AAC LC 36.7 dB @ 128 kbit/s; HE-AAC v1 ~11 dB @ 64 kbit/s; HE-AAC v2 ~10 dB @ 32 kbit/s; ALAC bit-exact 190,464/192,000; r178 AAC encoder bitrate read-back; r184 iLBC 8 kHz mono decode+encode; r190 AMR-NB decode (RFC 4867 §5.1 TOC + per-mode storage-byte tables; reserved FT 9-14 rejected; AT vends 120-sample S16 mono blocks, not 160-sample frames). Roadmap: FLAC, Opus, AMR-WB. |
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
| **`oxideav-http`** | HTTP / HTTPS source driver | ✅ `http://` + `https://` via pure-Rust `ureq` + `rustls` + `webpki-roots`; Range-request seeking; `HttpConfig` policy (timeouts/redirect cap/custom headers) + r171 RFC 7233 §4.2 Content-Range validation + §3.1 200-fallback prefix-drop + r179 §15.5.17 + §14.4 416 handling + r186 RFC 9110 §13.1.5 If-Range strong-validator (strong ETag or §8.8.2.2-promoted Last-Modified replayed on Range GETs; mid-stream representation mutation surfaces as fatal io::Error) |
| **`oxideav-generator`** | Synthetic media source (`generate://...` URIs) + zero-input filters | ✅ audio synth (sine + chirp/FM/DTMF/multitone/ADSR/ringmod + r180 5-colour noise) + image (xc/gradient/pattern/fractal/plasma/noise/label; r188 `noise?type=simplex` now a genuine Perlin-2001 2-D simplex generator, no longer a Perlin alias) + video (testsrc/smptebars/fractal_zoom/gradient_animate/zoneplate); ImageMagick/sox shorthands in `convert` verb |
| **`oxideav-rtmp`** | RTMP ingest + push | ✅ Server + client; AMF0/AMF3 parser/builder; Enhanced-RTMP v1 video + v2 audio + ModEx; pluggable key-verification; `rtmp://` PacketSource; symmetric teardown + client `poll_event` + r164 injection-robust parser surface + r179 v2 `MultichannelConfig` audio body (24 SMPTE ST 2036-2-2008 22.2 channel positions) + r187 Enhanced-RTMP v2 Multitrack body parser+builder (`AudioPacketType.Multitrack=5` / `VideoPacketType.Multitrack=6`; OneTrack / ManyTracks / ManyTracksManyCodecs; inner PacketType must not be Multitrack; 202 tests) |
| **`oxideav-sysaudio`** | Native audio output | ✅ Runtime-loaded backends (ALSA, PulseAudio, WASAPI, CoreAudio); no C build-time linkage. CoreAudio + WASAPI backends report **real HAL latency** — CoreAudio sums `kAudioDevicePropertyLatency` + `BufferFrameSize` + `SafetyOffset` + `kAudioStreamPropertyLatency`; WASAPI reads `IAudioClock`-derived presentation latency. Output-device enumeration (names + default flag) across WASAPI / ALSA / CoreAudio. r178 per-device routing API (`StreamRequest::with_device(id)` / `open_on`) — r184 CoreAudio wired via HAL `kAudioDevicePropertyDeviceUID` + `AudioQueueSetProperty(kAudioQueueProperty_CurrentDevice)`, all 4 backends now route per-device. BT-aware; falls back to software estimate if HAL unavailable. |
| **`oxideav-pipeline`** | Pipeline composition (source → transforms → sink) | ✅ JSON transcode-graph executor; pipelined multithreaded runtime + `Executor::with_channel_caps(ChannelCaps { packets, frames })` configurable per-track depth (embedded `{1,1}` → offline `{64,32}`) + `Executor::with_max_queue_bytes(n)` orthogonal byte-ceiling on demux→worker queues + r178 `Progress::elapsed_micros` wall-clock stamp on every emission (realtime ratio + live-source drift diagnostics) + r184 `packets_skipped: u64` on `Progress` + `ExecutorStats` (decoder error-tolerance visibility; staged + serial paths both increment, partial-output produced_any suppresses double-count) |
| **`oxideav-scene`** | Time-based scene / composition model | 🚧 data model for PDF pages / RTMP streaming compositor / NLE timelines + r179 per-frame `Sample` + animation-track composition helpers (`effective_transform_at` / `effective_opacity_at` / `Scene::sampled_at` paint-order walk) + r188 `RasterRenderer` (first concrete `SceneRenderer`: bg solid/gradient + Rect/Polygon shapes + `ObjectKind::Vector` → RGBA via oxideav-raster, transform/opacity/clip honoured); image/video/text/group + `Shape::Path` pending |
| **`oxideav-audio-filter`** | Audio effects & conversions (streaming) | ✅ ~46 filters: classic + transient/spatial/restoration family + MidSide / EnvelopeFollower / DeEsser / Wah / OctaveDoubler / AdaptiveNoiseGate + Exciter / MultibandCompressor / StereoImager / Talkbox + TransientDesigner / Ducker / GainNormalizer / FreqShifter + HardClipper + r106 SlewLimiter + r188 crossover LR4 slope (4th-order Linkwitz-Riley, perfect magnitude reconstruction, `|low+high|=1`) — see crate README for the catalogue |
| **`oxideav-image-filter`** | Single-frame image effects (stateless) | ✅ 127 filter types / 162 factory names — r186 Dither filter (Bayer 4×4 ordered + 7 error-diffusion kernels) from clean-room kernel transcription; r105 Scharr 3×3 (±3 ±10 ±3); r101 Prewitt + PrewittMagnitude L1/L2; r24 Roberts cross 2×2; r22 Reinhard/Hable/Drago tone-mapping + Curves (monotone-cubic) + Borgefors distance transform + Cyanotype — see crate README for the catalogue |
| **`oxideav-pixfmt`** | Pixel-format conversion + palette + dither | ✅ YUV↔RGB matrices (BT.601 / BT.709 / BT.2020 / BT.2100), chroma subsampling + r179 packed 4:2:2 (YUYV / UYVY) ↔ planar/RGB/RGBA with byte-position pins, palette quantisation (median-cut / k-means), Floyd-Steinberg dither, PQ + HLG + BT.1886 transfer functions |

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
| **ASS / SSA**       | ✅ | ✅ | Script Info + V4+/V4 Styles (BGR+inv-alpha) + override tags (b/i/u/s/c/fn/fs/pos/an/k/kf/ko/K/N/n/h). Typed positional/rotation/blur/border/karaoke + r172 `\fn`/`\fe`/`\b<weight>`/`\r[<style>]` + r177 `\pbo` + r183 `\i`/`\u`/`\s` face-flag toggles + r186 typed `\p<scale>` drawing-mode toggle (\p0 disables, \pN enables at 2^(N-1) sub-pixel scale, non-animatable per Aegisub spec) via `extract_cue_animation` → `RenderState`; `[Aegisub Project Garbage]` + `[Fonts]`/`[Graphics]` round-trip via extradata |

**Bitmap-native (own crate)** — `oxideav-sub-image`:

| Format              | Decode | Encode | Notes |
|---------------------|:------:|:------:|-------|
| **PGS / HDMV** (`.sup`) | ✅ | ✅ | Blu-ray subtitle stream; PCS/WDS/PDS/ODS + RLE + YCbCr palette → RGBA + r183 RLE codec property+negative sweep (1500 randomised roundtrips + edge cases) |
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
