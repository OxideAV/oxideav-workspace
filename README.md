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
  `remux` / `transcode` / `run` / `validate` / `dry-run` / `convert`) and
  `oxideplay` (reference SDL2 + TUI player).

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
| MP4       | ✅ | ✅ | ✅ | mp4/mov/ismv brands; faststart; iTunes ilst; fragmented demux + mux (DASH/HLS/CMAF) + sidx/mfra/tfra; AC-3/E-AC-3/DTS sample-entry FourCCs |
| AVI       | ✅ | ✅ | ✅ | LIST INFO, avih duration; idx1 keyframe-index seek |
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
| **Vorbis** | ✅ ~95% — all residue types per RFC 5215 | 🚧 ~88% — BitrateTarget {Low/Medium/High/HighTail} bank + per-target stereo crossover + 3-class residue for High/HighTail + per-target silence percentile + residue begin offset + frame-level global M/S correlation override on Low + spread (rotation) parameter encoder (peak-to-RMS scoring picks SPREAD_NONE/LIGHT/NORMAL/AGGRESSIVE per band) + mono dynalloc band-energy boost (one outlier band ≥ 6 dB above median gets one extra pulse-budget quanta); ffmpeg cross-decodes |
| **Opus** | 🚧 ~88% — full TOC + RFC 7845 pre-skip + output_gain + §4.3.7.2 CELT de-emphasis + §4.3.5 CELT auto-scale (libopus interop 10-26 dB on 5 fixtures) + §4.2.7.4 integer silk_log2lin + §4.2.7.8 Q23 shell-pulse + §4.2.7.8.6 LCG dither + §4.2.7.5.5 NLSF stage-2 residual decoding + §4.2.7.5.5 LSF interpolation + §4.2.7.5.7/8 Q12-saturated Levinson stability check + §4.2.7.9.1 rewhitening scaffold (out_history ring); SILK NB/MB libopus interop 16-17 dB. Spec-correct routines are on hot-path-gated — full ≥ 20 dB target needs Q15 fixed-point synthesis filter rework + coordinated encoder LTP-feedback scale fix | ✅ ~85% — CELT full-band + SILK NB/MB/WB + Hybrid mono/stereo at 10/20 ms; ffmpeg + libopus cross-decode clean |
| **MP1** | ✅ 100% — all modes | ✅ ~95% — CBR + psy-driven VBR (192 kbps → 192.0 measured) |
| **MP2** | ✅ 100% — all modes | ✅ ~95% — CBR + VBR + intensity-stereo joint (-11 to -17% on correlated input) |
| **MP3** | ✅ ~95% — MPEG-1 Layer III M/S | 🚧 ~84% — CBR + VBR + MS-stereo + MPEG-1/2/2.5 intensity-stereo + Annex D Psy-1 (24 Bark partitions, peak-detection tonality, TMN/NMT offsets) on long + short blocks; mixed-block window-switching + simple-mask FFT lift |
| **AAC** | 🚧 ~84% — AAC-LC + HE-AACv1 SBR + HE-AACv2 PS + LATM + PCE + LD/ELD AudioSpecificConfig parse + LD/ELD MDCT/IMDCT 480/512-sample kernels + LD sine half-windows + LD overlap-add filterbank + USAC (objectType 42) AudioSpecificConfig scaffold; lacks LD/ELD raw_data_block frame decode, LD-SBR wiring, USAC frame body, ELD low-overlap window | 🚧 ~78% — LC + HE-AACv1/v2 + PNS + 5.1/7.1 + gapless + AscBuilder + Bark-band PE/SMR psy default-on across mono/v2/stereo (M/S CPE IMDCT side-lobe leakage closed via use_tns gate; pns-noise gate) |
| **CELT** | ✅ ~95% — full §4.3 pipeline | 🚧 ~88% — mono + stereo intra-only long-block + short-block on transients + per-band TF + LM=0/1/2 + comb pitch pre-filter + anti-collapse flag + LM heuristic + RFC 6716 §4.3.4.4 spread parameter encoder (peak-to-RMS scoring picks SPREAD_NONE/LIGHT/NORMAL/AGGRESSIVE per band) + RFC §4.3.3 mono dynalloc band-energy boost |
| **Speex** | ✅ ~95% — all NB 1-8 + WB 1-4 + UWB folding + intensity stereo + RFC 5574 in-band | ✅ ~95% — full NB + WB ladder + UWB + folding + RFC 5574 |
| **GSM 06.10** | ✅ 100% — full RPE-LTP | ✅ 100% — full RPE-LTP (incl. WAV-49) |
| **G.711** (μ-law / A-law) | ✅ 100% — ITU tables | ✅ 100% — ITU tables |
| **G.722** | ✅ 100% — 64 kbit/s QMF + dual-band ADPCM | ✅ 100% |
| **G.723.1** | ✅ 100% — 5.3k ACELP + 6.3k MP-MLQ | ✅ 100% — both rates |
| **G.728** | ✅ 100% — LD-CELP 50-order + ITU Annex B + §3.7 + §5.5 postfilter | ✅ 100% |
| **G.729** | 🚧 ~70% — CS-ACELP with non-spec codebook tables (audible but not bit-exact vs ITU) | 🚧 ~70% — symmetric to decoder; same non-spec tables |
| **IMA-ADPCM (AMV)** | ✅ 100% | ✅ 100% |
| **8SVX** | ✅ 100% | ✅ 100% |
| **iLBC** (RFC 3951) | ✅ 100% — NB 20/30 ms + RFC §4.6 enhancer + §4.7 synth shift | ✅ 100% — NB 20/30 ms LPC + LSF split-VQ + §3.6 residual CB + opt-in §3.1 HP biquad + §3.5.1 position-bit + variable start_idx via Appendix A.20 frame_classify; voiced 30 ms +1.4 dB SNR vs round 22 |
| **AC-3** (Dolby Digital) | ✅ ~95% — full decode + downmix (90+ dB vs ffmpeg) | 🚧 ~92% — acmod 1/2/3/6/7 + LFE (0–120 Hz spectral cap) + rematrix + transient detector + DBA with tonal-vs-noise psy classification + 5-fbw coupling + per-channel D15/D25/D45 chexpstr + E-AC-3 indep + dep substream encode with adaptive expstr selection (~430 bits/ch saved at D45) |
| **AC-4** (Dolby) | 🚧 ~88% — A-SPX + DRC + DE walker + 60+ ETSI Huffman codebooks + ASPX_ACPL_1/2/3 + 5_X/7_X channel walkers + 5_X ACPL_3 mch synthesis + mono/stereo/joint short-frame sf_data(ASF) + SSF bitstream walker + SSF PCM synthesis + SNF spectral-noise inject + §5.2.5.2.2 Heuristic Scaling + Tables 18 & 79 EMDF payloads_substream + emdf_payload_config parser (full conditional gates incl. variable_bits(5) extension) + §5.7.9.3.3 DRC PCM gain application API (`drc_raw_to_linear` 6 dB/step + dialnorm correction + 5.1 default channel-group map + planar in-place per-subframe apply) + DE walker hardening (4 edge-case paths) | — |
| **MIDI** (SMF) | ✅ ~95% — SMF Type 0/1/2 → PCM via 32-voice mixer + DAHDSR + pitch bend + GM modulator chain + SF2 (sm24/stereo/mod-env/RBJ LPF) + SFZ + DLS Level 1/2 + `SmfPlayer::with_instrument`; lacks RP-001 file-format spec coverage | — synthesis only |
| **NSF** (Nintendo Sound Format) | 🚧 ~50% — NSF v1.x + NSFe header parse; full 256 official 6502 opcodes (151 mnemonics × all addressing modes + cycle counts); 4 of 5 APU channels (Pulse 1/2 + Triangle + Noise LFSR; DMC partial — DAC level only); non-linear mixer; 525× realtime; lacks ~80 unofficial-opcode semantics, DMC DMA, expansion chips (VRC6/VRC7/FDS/MMC5/N163/Sunsoft) | — synthesis only |
| **Shorten** (.shn) | 🚧 ~85% — clean-room rebuild from `docs/audio/shorten/` (orphan master 2026-05-08; pre-rebuild history on `old` branch). Round 1: SHN header + ajkg magic + all 10 FN commands (DIFF0-3 / QUIT / BLOCKSIZE / BITSHIFT / QLPC / ZERO / VERBATIM) + Rice-Golomb residuals (uvar/svar/ulong/slong) + filetypes 2/3/5 (u8 / s16hl / s16lh); 38 tests green; mean-estimator ±1 drift on bshift>0 lossy + missing filetypes (ulaw/s8/s16/u16 family) + format-v1 fixture confirmation deferred to round 2 | 🚧 ~50% — `#[cfg(test)]`-gated reference encoder for self-roundtrip; production encoder (predictor search + energy-width + TR.156 §3.3 residual-width heuristic) deferred |
| **TTA** (True Audio) | ✅ ~95% — clean-room rebuild from `docs/audio/tta-cleanroom/` (orphan master 2026-05-06; pre-rebuild history on `old` branch). TTA1 format=1 + format=2 password decode + `oxideav-core::Decoder` integration + `--features trace` 18-event tape per spec/06 § Auditor PASS on spec/06 conformance + §1..§11 ordering / count discipline; libtta-side lockstep deferred pending a checked-in reference tape | — |
| **aptX** (classic + HD) | 🚧 ~70% — 4-band 2-stage dyadic QMF + Jayant ADPCM + codeword dither + 8-block parity-rotation sync; ~22 dB self-roundtrip; bit-exact blocked on Qualcomm-NDA QMF/quantizer tables | — |

