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
  animate/set@t=0), `oxideav-pdf` (round 2 multi-page writer + Scene
  metadata via `/Info` dict; round 3 reader: bytes → Scene with xref +
  FlateDecode + content-stream operator parser), `oxideav-raster`
  (vector→raster rendering kernel — scanline AA, bilinear/Lanczos2,
  trapezoidal coverage, soft masks, patterns, filter primitives, ICC
  pipeline, bitmap cache keyed by `Group::cache_key`), `oxideav-ttf`
  (TrueType parser — cmap 0/4/6/12/14 incl. Variation Sequences, GSUB
  ligatures, GPOS kerning, COLR + CPAL + sbix tables, TTC subfont
  selection), `oxideav-otf` (CFF / Type 2 charstrings, cubic outlines),
  `oxideav-scribe` (shaper with vector-first `Shaper::shape_to_paths`
  API — no rasterizer dep; trapezoidal horizontal AA, GPOS mark-to-mark,
  COLR/CBDT colour glyphs via raster bilinear/composer, bidi UAX #9).
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
  `remux` / `transcode` / `run` / `validate` / `dry-run` / `convert`),
  `oxideplay` (reference SDL2 + TUI player), and `oxidetracevfw`
  (in `oxideav-tracevfw` — debugger CLI for the Windows codec
  sandbox: `probe` / `decode` / `encode` subcommands plus an
  optional `--gdb HOST:PORT` GDB Remote Serial Protocol server;
  see Windows codec sandbox section below).