</details>

<details>
<summary><strong>Video</strong> (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **MJPEG** | ✅ ~95% — baseline + progressive 4:2:0/4:2:2/4:4:4/grey + SOF9 arithmetic Q-coder | ✅ ~90% — baseline + progressive (SOF2 spectral selection) |
| **FFV1** | ✅ 100% — v3 all coder_types + 4:2:0/4:4:4 YUV + RGB+alpha + 9..16-bit; bit-exact ffmpeg | ✅ 100% — v3 range-coded + Golomb-Rice + 10-bit YUV + RGB+alpha; bit-exact ffmpeg |
| **MPEG-1 video** | ✅ ~95% — I+P+B | ✅ ~95% — I+P+B + half-pel diamond ME (QP-tuned Lagrangian biases scaling as 4·sqrt(qp) / 2·sqrt(qp) / 4·qp) + activity-based per-MB QP on I-pictures (variance ranking → top-quartile MBs at QP-2, bottom-quartile at QP+1 via 5-bit `quantizer_scale_code`) + per-encoder B-frame QP offset; +1.4-2.5 dB PSNR-Y vs round 38 |
| **MPEG-2 video (H.262)** | ✅ ~95% — I+P+B + alternate_scan + q_scale_type + field/interlaced + 4:2:2/4:4:4 + dual-prime | ✅ ~80% — I+P GOPs + 4:2:2/4:4:4 chroma + field-DCT interlaced encode (Yuv422P / Yuv444P factories); ffmpeg cross-decode at all chroma layouts |
| **MPEG-4 Part 2** | ✅ ~85% — I+P+B-VOP + 4MV + ¼-pel + field-MV/DCT + GMC + data partitioning + RVLC | 🚧 ~88% — I+P+B + 1MV/4MV (incl. under DP) + intra-in-P + ¼-pel + multi-warp GMC + static-sprite + DP + RVLC + MPEG-quant; lacks dquant, sprite_brightness_change, B+GMC trajectory warp |
| **Theora** | ✅ ~95% — I+P (no B); 1080p + 4 corpus fixtures bit-exact | ✅ ~95% — I+P + INTER_MV_FOUR + scene-change keyframe (per-MB SAD threshold) + two-pass complexity-driven QI + full DCT token coverage (tokens 1-6 multi-block EOB + 23-31 combined zero-run+value); ffmpeg cross-decode 46.95 dB on 256×256 |
| **H.263** | 🚧 ~80% — I+P + half-pel + Annexes D/E/F/G/I/J/K/M/N; lacks L-W | 🚧 ~65% — I+P + diamond ME + Annexes F/J/D/N/G/I/M/K; lacks intra-in-P MVD, plus L/P/Q/R/S/T/U/V |
| **H.261** | ✅ ~95% — I+P QCIF/CIF + integer-pel + loop filter | ✅ ~95% — I+P QCIF/CIF + spiral+diamond ME (8-conn refinement, ~80% fewer SADs than flat scan) + Encoder trait + registry + GQUANT-from-bitrate; 45 dB at 64 kbit/s QCIF (vs ffmpeg h261enc ~30 dB) |
| **MS-MPEG-4** (v1/v2/v3) | 🚧 ~30% — clean-room scaffold; v3 intra 3-tier ESC + custom intra-DC VLC + region_060988 ESC extension cluster (24 slices, 6 G-descriptor pointer-blocks per spec/08 §2.2 + spec/14 §2.1); ffmpeg-DIV3 fixtures non-conforming per spec/13 (decoder bit-correct, fixture is the bug); blocks remaining (G0..G3 packed-Huffman, v3 alt-MV VLC, ESC slice-content semantics) tracked in #303 | — |
| **H.264** | 🚧 ~80% — I/P/B + 4:2:0/4:2:2/4:4:4 + CAVLC + CABAC (full Tables 9-12..9-33 ctxIdx 0-1023) + DPB + B-pyramid POC + 8 SEI payload types + Level 1b sizing; lacks MBAFF, SVC/3D/MVC | 🚧 ~80% — I + P (1MV/4MV, ¼-pel) + B 16x16/16x8/8x16/B_8x8 + per-cell mixed B_8x8 + B_Skip/B_Direct_16x16 + weighted pred + CABAC I/P/B at 4:2:0/4:2:2/4:4:4 (Tables 9-25..9-33 transcribed verbatim from H.264 08/2024 — fixed silent ffmpeg interop divergence at 4:4:4 from missing ctxIdx 460-1023 init); ffmpeg bit-exact PSNR_Y 44.20 dB |
| **H.265 (HEVC)** | 🚧 ~72% — I/P/B 8-bit + Main 10/12 + 4:2:0/4:2:2/4:4:4 + SAO + deblock + bit-depth-aware pipeline + Profile-4 4:2:2 inter chroma residual y-placement now uses `sub_height_c()` (was hard-coded `/2`, off for SubHeightC=1); HEIF/HEIC corpus 14/14 (tolerance tier); textured 4:2:2 P-slice cu_qp_delta + last_sig_coeff bin-sequence drift still pending docs trace (#444) | 🚧 ~75% — Baseline I+P + Main P + B (mini-GOP > 1 at 8/10/12-bit + 8-bit 4:4:4) + AMP + dedicated HBD + 4:4:4 P/B writers; lacks SAO encoder RDO, deblock auto-derive, HBD 4:4:4, AMP/merge/B_Skip at HBD+4:4:4 |
| **H.266 (VVC)** | 🚧 ~50% — 4:2:0 IDR intra + ALF/SAO/CC-ALF + P/B merge+skip + HMVP + MMVD + CIIP + BCW + PH pred_weight_table + BDOF + GPM (geometric partitioning per Tables 36+37, §8.5.4 + §8.5.7 blend) + AMVR helper module (Table 16 AmvrShift + Tables 89/90 init) + PicturePlane u8/u16 HBD; lacks DMVR/PROF, affine, full mvd_coding | 🚧 ~40% — forward CABAC + DCT-II + flat quant + §7.3.10.11 three-pass residual encoder + per-CTU SAO RDO + IDR pipeline + per-CTB ALF on/off RDO (post-SAO SSE_Y compare against §7.4.3.18 fixed filter set 0); 30 dB at QP=26, 40 dB at QP=0 |
| **VP6** | ✅ ~95% — full FLV playback (845/845 sample frames); Huffman coefficient path on `use_huffman==1` | 🚧 ~88% — keyframe + skip + inter + real DCT residual (43 dB internal) + iterative diamond qpel ME (8-conn × ≤6 iterations, ±6 qpel, ≤48 probes per MB) + scene-change-driven golden refresh + INTER_FOURMV + Huffman inter encode + bool/Huffman RDO + PID rate controller (PI + derivative term, `kd=0` recovers PI exactly) + Trellis-style AC quantisation (per-coef RD pass on inter residual; default-on, `allow_trellis=false` opts out) |
| **VP8** | ✅ 100% — I+P + 6-tap luma/chroma + per-MB loopfilter slab + persistent ref/mode_deltas + correct §16.3 split_mv_tree + RFC 6386 §17.1 default_mv_context high-bit probs + RFC §18.1 luma-MV doubling + RFC §20.13 sixtap H/V intermediate clamp; entire 15-fixture corpus uniformly bit-exact | 🚧 ~96% — I+P + B_PRED intra-in-P + SPLIT_MV + alt-ref/golden + Lagrangian RDO + segment QP/LF + per-frequency AC/DC deltas + libvpx-shape Trellis (`vp8_optimize_b`-analogue forward DP per coef position with q→q-1 candidates, ctx-tracked rate calc, distortion `(q-mag)²·step²/2`, runs before EOB-trim — −1.4% bytes for −0.02 dB PSNR-Y) + activity-driven AQ (variance + 16·Laplacian-edge population quartiles → 4-segment QP delta, no new header bits) + rate-aware sub-pel ME + two-pass ABR + opt-in psy-RDO + opt-in ARNR NLM temporal denoiser; default Trellis/AQ are opt-in |
| **VP9** | 🚧 ~85% — keyframe + inter + segmentation + bit-accurate MV + compressed-header probs + show_existing_frame DPB + 10 directional intra modes + COMPOUND_PRED + INTERINTRA + per-frame CDF refinement + sharpness-aware loopfilter; chroma bit-exact | 🚧 ~35% — keyframe with all intra modes + simple P-frame single-ref + per-frame I/P QP allocation + per-block luma intra-mode RDO across {DC, V, H, TM} (decoder-shape neighbour buffers + KF_Y_MODE_PROBS lookup) + QP-derived loop filter level (libvpx-shape `q*0.45 + 1` clamped) — smooth gradient 50.60 → 53.06 dB at base_q_idx=64 (+2.46 dB) |
| **AV1** | 🚧 ~72% — OBU + range coder + all intra preds + CDEF + LR + inter MC + palette + multi-ref compound + super-res; SVT-AV1 48/48; lacks intrabc | 🚧 ~55% — forward range coder + forward DCT-II 8/16/32 + full §7.3.10.11 coefficient emitter + streaming-precarry + partition/mode/TX emit + `tile::write_tile_group_intra_64` walks per-plane decode_coefficients mirror (1 luma TU at 64×64 + 2 chroma at 32×32 non-lossless 4:2:0; 256 × 4×4 luma in coded-lossless) — replaces the round-3 block-level `skip = 1` shortcut with 3 extra `txb_skip` symbols per block; cfl_idx mirrors decoder's `(bw.max(bh) <= 32)` test; self-roundtrip via own decoder bit-exact, dav1d cross-decode still rejects 64×64 (residuals all-zero by construction; real residual emit is round 41) |
| **Dirac / VC-2** | ✅ ~90% — VC-2 LD + HQ intra + Dirac core-syntax intra/inter + OBMC + 7 wavelets + 10/12-bit; ffmpeg bit-exact 8-bit 4:2:2/4:4:4 + 10-bit 4:2:0 | 🚧 ~91% — HQ + LD intra + Dirac core-syntax intra + core-syntax inter (OBMC + LeGall 5/3 wavelet residue) + 2-ref bipred B-picture with **per-block adaptive sub-pel-vs-integer-pel selection** (each MV scored at both refined sub-pel and nearest int-pel; lower SAD wins, four-way `(sub-pel, int-pel) × (ref1, ref2)` for `Ref1And2`); camera-pan bipred 48.10 → 52.53 dB ffmpeg cross-decode (+4.43 dB), translating-square 31.16 dB held (1-ref baseline cap), complementary-bars unchanged; default `bipred_mv_precision = qpel` (was integer-pel workaround) |
| **AMV video** | ✅ 100% — synthesised JPEG header + vertical flip | ✅ 100% — via MJPEG encoder |
| **ProRes** | ✅ ~95% — RDD 36 entropy + 8/10/12-bit (60-68 dB ffmpeg interop apcn + apch) + 4:4:4:4 alpha + interlaced (TFF/BFF + PAL 1080i50) + spec-compliant §7.5.1 level shift | ✅ ~90% — emits valid RDD 36; self-roundtrip ≥30 dB on all 6 profiles + interlaced + alpha + custom perceptual quant matrices (-20-29% bytes vs flat) |
| **EVC** (MPEG-5) | 🚧 ~70% — NAL + SPS/PPS/APS + §9.3 CABAC (Baseline + 51 Main init tables) + §8 intra (5-mode Baseline) + DCT-II + Baseline P/B inter + cbf!=0 residual + deblock + RPL non-IDR (full §7.3.7/§7.4.8 ref_pic_list_struct) + HMVP + multi-reference DPB + HMVP-as-AMVP fallback + POC reordering + spatial-neighbour MV grid AMVP + LTRP entries + ALF (§8.7.5 §9.3.5 luma 7×7 + chroma 5×5) + DRA (§8.7.6 §9.3.6); lacks IBC | — |
| **HuffYUV** / FFVHuff | 🚧 ~85% — clean-room rebuild from `docs/video/huffyuv/` (orphan master 2026-05-07; pre-rebuild history on `old` branch). Round 1: file-header + method dispatch (HFYU + FFVH FourCCs registered) + frame-layout (YUY2 / RGB24 / RGB32) + 6 predictors (predict_old / Left / Median / Gradient / LeftDecorr / GradientDecorr) + classic-tables RLE record format + per-plane canonical Huffman built longest-length-first from `tables/06..09`; round 2 added full encoder symmetry (v1.x precomputed-codes path + v2.x ClassicV2 reuse + CustomV2 build via package-merge length-limited Huffman + RLE encode + public BIH writer); 48 self-roundtrip tests green; lacks 10/12-bit FFVHuff family, interlaced field-stride=2, fast-LUT decoder | 🚧 ~85% — round-2 encoder mirrors decoder across all 6 predictors × 3 pixel formats with v1.x + v2.x extradata write; lacks lockstep against third-party HuffYUV/FFVHuff fixtures, 10/12-bit FFVHuff write-side |
| **Lagarith** | 🚧 ~80% — clean-room rebuild from `docs/video/lagarith/` (orphan master 2026-05-07; pre-rebuild history on `old` branch). Rounds 1-2 spec/00..06 implemented: frame layout + dispatcher table; modern range coder + Fibonacci-Zeckendorf probability prefix + Left + JPEG-LS clamped Median predictor + cross-plane G-pivot decorrelation + residual zero-run RLE escape with 256-entry permutation LUT + channel-header dispatcher; types 1/2/4/5/6/8/9 + new type 10 (YV12) decode + NULL-frame ("JUMP") replay via stateful `Decoder::decode_frame_with_prev`; types 3 (YUY2) / 7 (legacy RGB) / 11 (reduced-res) reject; 55 tests green; legacy ARITH (`spec/07` adaptive-CDF range coder) deferred | 🚧 ~50% — round-2 self-roundtrip-only encoder (SOLID/RGB/RGBA/YV12/uncompressed/null); byte-exact match vs proprietary encoder deferred to Auditor |
| **Ut Video** | ✅ ~95% — clean-room rebuild from `docs/video/utvideo/` (orphan master 2026-05-07; pre-rebuild history on `old` branch). 5 native FourCCs (ULRG / ULRA / ULY0 / ULY2 / ULY4) × 4 predictors (None / Left / Gradient / Median) + RGB inter-plane decorrelation + RFC-1951-mirrored canonical Huffman + 32-bit-LE-word MSB-first bitstream; round 2 added a 415-line ~3000-cell pattern matrix (5 FourCC × 4 predictor × 8 patterns × ≤11 sizes × {1,2,4,8} slices); 65 tests green; ULH0/ULH2/10-bit ULY4/interlaced/raw-mode out-of-scope per spec/00 | ✅ ~95% — codec-internal encoder mirrors decoder for self-roundtrip; lacks UQ 10-bit / UM SymPack / interlaced encoders, raw-mode (Huffman bit clear) emit |
| **MagicYUV** | ✅ 100% — clean-room rebuild from `docs/video/magicyuv/` (orphan master 2026-05-06; pre-rebuild history on `old` branch). Auditor declared spec-coverage-complete on round 3 (`OxideAV/docs@298716f`): all 17 native v7 FOURCCs (8-bit + 10/12/14-bit M0/M2/M4); modular 8-bit Median + JPEG-LS Median (high bit depths) per spec/04 §4.4 audit-corrected; non-RFC-1951 longest-length-first cumulative Huffman per spec/05 §2.0; raw-mode fallback; interlaced field-stride=2; AVI 1.0 + OpenDML 2.0 super-index demuxer; `--features trace` JSONL emitter strict-jq-line-diff-equal to the cleanroom Python ref's `--trace`. 142/142 cumulative cross-validation cases (56 round-1 + 80 high-bit-depth + 2 interlaced + 4 encoder-roundtrip) all pass against the Validator-certified Python reference codec | ✅ 100% — `encode_frame` / `encode_avi` / `encode_avi_opendml` covering all 17 FOURCCs across Left / Gradient / Median + Huffman / raw + multi-RIFF AVIX segmentation with `indx` super-indexes; doesn't pursue proprietary-encoder-side "Dynamic" strategy or `ix00` per-stream chunks (intentional carve-outs per spec/04 §3 + spec/06 §6.1: muxer territory, not codec territory) |
| **Cinepak** (CVID) | ✅ ~85% — clean-room rebuild from `docs/video/cinepak/` (orphan master 2026-05-08; pre-rebuild history on `old` branch). Round 1 spec/00..05 implemented: frame header (`movi.cvid`) + multi-strip layout with Y-coordinate sentinel + V1 (4-byte) and V4 (12-byte) codebooks + intra (0x3000/0x3200) + inter with skip + selector-bit spillover across flag-word boundaries + grayscale family (0x24xx/0x26xx) + spec §3 YUV→RGB inverse with truncation-toward-zero on U/2 + clamp; CVID FourCC registered; RGB24 + Gray8 output; 43 tests green; lacks selective-update fixture validation (FFmpeg never emits 0x21xx/0x23xx/0x25xx/0x27xx), Sega FILM container, encoder | — |
| **SVQ1** (Sorenson) | 🚧 ~30% — frame-header + I/P/P-nonref + multistage QT walker; flat-fill output (~11 dB Y) — blocked on docs (§14.10/§14.11 L=4 + L=5 codebook bytes missing) | — |
| **Indeo 2** (RT21/IV20) | 🚧 ~15% — frame-header (`'RF'` magic + table selectors + dims) + structural pipeline; mid-grey placeholder — blocked on docs (143-symbol Huffman + four delta tables missing) | — |

</details>

<details>
<summary><strong>Image</strong> (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **PNG / APNG** | ✅ 100% — 5 color types × 8/16-bit + all 5 filters + APNG animation | ✅ 100% — same matrix + APNG emit |
| **GIF** | ✅ 100% — GIF87a/89a + LZW + interlaced + animation | ✅ 100% — GIF89a + animation + per-frame palettes |
| **WebP VP8L** | ✅ 100% — full lossless; 7/7 BitExact vs `dwebp` | ✅ ~99% — subtract-green + predictor + colour transform + RDO + K=16 meta-Huffman + cache-bits + LZ77 16384-px window/256-deep chains + near-lossless + palette RDO bias + three-pass cost-modelled LZ77 (Viterbi-style optimal DP at ≥256×256) + multi-iteration Huffman codebook refit + predictor-tile-bits + EntropyImage tile-bits RDO sweeps + Shannon-entropy scoring in **both** predictor and colour-transform paths; landscape-256 stays 1.0124× cwebp (within rounding); portrait/brick natural fixtures take small additional gains from the colour-transform entropy switch |
| **WebP VP8** | ✅ 100% — via VP8 (whole 15-fixture corpus uniformly bit-exact) + bit-exact YUV→RGB + fancy chroma upsample | 🚧 ~94% — VP8 I-frame + ALPH + per-segment QP/LF + per-frequency AC/DC deltas + quality-driven quant matrix curve + Trellis quant + rate-aware sub-pel ME (upstream vp8); animated mixed lossy/lossless ANIM/ANMF with file-level ICCP/EXIF/XMP write (spec-mandated chunk order: ICCP-after-VP8X, EXIF/XMP after last ANMF) + canvas-level VP8X ALPHA flag fidelity (set only when an animation frame carries non-opaque alpha); standalone `encode_vp8l_argb_with_metadata` one-shot std-only entry point (auto-promotes to extended VP8X iff alpha or any metadata); dwebp cross-decode clean; 350K+ fuzz runs/min clean |
| **JPEG** (still) | ✅ ~95% — via MJPEG codec | ✅ ~90% — via MJPEG codec |
| **TIFF** (6.0) | ✅ ~90% — II/MM + BigTIFF (read) + 6 photometrics (WhiteIsZero/BlackIsZero/RGB/Palette/CMYK/YCbCr) + 1/4/8/16-bit + None/PackBits/LZW/Deflate + horizontal predictor + strips + tiles + multi-page; bit-exact ImageMagick/tiffcp interop; lacks CCITT G3/G4, JPEG-in-TIFF, BigTIFF write, tile write, planar layout | ✅ Gray8/Gray16/RGB24/Palette8 — None/PackBits/LZW/Deflate, single+multi-page |
| **BMP** | ✅ ~95% — 1/4/8/16/24/32-bit + V4/V5 + RLE4/RLE8 | ✅ ~95% — 1/4/8/16/24/32-bit + indexed (palette8) + RLE4/RLE8 + V5 header |
| **Netpbm** (PBM/PGM/PPM/PNM/PAM) | ✅ ~95% — all 8 magics P1-P7 at 1/8/16-bit + 6 standard PAM TUPLTYPEs; lacks user-defined TUPLTYPE strings | ✅ ~95% — picks closest binary form per input PixelFormat; ASCII on demand |
| **ICO / CUR** | ✅ ~95% — multi-resolution + BMP/PNG sub-images + CUR hotspot | ✅ ~90% — emits BMP (PNG for ≥256×256) |
| **JPEG 2000** | ✅ ~88% — Part-1 baseline + multi-tile + MQ + EBCOT + 5/3 + 9/7 + JP2 + 5 progression orders + POC + HTJ2K (Part 15) FBCOT cleanup/SigProp/MagRef; HTJ2K 5/3 + 9/7 fixtures bit-exact (pblk plumbing closed) | ✅ ~85% — 5/3 lossless + 9/7 irreversible RGB + 5 progression orders + POC + PPM/PPT + HTJ2K Part-15 cleanup-pass encoder round 3 (multi-component RGB + MCT/RCT; ojph_expand bit-exact) |
| **JPEG XL** | 🚧 ~62% — rounds 1+2+3+4 against ISO/IEC 18181-1:2024 final core (master @ 70348c05). **4 fixtures decode PIXEL-CORRECT** vs expected.png (PNG-decoder-backed byte-for-byte assertion via `png = "0.18"` dev-dep): pixel-1x1, gray-64x64, gradient-64x64-lossless, palette-32x32. Coverage: C.2.1 + H.2 + H.3 + H.4.1+H.4.2 + H.6.4 inverse Palette + H.6.3 inverse RCT (6 modes × 6 perms) + H.6.2 inverse Squeeze + H.5 Self-correcting predictor (WpState). 206 tests. Round-4 found 6 spec divergences: C.3.3 ReadUint missing `- msb - lsb` term (multi-channel EOF root cause); H.5 subpred[3] sign / error2weight `>> shift` / s_init `- 1` / err[i] rounding / property[15] timing. FDIS-2021 typo #5 fixed by 2024; typos #1-#4 retained. Decoder rounds 7-11 + encoder rounds 1-6 retired 2026-05-08 under fruits-of-poisonous-tree (`old` branch); v0.0.5/v0.0.6/v0.0.7 yanked. grey_8x8 (cjxl 0.11.1) blocked at sym-stream prelude clcl decode (round-5 priority) | — RETIRED 2026-05-08, re-author after decoder forward progress |
| **JPEG XS** | 🚧 ~70% — ISO/IEC 21122 Part-1 codestream + inverse 5/3 DWT + Annex C/D/F/G entropy + quant + colour transforms + multi-component (4:2:2/4:2:0) + multi-level DWT cascade + CAP-bit decoder | 🚧 ~52% — multi-component (Nc 1/3 + RCT cpih ∈ {0,1,3}) + multi-decomp NL ∈ {1,2} + odd dimensions + Dr=0 VLC + Fq=8 lossy + 4:2:2/4:2:0 sub-sampling + Star-Tetrix Cpih=3 + vertical-prediction VLC; bytes -26% vs raw at q=8; lacks significance coding, NL_x≠NL_y, NLT, per-band Q |
| **AVIF** | 🚧 ~75% — HEIF→AV1 + grid + imir/clap/colr/pixi/pasp + HDR metadata (clli/mdcv/cclv) + multi-extent iloc reassembly + AV1 wrap pass-through (raw OBU → ISOBMFF without re-encode); standalone-friendly via `registry` feature; gated on AV1 decoder completeness | — |
| **DDS** (DirectDraw Surface) | ✅ ~92% — DDS_HEADER + DDS_HEADER_DXT10 + uncompressed (10 layouts) + BC1/BC2/BC3/BC4/BC5/BC7 decompression + BC6H modes 1+11 to RGBA half-float + mipmap chain + 6-face cubemaps + DX10 texture arrays + full 132-entry DXGI table + .dds container demuxer/muxer; lacks BC6H modes 0/2-10/12/13 (12 delta-encoded modes need per-mode bit-allocation tables) | ✅ ~62% — uncompressed + BC1/BC2/BC3/BC4/BC5 encoders (furthest-point endpoint heuristic, ~25 dB PSNR-RGB on natural gradients); lacks BC6H/BC7 encoders + mipmap-chain emission |
| **OpenEXR** | 🚧 ~65% — magic + 8 required attributes + chlist HALF/FLOAT/UINT + NO_COMPRESSION/ZIP/ZIPS/RLE + tiled (single_tile/ONE_LEVEL) + sub-sampled chroma + predictor +128 centring fix; exrmetrics cross-validates; PIZ decode blocked on a clean-room wavelet+Huffman trace doc (the public openexr.com page only summarises PIZ in one line; OpenEXR source is barred as reference); lacks PIZ/B44/B44A/DWAA-B, multi-part, deep-data, multi-level mip/rip-maps | ✅ ~75% — RGBA HALF/FLOAT/UINT scanline + ZIP/ZIPS/RLE/uncompressed + tiled-output single-part ONE_LEVEL with NONE/ZIP/ZIPS/RLE (sets `single_tile` bit + `tiles`/`chunkCount`/`type=tiledimage` + INCREASING_Y row-major offset table + per-tile `tx|ty|lvlx|lvly|size|payload`; edge tiles handled) + multi-part scanline output (sets multipart bit + per-part `name`/`type=scanlineimage`/`chunkCount` + double-NUL header terminator + per-part offset tables + `part_number`-prefixed chunks); exrmetrics + exrmultipart cross-validate bit-exact |
| **Farbfeld** | ✅ 100% — full spec (8B `farbfeld` magic + W/H u32 BE + RGBA u16 BE rows) | ✅ 100% — full spec |
| **HDR** (Radiance RGBE) | ✅ ~95% — `#?RADIANCE`/`#?RGBE` magic + KEY=VALUE attrs + new-RLE encode/decode + old-RLE decode + 8 axis-flag combos + shared-exponent codec | ✅ ~96% — new-RLE + old-RLE encode + XYZE↔RGB helpers (sRGB/Rec.709 + Radiance matrices) + tone-mapping (Linear/Gamma/Reinhard/ACES); lacks CRLF line endings on write + non-canonical axis-flag combos |
| **QOI** | ✅ 100% — full one-page spec (header + 6 chunk types + hash + end marker); byte-exact roundtrip vs all 8 phoboslab/qoi reference fixtures | ✅ 100% — byte-exact match to reference encoder on all 8 fixtures |
| **TGA** | ✅ ~98% — types 1/2/3/9/10/11 read at 8/15/16/24/32 bpp + TGA 2.0 extension area body decode + postage-stamp (thumbnail) extract; auto-flip top/bottom origin; magick cross-validated | ✅ 100% — all six spec image types (1/2/3/9/10/11) write + TGA 2.0 extension area + thumbnail emit |
| **ICER** (JPL) | 🚧 ~75% — Mars-rover heritage; bit-plane scan orchestration + compressed-segment encode/decode + multi-segment + float filters A-G + IPN 42-155 §III.B H/V/D context model + stripe-ordered scan + multi-packet ordering | ✅ ~75% — compressed + uncompressed segments + multi-segment + all 8 filters (Q + A-G) + quota-controlled encoding (`with_byte_budget` hard cap + `with_target_bytes` soft target) — Mars-rover MSB-down progressive truncation |
| **WBMP** (WAP Bitmap) | ✅ 100% — Type 0 monochrome + MBI variable-length integers (Types 1+ never normatively defined) | ✅ 100% — Type 0 |
| **PCX** (ZSoft) | ✅ ~95% — 1/2/4/8 bpp planar + packed-bits (mono/CGA/EGA/VGA) + 24 bpp RGB planar + CGA 4-colour palette resolver + DCX multi-page container; RLE codec; magick cross-validated | ✅ ~95% — 6 write paths (mono / EGA / CGA / packed-4bpp / 8 bpp indexed / 24 bpp planar) + DCX container; lacks 4 bpp × 4 planes |
| **ILBM** (Amiga IFF) | ✅ ~85% — BMHD/CMAP/CAMG/BODY chunks + uncompressed planar + ByteRun1 RLE + EHB (32→64 palette mirror) + HAM6/HAM8 decode (per-pixel R/G/B state, control-op + channel-widening); under existing `oxideav-iff` crate; lacks PBM chunky variant, ANIM, SHAM/PCHG | ✅ ~75% — `IlbmMuxer::with_mode(MuxerMode)` lifts streaming muxer to feature parity with `encode_ilbm` across `IndexedAuto / Ham6 / Ham8 / Ehb / Pbm`; `IlbmMuxer::with_masking` exposes `HasMask` / `HasTransparentColor` keying; `bmhd.transparent_color` now written directly when source alpha < 0x80 (was nearest-RGB-matching, round-tripped wrong colour); 13 new round-4 tests including 3 ImageMagick `magick convert` cross-decode (indexed + PBM bit-exact via `ilbmtoppm`; HAM6 dim-only); lacks ANIM op-5 byte-vertical-delta encode, PCHG big-format encode, CRNG / CCRT colour-cycling chunks |
| **PICT** (Apple QuickDraw) | ✅ ~80% — v1 + v2 opcode walkers + drawing-command rasteriser (Bresenham/mid-point/active-edge-list for line/rect/oval/poly/round-rect/arc) + DirectBitsRect packType 1/2/3/4 + Region paths (inversion-encoded parser) + Compressed/UncompressedQuickTime opcode skip; lacks clip-region honouring, pattern fills, text rasterisation, embedded JPEG decode | ✅ ~65% — `PictBuilder` + low-level `build_*_op` byte builders for every v2 drawing-command family (line / line-from / rect / round-rect / oval / arc / polygon / region rect + inversion-encoded) + `RGBFgCol` / `RGBBkCol` / `PnSize` / `OvSize` state opcodes + DirectBitsRect packType 1/2/3/4 emit (packType=3 = A1R5G5B5 + u16-PackBits per row); ImageMagick `magick` cross-decode bit-exact on packType-3 + drawing-only + region-rect PICTs; 96 tests green |
| **SVG** | ✅ ~88% — full shape set + path + g + defs + gradients + text/tspan + filter pass-through + mask + clipPath + use/symbol + svgz inflate + SMIL animate/set/animateTransform snapshot at arbitrary `t` (begin / dur clock-values incl. H:M:S / repeatCount / keyTimes+values segmented interpolation / from-to-by shorthand / discrete+linear calcMode / componentwise colour lerp) + minimal CSS cascade (`<style>` blocks + style="..." with tag/class/id selectors + CSS2.1 specificity) | ✅ ~85% — round-trips shape/stroke/fill/gradient/transform/mask/clipPath + `parse_svg_with_extras` / `write_svg_with_extras` PreservedExtras side-channel that re-emits captured `<style>` / `<filter>` / `<animate>` / `<foreignObject>` fragments alongside the rasterised scene |
| **PDF** | ✅ ~60% — bytes → Scene via xref + recursive object parser (FlateDecode) + content-stream operator parser; `/Info` → Metadata; lacks encryption | ✅ ~70% — PDF 1.4 multi-page + paths + gradients + strokes + transforms + opacity + clip + RGBA images + `/Info` dict; lacks text, JPEG passthrough |

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
| **`oxideav-image-filter`** | Single-frame image effects (stateless) | ✅ Blur (separable Gaussian, per-plane selector), Edge (3×3 Sobel), Resize (Nearest / Bilinear, YUV-subsampling-aware) |
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

<details>
<summary><strong>Scaffolds</strong> — API registered, pixel/sample decode not yet implemented (click to expand)</summary>

| Codec | Status |
|-------|--------|
| **AVIF** | end-to-end HEIF→AV1 wired but gated on AV1 decoder completeness — see Image table |

(JPEG XL, JPEG XS, EVC, MIDI all moved out of "scaffolds" — they now have working decoders or substantial pixel-emit pipelines; see their dedicated rows.)

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