(`oxideav-job` is retired — its functionality moved into
`oxideav-pipeline`. The old crate's GitHub repo is archived.)

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
| WAV       | ✅ | ✅ | ✅ | LIST/INFO metadata; byte-offset seek |
| FLAC      | ✅ | ✅ | ✅ | VORBIS_COMMENT, streaminfo, PICTURE block; SEEKTABLE-based seek |
| Ogg       | ✅ | ✅ | ✅ | Vorbis/Opus/Theora/Speex pages + comments; page-granule bisection |
| Matroska  | ✅ | ✅ | ✅ | MKV/MKA/MKS; DocType-aware probe; Cues seek; SeekHead emit; Chapters + Attachments + subtitle tracks surfaced |
| WebM      | ✅ | ✅ | ✅ | First-class: separate fourcc, codec whitelist (VP8/VP9/AV1/Vorbis/Opus); inherits Matroska Cues seek |
| MP4       | ✅ | ✅ | ✅ | mp4/ismv brands; faststart; iTunes ilst; fragmented demux + mux (DASH/HLS/CMAF) + sidx/mfra/tfra; AC-3/E-AC-3/DTS sample-entry FourCCs |
| MOV (QuickTime) | ✅ | — | ✅ | Native `oxideav-mov` crate — Apple QTFF + chan layout + faststart + udta + dref (incl. external file refs + multi-hop alias chains + cross-platform `file://`) + tkhd matrix→rotation + chapter resolution + gmhd/text/tmcd + meta-only files open without moov + ISO BMFF meta (pitm + iinf + iloc + idat + iref) + iloc construction_method 0/1/2 with cycle-detected recursive resolver + index_size>0 extent indexing + HEIF/HEIC item-properties (colr/pixi/auxC alpha-plane/clli/mdcv/cclv/amve HDR + lsel layer-selector) + derived images (grid/iovl/iden/tmap) with TransformChain cascade + post-transform `output_extent` + per-tile ispe validation + typed `base` iref + typed `ipro` ItemProtection / ProtectionScheme surface + 29-variant BrandClass + `is_heic`/`is_avif`/`is_miaf` + styl/ftab text-sample style trailers; rejects fragmented MP4 (use `oxideav-mp4`) |
| AVI       | ✅ | ✅ | ✅ | OpenDML 2.0 super-index + AVIX + dmlh + vprp + 2-field interlaced + 02ix mid-`movi` + truncated-head recovery + VBR audio (with VBR/CBR `dwSampleSize` validator) + LIST INFO emit/read (hdrl-nested + top-level + multi-value FourCC accessors) + typed `PaletteChange` + `TextChunk` round-trip (eager + lazy iterators) + typed `AvihFlags` + fluent muxer flag builders + computed `dwSuggestedBufferSize`/`dwMaxBytesPerSec` (audio-only fallback) + `avi:over_budget` metadata + opt-in `synthesise_idx1_from_ix` muxer reconstructs idx1 from ix## entries for AVI 1.0 reader compat + WAVE_FORMAT_* constants for AC3/DTS/WMA*/Opus/AAC + typed `Idx1Flags` AVIIF_* accessors + idx1↔ix## cross-validator surfacing `avi:idx1.<n>.divergent_offsets` (with `open_avi_strict` to promote to hard error) + `seek_to_first_video_keyframe_after` skip-NO_TIME helper + `with_per_stream_max_bytes_per_sec` per-stream cap + `with_strict_per_stream_budget` enforcement; idx1 + ODML keyframe seek with `KeyframeSeekResult` |
| MP3       | ✅ | ✅ | ✅ | ID3v2/v1 tags + cover art, Xing/VBRI TOC seek (+ CBR fallback), frame sync with mid-stream resync |
| IFF / 8SVX| ✅ | ✅ | — | Amiga IFF with NAME/AUTH/ANNO/CHRS |
| IVF       | ✅ | — | — | VP8 elementary stream container |
| AMV       | ✅ | — | — | Chinese MP4 player format (RIFF-like) |
| FLV       | ✅ | — | — | Flash Video — MP3/AAC/H.264 audio + VP6f/VP6a/H.264 video + AMF0 onMetaData |
| WebP      | ✅ | ✅ | — | RIFF/WEBP (lossy + lossless + animation; ANIM + ANMF emit) |
| TIFF      | ✅ | — | — | TIFF 6.0 single-image; magic II*\0 / MM\0* |
| PNG / APNG| ✅ | ✅ | — | 8 + 16-bit, all color types, APNG animation |
| GIF       | ✅ | ✅ | — | GIF87a/GIF89a, LZW, animation + NETSCAPE2.0 loop |
| JPEG      | ✅ | ✅ | — | Still-image wrapper around the MJPEG codec |
| BMP       | ✅ | ✅ | — | Windows bitmap — DIB headers BITMAPINFOHEADER / V4 / V5, 1/4/8/16/24/32-bit; also exposes the DIB helpers used by ICO / CUR sub-images |
| Netpbm    | ✅ | ✅ | — | All seven PNM magics + PAM (P1-P7); 1/8/16-bit; comment-tolerant ASCII + binary; .pbm/.pgm/.ppm/.pnm/.pam |
| ICO / CUR | ✅ | ✅ | — | Windows icon + cursor — multi-resolution, BMP and PNG sub-images |
| slin      | ✅ | ✅ | — | Asterisk raw-PCM: .sln/.slin/.sln8..192 |
| MOD / S3M / STM | ✅ | — | — | Tracker modules (decode-only by design; STM is structural-parse only) |

Cross-container remux works for any pair whose codecs don't require
rewriting (FLAC ↔ MKV, Ogg ↔ MKV, MP4 ↔ MOV, etc.).

</details>

### Codecs

> Each row below is a current-state summary. For round-by-round history, design notes, and per-feature trade-offs, see the per-crate `README.md` and `CHANGELOG.md` in `crates/oxideav-<codec>/`.

<details>
<summary><strong>Audio</strong> (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **PCM** (s8/16/24/32/f32/f64) | ✅ 100% | ✅ 100% |
| **slin** (Asterisk raw PCM) | ✅ 100% | ✅ 100% |
| **FLAC** | ✅ 100% — bit-exact vs spec | ✅ 100% — bit-exact roundtrip |
| **Vorbis** | ✅ ~95% — RFC 5215 all residue types | 🚧 ~88% — bitrate-target tunings + spread + dynalloc; ffmpeg cross-decodes |
| **Opus** | 🚧 ~88% — TOC + CELT + SILK NB/MB; libopus interop 10-26 dB | ✅ ~85% — CELT full-band + SILK NB/MB/WB + Hybrid; ffmpeg + libopus cross-decode clean |
| **MP1** | ✅ 100% | ✅ ~95% — CBR + psy-driven VBR |
| **MP2** | ✅ 100% | ✅ ~95% — CBR + VBR + intensity-stereo |
| **MP3** | ✅ ~95% — MPEG-1 Layer III M/S | 🚧 ~84% — CBR + VBR + M/S + intensity + Annex D Psy-1 |
| **AAC** | 🚧 ~84% — LC + HE-AACv1 SBR + HE-AACv2 PS + LATM + PCE; lacks LD/ELD raw_data_block, USAC frame body | 🚧 ~78% — LC + HE-AACv1/v2 + PNS + 5.1/7.1 + Bark psy default-on |
| **CELT** | ✅ ~95% | 🚧 ~88% — mono+stereo + transient short-block + spread + dynalloc |
| **Speex** | ✅ ~95% — NB/WB/UWB + RFC 5574 | ✅ ~95% |
| **GSM 06.10** | ✅ 100% | ✅ 100% — incl. WAV-49 |
| **G.711** (μ/A-law) | ✅ 100% | ✅ 100% |
| **G.722** | ✅ 100% | ✅ 100% |
| **G.723.1** | ✅ 100% | ✅ 100% — both 5.3k + 6.3k |
| **G.728** | ✅ 100% — LD-CELP 50-order | ✅ 100% |
| **G.729** | 🚧 ~70% — non-spec codebooks (audible, not bit-exact) | 🚧 ~70% |
| **IMA-ADPCM (AMV)** | ✅ 100% | ✅ 100% |
| **8SVX** | ✅ 100% | ✅ 100% |
| **iLBC** (RFC 3951) | ✅ 100% — NB 20/30 ms | ✅ 100% |
| **AC-3** (Dolby Digital) | ✅ ~96% — full decode + downmix + WAVE_FORMAT_EXTENSIBLE channel reorder for acmod 3/5/7 (corrects bitstream-slot → mask order; +78 dB PSNR jump on 3.0 + 5.1 fixtures) + audblk `cplbegf <= cplendf+2` bounds-check fix (+82 dB PSNR jump on `ac3-3-2-48000-384kbps` 5.0 fixture; corpus aggregate match-pct 1.37% → 35.29%); 90+ dB vs ffmpeg | 🚧 ~92% — acmod 1/2/3/6/7 + LFE + DBA + 5-fbw coupling + E-AC-3 indep+dep substream |
| **AC-4** (Dolby) | 🚧 ~98% — A-SPX + DRC + 60+ ETSI codebooks + 5_X/7_X ACPL_1/2/3 + cfg0/1/2/3 trailer-aware + LFE + SSF/SNF + SAP Tables 181/183 (sap_mode 0/1/3) + Pseudocode 121 companding with multi-channel cross-channel exact `g_synch` + ACPL_3 stereo-CPE M=2 synced + IMS bitstream_version≥2 walker; lacks ETSI fixture RMS audit, object/a-joc substreams | 🚧 IMS scaffold ~35% — v0 + v2 TOC round-trip through `Ac4Decoder` via `Ac4ImsEncoder` + arbitrary-PCM mono SIMPLE/ASF body (forward MDCT + KBD + scalefactor + HCB1..11 codebook-selection optimiser + DP-optimal section boundaries over SFBs + Spectral Noise Fill emission for zero-quant bands; white-noise SNR 27.5 dB) + tunable max_sfb (40/50/55 covering ~7.5/11/15 kHz); lacks stereo/multichannel forward analysis |
| **MIDI** (SMF) | ✅ ~95% — SMF Type 0/1/2 → PCM via 32-voice mixer + SF2/SFZ/DLS | — synthesis only |
| **NSF** (NES) | 🚧 ~50% — 6502 ISA + 4/5 APU channels; lacks unofficial opcodes, DMC DMA, expansion chips | — synthesis only |
| **Shorten** (.shn) | ✅ ~95% — all 10 FN cmds + filetypes 1-11 + format-v1 + running-mean estimator + 64-bit bit-reservoir reader using hardware `lzcnt`/`clz` + fused SoA stereo decode (predictor recurrence writes scratch, single strided commit applies bshift; 2.13× mono / 1.16× stereo throughput) | ✅ ~85% — production encoder + Levinson–Durbin LPC + BITSHIFT lossy + lossy `-n N`/`-r N` bit-budget modes |
| **TTA** (True Audio) | ✅ ~95% — TTA1 fmt=1/2 + password + trace tape | — |
| **aptX** (classic + HD) | 🚧 ~70% — 4-band QMF + ADPCM; bit-exact NDA-blocked | — |

</details>

<details>
<summary><strong>Video</strong> (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **MJPEG** | ✅ ~95% — baseline + progressive 4:2:0/4:2:2/4:4:4/grey + SOF9 arithmetic | ✅ ~90% — baseline + progressive |
| **FFV1** | ✅ 100% — bit-exact ffmpeg | ✅ 100% — bit-exact ffmpeg |
| **MPEG-1 video** | ✅ ~95% — I+P+B | ✅ ~95% — I+P+B + half-pel diamond ME + activity-based per-MB QP + B-frame QP offset |
| **MPEG-2 video** | ✅ ~95% — I+P+B + alternate_scan + field/interlaced + 4:2:2/4:4:4 + dual-prime | ✅ ~80% — I+P GOPs + 4:2:2/4:4:4 chroma + field-DCT interlaced |
| **MPEG-4 Part 2** | ✅ ~85% — I+P+B-VOP + 4MV + ¼-pel + field-MV/DCT + GMC + DP + RVLC | 🚧 ~88% — I+P+B + 4MV + ¼-pel + multi-warp GMC + static-sprite + DP + RVLC + MPEG-quant |
| **Theora** | ✅ ~95% — I+P; 1080p + 4 corpus fixtures bit-exact | ✅ ~95% — I+P + INTER_MV_FOUR + scene-change keyframe + two-pass complexity-driven QI |
| **H.263** | 🚧 ~80% — I+P + half-pel + Annexes D/E/F/G/I/J/K/M/N | 🚧 ~65% — I+P + diamond ME + Annexes F/J/D/N/G/I/M/K |
| **H.261** | ✅ ~95% — I+P QCIF/CIF + integer-pel + loop filter | ✅ ~95% — spiral+diamond ME + GQUANT-from-bitrate; 45 dB at 64 kbit/s QCIF |
| **MS-MPEG-4** (v1/v2/v3) | 🚧 ~30% — clean-room scaffold; v3 intra 3-tier ESC + custom intra-DC VLC; lacks G0..G3 packed-Huffman, alt-MV VLC (#303). VfW-sandboxed mpg4c32.dll runs in parallel — see Windows codec sandbox below | — |
| **H.264** | 🚧 ~80% — I/P/B + 4:2:0/4:2:2/4:4:4 + CAVLC + CABAC + DPB + B-pyramid POC + 8 SEI types; lacks MBAFF, SVC/3D/MVC | 🚧 ~80% — I+P (1MV/4MV, ¼-pel) + B 16x16/16x8/8x16/B_8x8 + per-cell mixed B_8x8 + B_Skip/B_Direct + weighted pred + CABAC at all chroma layouts; ffmpeg PSNR_Y 44.20 dB |
| **H.265 (HEVC)** | 🚧 ~72% — I/P/B 8-bit + Main 10/12 + 4:2:0/4:2:2/4:4:4 + SAO + deblock; HEIF/HEIC corpus 14/14; textured 4:2:2 P-slice CABAC drift pending docs trace (#444) | 🚧 ~75% — I+P + B (mini-GOP > 1 at 8/10/12-bit + 8-bit 4:4:4) + AMP + HBD + 4:4:4 P/B writers; lacks SAO/deblock RDO, HBD 4:4:4 AMP/merge/B_Skip |
| **H.266 (VVC)** | 🚧 ~50% — 4:2:0 IDR intra + ALF/SAO/CC-ALF + P/B merge+skip + HMVP + MMVD + CIIP + BCW + BDOF + GPM + AMVR + HBD; lacks DMVR/PROF, affine, full mvd_coding | 🚧 ~66% — forward CABAC + DCT-II + per-CTU SAO RDO (luma + chroma; Cb +2.04 dB at QP 26) + APS-signalled per-class luma filter-set learning (25 rows + Wiener + lattice quant + per-CTU 17-trial RDO + APS-vs-fixed picture-bits trade-off + chroma + CC-ALF APS) + chroma residual emit (PSNR ≥30 dB at QP 26) + explicit per-TU CBF CABAC + spec-shaped coding_tree_unit / coding_tree / coding_quadtree / coding_unit shells + cu_qp_delta + opt-in alf_luma_clip_idx[] joint coeff/clip RDO (≈ +13.5 dB PSNR_Y on stripes) + opt-in per-CTB chroma SAO merge with sao_merge_left/up_flag CABAC emit per §7.3.10.3 (both flags wired into encode_picture) + 128×128 forced QT-split recursion to four 64×64 sub-CUs + MTT BT/TT split syntax + ctxIdx tables + opt-in MTT BT picker RDO (BT_VERT/BT_HORZ vs leaf SSE+λ·bits comparison; lossless on horizontal-edge fixture) + multi-row neighbour tracking via `CuNeighbourMap` (4-sample granularity; ctx_inc_split_cu_flag now reflects real neighbour split state); lacks TT picker, inter-frame P-slice pipeline |
| **VP6** | ✅ ~95% — full FLV playback (845/845 sample frames) | 🚧 ~88% — keyframe + inter + iterative diamond qpel ME + INTER_FOURMV + Huffman + bool/Huffman RDO + PID rate ctrl + Trellis quant |
| **VP8** | ✅ 100% — entire 15-fixture corpus bit-exact | 🚧 ~97% — I+P + B_PRED + SPLIT_MV + alt-ref/golden + Lagrangian RDO + libvpx-shape Trellis + activity AQ + RFC 6386 §15.2 mode/ref deltas; ~15 opt-in advanced flags off-by-default (adaptive LF cap ladder + variance-driven cap + UV-channel deltas + per-MB / spatial 4-means / k-means++ segment_lf_deltas + chroma-aware spatial via mb_sse_uv_cache + chroma-aware per-MB median + joint r44+r49 two-pass picker + 4×4 B_PRED RDO + UV-mode RDO + joint LF-RDO + SPLIT_MV partition RDO with first-pass real-context + MV-cost-aware NEAREST/NEAR/NEW + sub-pel partition refinement + Trellis context-rate + psy-RDO/ARNR) + k-means convergence early-exit with iter telemetry via `Vp8EncoderStats` |
| **VP9** | 🚧 ~85% — keyframe + inter + segmentation + COMPOUND_PRED + INTERINTRA + per-frame CDF; chroma bit-exact | 🚧 ~35% — keyframe + simple P-frame + per-block luma intra-mode RDO; smooth gradient 53.06 dB at base_q_idx=64 |
| **AV1** | 🚧 ~72% — OBU + range coder + all intra preds + CDEF + LR + inter MC + palette + multi-ref compound + super-res; SVT-AV1 48/48; lacks intrabc | 🚧 ~55% — forward range coder + forward DCT-II 8/16/32 + full coefficient emitter + partition/mode/TX emit; self-roundtrip bit-exact via own decoder, dav1d still rejects 64×64 |
| **Dirac / VC-2** | ✅ ~90% — VC-2 LD + HQ intra + Dirac core-syntax intra/inter + OBMC + 7 wavelets + 10/12-bit; ffmpeg bit-exact at multiple chroma | 🚧 ~91% — HQ + LD intra + Dirac core-syntax + 2-ref bipred B-picture with adaptive sub-pel-vs-int-pel selection; camera-pan bipred 52.53 dB |
| **AMV video** | ✅ 100% | ✅ 100% — via MJPEG encoder |
| **ProRes** | ✅ ~95% — RDD 36 entropy + 8/10/12-bit + 4:4:4:4 alpha + interlaced; ffmpeg interop 60-68 dB | ✅ ~90% — emits valid RDD 36 across all 6 profiles + interlaced + alpha + perceptual quant matrices |
| **EVC** (MPEG-5) | 🚧 ~70% — NAL + SPS/PPS/APS + §9.3 CABAC + §8 intra (Baseline) + DCT-II + P/B inter + RPL + HMVP + DPB + ALF + DRA; lacks IBC | — |
| **HuffYUV** / FFVHuff | ✅ ~92% — HFYU + FFVH FourCCs + 6 predictors + 8/10/12-bit FFVHuff + interlaced field-stride=2 + fast-LUT decoder | ✅ ~90% — full encoder symmetry × YUY2/RGB24/RGB32 + v1.x + v2.x ClassicV2/CustomV2 + walking-stride interlaced (~30% memory reduction) |
| **Lagarith** | ✅ ~95% — all 11 wire types (1-11 + NULL replay) + modern range coder with spec/02 §5 three-way fast path (Step A symbol-0 dominant + Step B slack-band sentinel + Step C cumulative search; 4.31× decode throughput on signal-heavy fixtures, 161 MSym/s) + legacy adaptive-CDF + Fibonacci-Zeckendorf prefix + JPEG-LS Median + G-pivot decorr + zero-run RLE; lacks pair-packed 513-entry CDF | 🚧 ~70% — encoder for SOLID/RGB/RGBA/YV12/YUY2/legacy-RGB; byte-exact vs proprietary encoder Auditor-blocked |
| **Ut Video** | ✅ ~95% — 5 native FourCCs (ULRG/ULRA/ULY0/ULY2/ULY4) × 4 predictors + RGB inter-plane decorrelation + canonical Huffman + 3000-cell pattern matrix tested | ✅ ~95% — codec-internal encoder mirrors decoder for self-roundtrip |
| **MagicYUV** | ✅ 100% — 17 v7 FOURCCs (8 + 10/12/14-bit M0/M2/M4) + Median + JPEG-LS Median (HBD) + raw-mode + interlaced + AVI 1.0/OpenDML 2.0; trace JSONL strict-jq-line-diff-equal to cleanroom Python ref; decode/encode 1.6-1.9× faster than pre-optimisation | ✅ 100% — `encode_frame` / `encode_avi` / `encode_avi_opendml` across all 17 FOURCCs |
| **Cinepak** (CVID) | ✅ ~95% — frame header + multi-strip + V1/V4 codebooks + intra + inter with skip + full selective-update family + grayscale + Sega FILM demuxer | ✅ ~88% — stateful `CinepakEncoder` with rolling codebooks (selective-update / full-replace / chunk-omit; 91.6% wire saved on static-fixture) + multi-strip + median-cut + skip-MB + two-pass rate ctrl + windowed bisection (variance-coupled adaptive tolerance) + tighter Lloyd refinement + empty-cluster slot reclamation (stale slot threshold + forced-full-replace recovery) + `last_frame_stats()` telemetry; ffmpeg AVI roundtrip 36.9 dB |
| **SVQ1** (Sorenson) | 🚧 ~30% — frame-header + I/P + multistage QT walker; flat-fill output — blocked on docs (§14.10/§14.11 codebook bytes #429) | — |
| **Indeo 2** (RT21) | 🚧 ~15% — frame-header + structural pipeline; mid-grey placeholder — blocked on docs | — |
| **Indeo 3/4/5** | — — see Windows codec sandbox below (sandboxed via `oxideav-vfw`) | — |

</details>

<details>
<summary><strong>Image</strong> (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **PNG / APNG** | ✅ 100% — 5 colour types × 8/16-bit + APNG | ✅ 100% |
| **GIF** | ✅ 100% — 87a/89a + LZW + interlaced + animation | ✅ 100% — per-frame palettes |
| **WebP VP8L** | ✅ 100% — 7/7 BitExact vs `dwebp` | ✅ ~99% — full lossless RDO + LZ77 + meta-Huffman + near-lossless + palette; landscape-256 1.0124× cwebp |
| **WebP VP8** | ✅ 100% — via VP8 + bit-exact YUV→RGB + fancy chroma upsample + streaming `WebpAnimDecoder` (lazy demux, one OWEB payload per `Demuxer::next_packet`) | 🚧 ~97% — VP8 I-frame + ALPH + per-segment QP/LF + Trellis + animated ANIM/ANMF with file-level ICCP/EXIF/XMP + AVIF-style perceptual frame-merge encoder (`AnimFrameMode::Delta`: SAD default + opt-in SSIM-lite + opt-in 3-scale MS-SSIM with Box/Gaussian pyramid + multi-rect ANMF via flood-fill + density-band-adaptive component budget + per-sub-rect lossy/lossless race; ~75% reduction on 5% changing-region fixture, ~67% on 320×240 3-cluster slow-tests) + standalone + registry-side metadata parity; dwebp cross-decode clean |
| **JPEG** (still) | ✅ ~95% — via MJPEG | ✅ ~90% — via MJPEG |
| **TIFF** (6.0) | ✅ ~90% — II/MM + BigTIFF read + 6 photometrics + 1/4/8/16-bit + None/PackBits/LZW/Deflate + tiles + multi-page; bit-exact tiffcp; lacks CCITT G3/G4, JPEG-in-TIFF, BigTIFF write | ✅ Gray8/16/RGB24/Palette8 — None/PackBits/LZW/Deflate, single+multi-page |
| **BMP** | ✅ ~95% — 1/4/8/16/24/32-bit + V4/V5 + RLE4/RLE8 | ✅ ~95% |
| **Netpbm** (PBM/PGM/PPM/PNM/PAM) | ✅ ~95% — all 8 magics at 1/8/16-bit + 6 PAM TUPLTYPEs | ✅ ~95% |
| **ICO / CUR** | ✅ ~95% — multi-res + BMP/PNG sub-images + CUR hotspot | ✅ ~90% |
| **JPEG 2000** | ✅ ~88% — Part-1 baseline + multi-tile + MQ + EBCOT + 5/3 + 9/7 + JP2 + 5 progression orders + POC + HTJ2K (Part 15) cleanup/SigProp/MagRef | ✅ ~88% — 5/3 + 9/7 + 5 progression orders + POC + PPM/PPT + HTJ2K Part-15 SigProp/MagRef encoder; ojph_expand cross-decodes bit-exactly |
| **JPEG XL** | 🚧 ~85% — ISO/IEC 18181-1:2024 final core. 5 small lossless fixtures decode PIXEL-CORRECT. Modular path complete; VarDCT scaffold wired into pipeline (Quantizer + LfCoefficients + F.1 LF dequant + F.2 adaptive smoothing + HfMetadata + HfGlobal + GlobalModular). Auditor d1-Squeeze bisect through r17..r22 has falsified per-token hybrid-uint accounting, cluster_map uniformity, state init, prelude bits, "267-bit overshoot", per-cluster distributions, alias self-map, and WP rounding bias — divergence localised to Y' sample 22 / one MA-tree leaf-flip; ANS final state never hits §D.3.3 sentinel. r24 then byte-traced cluster 0+1 D[] arrays (sum=4096, alias-routed total=4096; 0 invariant violations across 3072 calls). r25 extended leaf-pick dumper across log_idx 22..=79: first true ctx-flip at log_idx 79 fully explained as expected downstream rounding cascade through WP true_err history (NOT a new bug). Bug now confirmed NOT in the LfCoefficients sub-stream itself — must be in upstream prelude D[] vs encoder intent, or in TOC-entry boundaries / channel-order. Diagnostics in `round17..25-d1-*.md`; Annex L XYB inverse + YCbCr inverse colour transforms wired + Annex I.2 IDCT dispatch primitive (18 plain-DCT TransformTypes, power-of-2 sizes 1..=256) + Annex I.9.3..I.9.7 non-DCT IDCT helpers (Hornuss / DCT2x2 / DCT4x4 / DCT4x8 / DCT8x4 / AFV0..3); lacks HfPass + PassGroup HF + GetDCTQuantWeights + CfL / Gaborish / EPF | — retired; will re-author after decoder forward progress |
| **JPEG XS** | 🚧 ~70% — ISO/IEC 21122 Part-1 + inverse 5/3 DWT + Annex C/D/F/G entropy + multi-component (4:2:2/4:2:0) + CAP-bit | 🚧 ~58% — Nc 1/3 + RCT + NL up to 5 + odd dimensions + Star-Tetrix + vertical prediction; bytes -26% vs raw at q=8; lacks significance coding, NLT, per-band Q |
| **AVIF** | 🚧 ~75% — HEIF→AV1 + grid + imir/clap/colr/pixi/pasp + HDR metadata (clli/mdcv/cclv) + AV1 wrap pass-through; gated on AV1 decoder completeness | — |
| **DDS** | ✅ ~98% — DDS_HEADER + DXT10 + uncompressed (10 layouts) + BC1-5/7 + BC6H all 14 modes + mipmap + 6-face cubemaps + DX10 arrays + full 132-entry DXGI table | ✅ ~92% — uncompressed + BC1-5 + BC7 all 8 modes (0-7 incl. mode 4/5 channel-rotation; rank-3 multi-axis 30.4 dB; independent-alpha ≥30 dB-RGBA) + BC6H_UF16 all 14 modes + BC6H_SF16 mode 10 (signed-magnitude pipeline; signed gradient ≥19 dB) + box-downsample-then-encode mip chains + cubemap/array; lacks BC6H_SF16 modes 11/12/13 + 2-subset signed |
| **OpenEXR** | 🚧 ~65% — magic + 8 required attrs + HALF/FLOAT/UINT + NO_COMPRESSION/ZIP/ZIPS/RLE + tiled ONE_LEVEL + sub-sampled chroma; exrmetrics cross-validates; PIZ blocked on docs trace; lacks B44/B44A/DWAA-B, multi-part, deep | ✅ ~75% — RGBA scanline + ZIP/ZIPS/RLE + tiled-output ONE_LEVEL + multi-part scanline; exrmetrics + exrmultipart cross-validate bit-exact |
| **Farbfeld** | ✅ 100% | ✅ 100% |
| **HDR** (Radiance RGBE) | ✅ ~95% — new-RLE + old-RLE + 8 axis-flag combos + shared-exponent | ✅ ~96% — new-RLE + old-RLE + XYZE↔RGB + tone-mapping (Reinhard/ACES) |
| **QOI** | ✅ 100% — byte-exact vs all 8 reference fixtures | ✅ 100% — byte-exact vs reference encoder |
| **TGA** | ✅ ~98% — types 1/2/3/9/10/11 + TGA 2.0 extension + thumbnail; magick cross-validated | ✅ 100% — all six image types + TGA 2.0 extension + thumbnail |
| **ICER** (JPL) | 🚧 ~75% — Mars-rover heritage; bit-plane scan + compressed/uncompressed segments + 8 filters + IPN 42-155 §III.B context model | ✅ ~75% — quota-controlled encoding (`with_byte_budget` / `with_target_bytes`) — MSB-down progressive truncation |
| **WBMP** | ✅ 100% — Type 0 | ✅ 100% |
| **PCX** (ZSoft) | ✅ ~95% — 1/2/4/8 bpp planar + packed-bits + 24 bpp RGB planar + DCX multi-page; magick cross-validated | ✅ ~95% — 6 write paths + DCX |
| **ILBM** (Amiga IFF) | ✅ ~85% — BMHD/CMAP/CAMG/BODY + ByteRun1 RLE + EHB + HAM6/HAM8; lacks PBM, ANIM, SHAM/PCHG | ✅ ~75% — `IlbmMuxer` parity across IndexedAuto/Ham6/Ham8/Ehb/Pbm + masking; magick cross-decode bit-exact for indexed + PBM |
| **PICT** (Apple QuickDraw) | ✅ ~92% — v1 + v2 opcode walkers + drawing-command rasteriser + DirectBitsRect packType 1/2/3/4 + Region + clip-region honouring + pen-size aware draws + Compressed/UncompressedQuickTime opcode skip; lacks pattern fills, text rasterisation, embedded JPEG decode | ✅ ~90% — `PictBuilder` + every v2 drawing-command family + state opcodes + DirectBitsRect packType 1/2/3/4 + BitsRgn / PackBitsRgn encoders; magick cross-decode bit-exact |
| **SVG** | ✅ ~98% — full shape set + path + gradients + text/tspan + mask + clipPath + use/symbol viewport mapping + svgz + SMIL animate/set/animateTransform at arbitrary `t` (paced + spline calcMode + parent-id-tracked re-attachment) + CSS3 Selectors L3 cascade with pseudo-classes + `@import` resolve_imports (cycle detection + depth cap 8) + `@font-face` + `@keyframes` capture + runtime evaluation at `t_seconds` (lerps transform/opacity/colour; full timing-function set linear/ease*/cubic-bezier-bisection/steps + multi-name + direction + fill-mode per L1) + `@supports` parse + evaluation + Media Queries L4 `@media` parse + evaluation + viewBox + non-uniform preserveAspectRatio + `<image>` data-URI + external href + `<script>` graceful capture + 17 typed filter primitives + CSS Values L4 `LengthUnit` (em/rem/%/vw/vh/vmin/vmax/pt/cm/mm/in/pc/q) with `Length::resolve(ctx)` threaded through element.rs/decoder.rs parse paths (per-element font-size cascade for em-resolution) + CSS Easing L2 `linear()` multi-stop function | ✅ ~86% — round-trips full shape graph + PreservedExtras side-channel for `<style>`/`<filter>`/`<animate>`/`<foreignObject>`/`<script>`/`<image>` |
| **PDF** | ✅ ~94% — bytes → Scene via xref + xref-streams + ObjStm + content-stream parser + `/Prev` incremental updates + `/Encrypt` all revisions (R=2/3/4 RC4 + AES-128, R=5/6 AES-256) + per-stream `/Crypt /Identity` + public-key handlers `adbe.pkcs7.s3/s4/s5` incl. KARI ECDH (P-256/384/521/X25519/X448 with X9.63 + RFC 8418 HKDF KDFs + AES-KW 128/192/256) + `TrustStore` for long-term-cert KARI originators (with temporal validity lookup via RecipientKeyIdentifier `date`/`other`) + typed `OriginatorInfo { certs[], crls[] }` accessor + read-only RC2 / 3DES envelopes for PDF 2.0 legacy + document-level XMP `/Metadata` stream reader + PKCS#7 SignedData parser + verify_signature dispatch (SHA-1/256/384/512 × RSA-PKCS1v15 / RSA-PSS / ECDSA-P256/384/521 with IAS/SKI cert resolution + signed_attrs SET re-tagging per RFC 5652 §5.4 + messageDigest cross-check per §11.2) + `/Sig` AcroForm annotation reader (ISO 32000-1 §12.7.4.5 + §12.8.1) with end-to-end RSA-PKCS1v15+SHA-256 detached-signature verify against `/ByteRange`-spanned bytes + text extraction (`Tj`/`TJ`/`'`/`"` + Tm/Tlm + q/Q + ToUnicode CMap `bfchar`/`bfrange` + Identity-H Type0/CIDFontType0/2 + WinAnsiEncoding/MacRomanEncoding simple fonts; pdftotext cross-check) + JPEG passthrough on `/DCTDecode` Image XObjects (DeviceRGB/CMYK/Gray/Indexed; ASCII85/ASCIIHex/Flate wrapper unwind; ObjectId dedup; pdfimages cross-check); lacks /Sig writer, glyph `/Differences` resolver, reading-order layout pass | ✅ ~96% — PDF 1.4/1.5 multi-page + paths + gradients + strokes + opacity + clip + RGBA images + xref-stream + ObjStm + incremental updates + Linearization (Annex F lin-dict + F.4.1 hint table) + ObjStm+encryption combined path + `/Encrypt` ENCODE all revisions + public-key ENCODE (pkcs7_s4 / pkcs7_s5_v4_aes128 / pkcs7_s5_v5_aes256 + multi-CF + symmetric KARI writer across all 4 curves + 3 HKDF binds) |

</details>

<details>
<summary><strong>3D scenes & assets</strong> (click to expand)</summary>

> The typed Scene3D / Mesh / Material PBR / Skin / Animation / Camera / Light / AudioEmitter model lives in `oxideav-mesh3d`, with `Mesh3DDecoder` / `Mesh3DEncoder` traits and a `Mesh3DRegistry` that's parallel to `oxideav-core::CodecRegistry`. Per-format crates register into it. `oxideav-meta::populate_mesh3d_registry(&mut Mesh3DRegistry)` walks every enabled format's `register()`. Lazy bytes flow through `AssetSource` (with a `raw_storage` pass-through hook for archive-backed sources, e.g. ZIP-stored USDZ textures + audio).

| Format | Decode | Encode |
|--------|--------|--------|
| **STL** (ASCII + binary) | ✅ ~98% — both formats + per-face attribute round-trip + 16-bit colour extension (VisCAM + Materialise conventions) + Materialise binary header `COLOR=`/`MATERIAL=` + multi-`solid` ASCII + JSONL trace + `share_stats` event + fuzz-resistant header detection (BOM / leading-whitespace / `solid by Microsoft` 80-byte traps) | ✅ ~98% — both formats + per-face attribute pass-through + auto `stl:unique_vertex_count` extras + `EncodeStats` (bit-exact + tolerance + uniform-grid spatial-index dedup) + configurable float precision |
| **OBJ** (+ MTL) | ✅ ~95% — full Wavefront grammar (v/vt/vn/f/g/o/s/mtllib/usemtl) + MTL (Phong + Wavefront-PBR Pr/Pm/Pc/Ps + map_Pr/map_Pm + Tf spectral/xyz + sharpness + `d -halo` + map_* option flags `-blendu`/`-bm`/`-clamp`/`-imfchan`/`-o`/`-s`/`-t`/`-texres` + typed `refl -type cube_*/sphere`) + smoothing groups + multi-name `g` + display attrs (`bevel`/`c_interp`/`d_interp`/`lod`) + `p` point + `mg` merging-group + LineStrip/Loop promotion + free-form geometry (`vp`/`cstype`/`deg`/`curv`/`surf`/`parm`/`trim`/`hole`/`scrv`/`sp`/`end`/`bzp`/`bsp` via Scene3D::extras); lacks NURBS/Bezier tessellation evaluator | ✅ ~95% — symmetric + negative-index encoder + polyline rejoin + `with_mtl_basename` |
| **glTF 2.0** (+ .glb) | ✅ ~90% — JSON + binary container (magic `glTF`) + full PBR (base_color, metallic, roughness, normal+scale, occlusion+strength, emissive, alphaMode Mask{cutoff}, doubleSided) + KHR_lights_punctual (Directional / Point{range} / Spot{inner,outer}) + BufferViewAsset (BIN chunk → AssetSource) + skin + skeletal animation (LINEAR / STEP / CUBICSPLINE × translation/rotation/scale/weights) + sparse accessors + multi-scene + morph-targets + accessor min/max bounds validation + normalised-int animation outputs (BYTE/UBYTE/SHORT/USHORT); lacks KHR_audio_emitter / KHR_materials_* / KHR_texture_transform (blocked on docs/3d/gltf/extensions/ mirror, task #714) | ✅ ~90% — symmetric + sparse-encoding heuristic for animation outputs / MAT4 IBM / mesh attributes (POSITION/NORMAL/TANGENT/COLOR/WEIGHTS) + signed + unsigned normalised-int quantisation |
| **USDZ** (+ USDA) | ✅ ~85% — own ZIP STORED walker (rejects DEFLATE entries; validates 64-byte alignment) + USDA tokenizer + prim-tree parser + UsdGeomMesh → Mesh+Primitive + UsdPreviewSurface → PBR + UsdUVTexture (`ZipStoredAsset` exposes `raw_storage("zip-stored")` for pass-through) + upAxis + metersPerUnit + per-mesh `xformOp:translate/orient/scale/transform` + multi-primitive sibling-Mesh emit-and-fold + `usd:no_fold` flag + UsdMediaSpatialAudio reader (`auralMode`/`gain`/`startTime`); lacks `.usdc` binary backend, UsdSkel*, UsdGeomSubset, composition arcs | ✅ ~85% — symmetric ZIP STORED writer with 64-byte alignment + USDA writer + `EncodeReport.pass_through_textures`/`pass_through_audio` confirm zero-re-encode USDZ→USDZ for textures + audio when source `raw_storage("zip-stored")` matches archive scheme + strips/fans tessellation + `UsdGeomBasisCurves`/`Points` for line/point topologies |
| **FBX** | 🚧 0% — binary container reverse-engineered (Blender Foundation 2013 writeup mirrored at `docs/3d/fbx/`); object-graph semantics (Geometry/Model/Material/Texture/AnimationStack) need commissioned clean-room trace | — |
| **Alembic** | 🚧 0% — Sphinx API reference + Python examples staged at `docs/3d/alembic/`; on-disk Ogawa binary needs Wayback PDF recovery (Imageworks 2010-2012 manuals 404 today) or commissioned trace | — |

Cross-format integration: `oxideav-cli-convert` exposes a 3D conversion path through `oxideav_meta::populate_mesh3d_registry` — `oxideav convert in.obj out.gltf` (or `--probe` for structural inspection). `crates/oxideav-tests/tests/mesh3d_*.rs` runs the cross-format roundtrip suite (79 tests across stl ↔ obj ↔ gltf ↔ usdz pairs + extras side-channel preservation audit + multi-material pool stress + Mesh3DRegistry lookup contract).

</details>

<details>
<summary><strong>Trackers</strong> (decode-only by design) (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **MOD** | ✅ ~95% — 4-channel Paula-style mixer + full ProTracker 1.1B effect set; PT-fidelity rounds for loop boundary / LED filter / extended period range / EE pattern-delay; 89 unit + 39 integration tests | — |
| **STM** (Scream Tracker v1) | ✅ ~85% — structural parse + shared-mixer playback; XM-parity effects (Gxy/Jxy/Bxy/Cxy/Exy/Hxy + volume-slide variants); hard-pan LRRL | — |
| **XM** (FastTracker 2) | ✅ ~90% — structural parse + full playback; envelopes + fadeout + key-off; vibrato + tone porta + pattern jumps + fine/extra-fine porta + Exy/Kxy subcommands + volume-column slides | — |
| **S3M** | ✅ ~80% — stereo + SCx/SDx/SBx effects | — |

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
Lives in `oxideav-vfw`; design contract in
[`docs/winmf/winmf-emulator.md`](https://github.com/OxideAV/docs/blob/master/winmf/winmf-emulator.md).

| Codec | Binary | Test fixture | `ICDecompress` | Notes |
|-------|--------|--------------|----------------|-------|
| Indeo 3 (IV31) | `IR32_32.DLL` | `cubes.mov` 160×120 | ✅ ICERR_OK | Integer ISA only |
| Indeo 5 (IV50) | `IR50_32.DLL` | `cat_attack.avi` 320×240 + 3 more | ✅ ICERR_OK 8/8 frames | MMX kernels active (1.5M-5M dispatches/frame post-r20 FloatingPointProcessor registry probe + EFLAGS.ID / RDTSC / Pentium II CPUID fixes) |
| Indeo 4 (IV41) | `IR41_32.AX` | `crashtest.avi` 240×180 + `indeo41.avi` 320×240 | ✅ ICERR_OK 8/8 frames each | MMX kernels active |
| MSMPEG4 v3 (DIV3) | `mpg4c32.dll` | wmpcdcs8-2001 reference binary | ✅ ICERR_OK 17/17 frames across 5 multi-frame fixtures (gop-30 / with-skip-mbs / motion-pan / intra-pred-active / qscale-high) at 352×288; 42.9 dB PSNR-RGB vs ffmpeg | Required: 13 stubs + `Registry::register_data` (`_adjust_fdiv`) + x87 ISA (FLD/FST/FILD/FIST/FADD…/FXCH/FCHS/FNSTSW/FLDCW + FSIN/FCOS/FPREM/FSCALE) + lowercase FOURCC + DirectShow GUID handshake (`b4c66e30-…` at `[esi+0xb4..0xc8]`, gated on MP43/MP42/MPG4) + `ICINFO_SIZE = 568` strict-codec gate. New paths exercised: skip-MB (38% SKIP fraction), alternate-MV-VLC P-frames, AC-prediction, qscale=16. 12 dB matrix delta is intrinsic — codec rejects every non-BI_RGB output 4CC via `ICDecompressQuery`. |
| MSMPEG4 v3 DShow | `mpg4ds32.ax` | winxp | ✅ **First end-to-end MP43 keyframe decode through DirectShow** at r41 — codec emits a 176×144 24bpp BGR sample (76032 nonzero bytes) which `surface_received_dshow_frame` flips to `Frame::Video`. Path landed r25-r41: IBaseFilter Run/Pause/GetState + JoinFilterGraph + IPin::ReceiveConnection (synth AMT fallback) + HostIMemAllocator committed-state machine + IMemInputPin::GetAllocator + IID_IMediaSample2 host stub (r39) + esp-discipline forensic (r40) + `IMemAllocator::GetBuffer` arg_dwords 4→5 fix (r41 — missing `dwFlags` arg left stack 4 bytes off, causing `pop ebx` in Transform's epilogue to read filter_base instead of pInSample, mis-resolving primary-vtable slot 13 → IsEqualGUID(NULL+0x1c) trap at RVA 0x7184). r42 path: multi-frame robustness + preventive `arg_dwords` sweep across `IPin::Connect/ReceiveConnection`, `IBaseFilter::JoinFilterGraph` | DirectShow IBaseFilter wrapper: COM scaffolding + ole32 stubs + DllGetClassObject + full HostIFilterGraph + HostIPin (output+input) + HostIEnumMediaTypes + HostIMemAllocator (committed state-machine) + HostIMediaSample + EnumMediaTypes walker + IMediaFilter Pause/Run/GetState. user32 cascade: synthetic-HWND no-op family. CLSID `{82CCD3E0-F71A-11D0-9FE5-00609778EA66}`. |
| WMV1/2 DShow | `wmvds32.ax` | winxp | CLASS_E_CLASSNOTAVAILABLE on default CLSID | Needs the shipped `wmvax.inf` filter CLSID; round-26+ |

**Architecture** — the emulator is a 4 GiB MMU + i386 integer ISA
+ MMX ISA (~50 opcodes) + x87 FPU (8-deep stack) + PE32 loader +
Win32 stub surface (kernel32 + user32 + msvcrt + winmm + advapi32 + ole32
+ vfw32) + **a COM dispatch layer** (`Guid` parser + `ComObjectTable`
ref-count bookkeeping + vtable-slot dispatch + class-factory cache
covering IUnknown / IClassFactory / IBaseFilter / IPin / IMemAllocator
/ IMediaSample / IFilterGraph) for codecs that ship as DirectShow
filters rather than VfW drivers (`.ax` exposing `DllGetClassObject`
instead of `DriverProc`). Whole crate is `#![forbid(unsafe_code)]` — codec DLL
never runs on the host CPU, and the only `unsafe` boundary other
emulators have (mmap'd executable pages, JIT, longjmp) doesn't
exist here. **Provenance is not clean-room** — Microsoft's API
surface is public by design and explicitly licensable for
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

**Trace mode** — disabled by default behind a `trace` Cargo
feature (zero hot-path cost when off). When on, every memory
read/write to a watched range, every Win32 call (with arguments +
return value), and optionally every executed instruction emit
JSONL events. Schema documented in
`docs/winmf/winmf-emulator.md`. The reverse-engineering output is
the input format the project's
specifier→extractor→implementer round procedure consumes when
producing clean-room codec specs from scratch.

### `oxidetracevfw` — interactive debugger CLI

The `oxideav-tracevfw` crate ships an `oxidetracevfw` binary that
drives the trace surface programmatically. Three subcommands:

```
oxidetracevfw probe   <DLL>         # enumerate DllMain + exports
oxidetracevfw decode  <DLL> <BMP>   # ICOpen → ICDecompressQuery → ICDecompress
oxidetracevfw encode  <DLL> <RAW>   # ICOpen → ICCompress (waiting on Sandbox::ic_compress_*)
```

Plus four global flags surfaced on every subcommand:
`--asm` (per-instruction trace), `--trace-mem ADDR:SIZE[:MODE]`
(memwatch — MODE ∈ `r|w|rw`), `--break PC` (PC breakpoint that
emits a `kind=breakpoint` JSONL event when hit), and
`--trace-output FILE` (JSONL sink).

`--gdb HOST:PORT` ships a full **GDB Remote Serial Protocol**
server (via `gdbstub`) — read/write GPRs + EIP + EFLAGS + MMX
(MM0..MM7 mapped onto X86_SSE st[i].low64 per Intel SDM §9.2.1)
+ memory through the MMU; software breakpoints (`Z0`/`z0`);
hardware watchpoints (`Z2`/`Z3`/`Z4` → `Sandbox::watch`/`unwatch`)
that yield `T05watch:addr;` stop-replies; single-step + continue;
single-register `P`/`p` packets (covering EAX..EDI, EIP, EFLAGS,
MMX, segments + FPU zero-fill); `host_io` extension (`vFile:open`,
`vFile:pread`, `vFile:close`, `vFile:fstat` — env-pinned mtime via
`OXIDEAV_TRACEVFW_FSTAT_MTIME`) over the primary DLL bytes plus
cascade-loaded module stubs synthesised as minimal valid PE32 images
(DOS + PE\0\0 + COFF + 224-byte Optional Header + `.text` + `.edata`
sections; full `IMAGE_EXPORT_DIRECTORY` per PECOFF §6.3 advertising
per-module curated name lists for kernel32/user32/msvcrt/advapi32/
gdi32/ole32/vfw32/msvfw32/winmm — every export emits an 8-byte stub
with int3 trap that triggers a host-side `stub_call` JSONL event
(deduped per stub VA) before the `0xC3` ret; `IMAGE_DEBUG_DIRECTORY`
at DataDirectory[6] points at a CodeView RSDS record with a
deterministic FNV-1a-derived GUID + `<basename>.pdb` filename so
`info sharedlibrary` shows a Symbols hint; env-pinned
`OXIDEAV_TRACEVFW_PE_TIMESTAMP` echoed in COFF header + Export
Directory for byte-reproducible output; `IMAGE_IMPORT_DIRECTORY` at
DataDirectory[1] declares per-cascade-module imports
(kernel32 leaf + user32/advapi32/vfw32/msvfw32 →
`kernel32!{LoadLibraryA, GetProcAddress}` etc.) visible to
`objdump -p`) so `add-symbol-file remote:kernel32.dll` clears GDB's
PE validator and `info functions` shows recognisable export names; `monitor_cmd` extension (`monitor
stats`, `monitor files`) for live host-side state; clean disconnect
on `vKill`/`D`. Bind `:0` to pick a free port; chosen port logged at
startup.

```
oxidetracevfw decode --gdb :0 --trace-output /tmp/decode.jsonl IR50_32.DLL frame.bmp
# attach with `gdb -ex 'target remote :NNNN'` — set breakpoints, walk MMX,
# watch a memory range, dump the codec's internal state.
```

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
| **`oxideav-videotoolbox`** | macOS (Apple Silicon + Intel Macs) | 🚧 H.264 + HEVC | 🚧 H.264 + HEVC | Roadmap: ProRes + JPEG (round 3); VP9 / AV1 / MPEG-2 (round 4). H.264 round-trip ~46 dB PSNR-Y, HEVC ~50 dB. AV1 hardware needs M3+. |
| **`oxideav-audiotoolbox`** | macOS | 🚧 AAC LC | 🚧 AAC LC | Round-2 SNR 36.7 dB on 440 Hz @ 128 kbit/s stereo. Roadmap: AAC HE, ALAC, AMR-NB/WB, iLBC. |
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
| **`oxideav-source`** | URI resolution + file reader + prefetching BufferedSource | ✅ `file://` driver; generic `SourceRegistry` for pluggable schemes |
| **`oxideav-http`** | HTTP / HTTPS source driver | ✅ `http://` + `https://` via pure-Rust `ureq` + `rustls` + `webpki-roots`; Range-request seeking |
| **`oxideav-generator`** | Synthetic media source (`generate://...` URIs) + zero-input filters | ✅ audio synth + image (xc/gradient/pattern/fractal/plasma/noise/label) + video (testsrc/smptebars/fractal_zoom/gradient_animate); ImageMagick/sox shorthands in `convert` verb (vector text → raster via scribe + raster) |
| **`oxideav-rtmp`** | RTMP ingest + push | ✅ Server accepts incoming publishers (AMF0 handshake, chunk stream demux) + client pushes to remote servers; pluggable key-verification hook; `rtmp://` registered as a `PacketSource` on `SourceRegistry` (FLV-style → `Packet`, time_base 1/1000) — pulled into `oxideav-cli` by the default-on `rtmp` feature |
| **`oxideav-sysaudio`** | Native audio output | ✅ Runtime-loaded backends (ALSA, PulseAudio, WASAPI, CoreAudio); no C build-time linkage. CoreAudio backend (round 8) now reports **real HAL latency** — sums `kAudioDevicePropertyLatency` + `BufferFrameSize` + `SafetyOffset` + `kAudioStreamPropertyLatency` via runtime-loaded `CoreAudio.framework`, BT-aware; falls back to software estimate if HAL unavailable. |
| **`oxideav-pipeline`** | Pipeline composition (source → transforms → sink) | ✅ JSON transcode-graph executor; pipelined multithreaded runtime |
| **`oxideav-scene`** | Time-based scene / composition model | 🚧 Scaffold — data model for PDF pages / RTMP streaming compositor / NLE timelines; renderer still stubbed |
| **`oxideav-audio-filter`** | Audio effects & conversions (streaming) | ✅ Volume, NoiseGate, Echo, Resample (polyphase windowed-sinc), Spectrogram |
| **`oxideav-image-filter`** | Single-frame image effects (stateless) | ✅ ~39 filter types / 47 factory names (blur / edge / resize / sharpen / unsharp / gamma / brightness / contrast / level / normalize / posterize / solarize / threshold / negate / sepia / modulate / grayscale / motion-blur / emboss / flip / flop / rotate / crop / vignette / colorize / equalize / auto-gamma / tint / sigmoidal-contrast / implode / swirl / despeckle / wave / spread / charcoal / convolve (custom N×N kernel) / polar (Cartesian↔polar) / morphology (dilate/erode/open/close, 3×3 square or cross) / perspective (4-corner homography, bilinear) / distort (barrel/pincushion radial-polynomial) / tilt-shift (selective Gaussian masked by horizontal in-focus band) / composite (12 Porter-Duff ops: over/in/out/atop/xor/plus/multiply/screen/overlay/darken/lighten/difference, via two-input adapter)) |
| **`oxideav-pixfmt`** | Pixel-format conversion + palette + dither | ✅ YUV↔RGB matrix, chroma subsampling, palette quantisation (median-cut / k-means), Floyd-Steinberg dither |

</details>

<details>
<summary><strong>Subtitles</strong> (click to expand)</summary>

All text formats parse to a unified IR (`SubtitleCue` with rich-text
`Segment`s: bold / italic / underline / strike / color / font / voice /
class / karaoke / timestamp / raw) so cross-format conversion preserves
as much styling as each pair can represent. Bitmap-native formats (PGS,
DVB, VobSub) decode directly to `Frame::Video(Rgba)`.

**Text formats** — in `oxideav-subtitle`:

| Format              | Decode | Encode | Notes |
|---------------------|:------:|:------:|-------|
| **SRT** (SubRip)    | ✅ | ✅ | `<b>/<i>/<u>/<s>`, `<font color>` hex + 17 named, `<font face size>` |
| **WebVTT**          | ✅ | ✅ | Header, STYLE ::cue(.class), REGION, inline b/i/u/c/v/lang/ruby/timestamp, cue settings |
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
| **ASS / SSA**       | ✅ | ✅ | Script Info + V4+/V4 Styles (BGR+inv-alpha) + override tags (b/i/u/s/c/fn/fs/pos/an/k/kf/ko/N/n/h). Animated tags (`\t`, `\fad`, `\move`, `\clip`, `\fscx/y`, `\frz`, `\blur`) preserved as opaque raw so text survives round-trip |

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
