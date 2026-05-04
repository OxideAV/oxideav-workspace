# oxideav

A **100% pure Rust** media transcoding and streaming framework. No C libraries, no FFI wrappers, no `*-sys` crates — just Rust, all the way down.

## Goals

- **Pure Rust implementation.** Never depend on any C library — directly or transitively. Every codec, container, and filter is implemented from the spec.
- **Clean abstractions** for codecs, containers, timestamps, and streaming formats.
- **Composable pipelines**: media input → demux → decode → transform → encode → mux → output, with pass-through mode for remuxing without re-encoding.
- **Modular workspace**: per-format crates for complex modern codecs/containers, a shared crate for simple standard formats, and an aggregator crate that ties them together behind Cargo features.

## Non-goals

- Wrapping existing C codec libraries.
- Perfect feature parity with FFmpeg on day one. Codec and container coverage grows incrementally.
- GPU-specific acceleration (may come later through pure-Rust compute libraries, but never C drivers).

## Workspace policy: clean-room, no external code

This is the **strict and universal rule** every contributor and every automated agent must follow. It is not a list of named libraries — it is a categorical prohibition:

> **No external library source code may be consulted, quoted, paraphrased, or used as a cross-check oracle while implementing any codec, container, protocol, or filter in this workspace.**

The rule applies to **every** external implementation, not a specific blocklist. That includes (but is in no way limited to): `ffmpeg` / `libav*`, `x264`, `x265`, `libvpx`, `libaom`, `dav1d`, `SVT-AV1`, `libvorbis`, `libopus`, `libspeex`, `fdk-aac`, `LAME`, `libjxl`, `jxlatte`, `jxl-rs`, `FUIF`, `brunsli`, `OpenJPEG`, `OpenJPH`, `Kakadu`, `schroedinger`, `xeve` / `xevd`, `VTM`, `JM`, `mp4v2`, every reference implementation distributed alongside a spec, and every third-party Rust crate that wraps or implements the same format (`lewton`, `claxon`, `image`'s codec submodules, `png`, `jpeg-decoder`, anything else of similar shape).

**"Cross-checking" counts.** Reading an external implementation "just to verify a table value" or "just to see how they handle this edge case" still contaminates the code. If you couldn't have written it without that reference, the resulting code is no longer clean-room.

**Allowed references:**
- Spec PDFs (ISO, ITU, ATSC, ETSI, RFC, IETF drafts, Annex documents)
- Clean-room behavioural-trace docs commissioned for this project (these are explicitly source-quote-free; see `docs/image/jpeg2000/openjph-htj2k-trace-analysis.md` and `docs/image/jpegxl/libjxl-trace-reverse-engineering.md` for examples)
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
  New sibling crates landed this session: `oxideav-evc` (MPEG-5 EVC,
  ISO/IEC 23094-1), `oxideav-jpegxs` (JPEG XS, ISO/IEC 21122),
  `oxideav-midi` (Standard MIDI File + soft-synth scaffold),
  `oxideav-pbm` (Netpbm: PBM/PGM/PPM/PNM/PAM).
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
- **Aggregator** — `oxideav` re-exports every enabled crate behind Cargo
  features. `Registries::with_all_features()` builds a registry covering
  every format compiled in. The `with_all_features_traced(callback)`
  variant invokes a callback before each crate registers — used by the
  CLI's `--debug` flag to bisect startup hangs.
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
| **Vorbis** | ✅ ~95% — all residue types per RFC 5215 | 🚧 ~70% — floor1 + ATH + trained VQ (-11.4%) + per-band point-stereo + floor0 LSP encoder; lacks bitstream-resident trained books |
| **Opus** | ✅ ~95% — CELT + SILK NB/MB/WB + Hybrid all frame sizes + RFC 7845 pre-skip + output_gain | ✅ ~85% — CELT full-band + SILK NB/MB/WB + Hybrid mono/stereo at 10/20 ms; ffmpeg + libopus cross-decode clean |
| **MP1** | ✅ 100% — all modes | ✅ ~95% — CBR + psy-driven VBR (192 kbps → 192.0 measured) |
| **MP2** | ✅ 100% — all modes | ✅ ~95% — CBR + VBR + intensity-stereo joint (-11 to -17% on correlated input) |
| **MP3** | ✅ ~95% — MPEG-1 Layer III M/S | 🚧 ~70% — CBR + VBR + MS-stereo + MPEG-1/2/2.5 intensity-stereo encode; lacks Bark-partition psy-1 |
| **AAC** | 🚧 ~80% — AAC-LC + HE-AACv1 SBR + HE-AACv2 PS + LATM + PCE; lacks LD/ELD, USAC | 🚧 ~72% — LC + HE-AACv1/v2 + PNS + 5.1/7.1 + gapless + AscBuilder + opt-in Bark-band PE/SMR psy model (+5 dB SDR / -22% bytes on harmonic) |
| **CELT** | ✅ ~95% — full §4.3 pipeline | 🚧 ~70% — mono + stereo intra-only long-block + short-block on transients + per-band TF + LM=2 (480-sample MDCT) + comb pitch pre-filter |
| **Speex** | ✅ ~95% — all NB 1-8 + WB 1-4 + UWB folding + intensity stereo + RFC 5574 in-band | ✅ ~95% — full NB + WB ladder + UWB + folding + RFC 5574 |
| **GSM 06.10** | ✅ 100% — full RPE-LTP | ✅ 100% — full RPE-LTP (incl. WAV-49) |
| **G.711** (μ-law / A-law) | ✅ 100% — ITU tables | ✅ 100% — ITU tables |
| **G.722** | ✅ 100% — 64 kbit/s QMF + dual-band ADPCM | ✅ 100% |
| **G.723.1** | ✅ 100% — 5.3k ACELP + 6.3k MP-MLQ | ✅ 100% — both rates |
| **G.728** | ✅ 100% — LD-CELP 50-order + ITU Annex B + §3.7 + §5.5 postfilter | ✅ 100% |
| **G.729** | 🚧 ~70% — CS-ACELP with non-spec codebook tables (audible but not bit-exact vs ITU) | 🚧 ~70% — symmetric to decoder; same non-spec tables |
| **IMA-ADPCM (AMV)** | ✅ 100% | ✅ 100% |
| **8SVX** | ✅ 100% | ✅ 100% |
| **iLBC** (RFC 3951) | ✅ 100% — NB 20/30 ms + RFC §4.6 enhancer + §4.7 synth shift | ✅ ~95% — 20/30 ms LPC + LSF split-VQ + RFC §3.6 residual CB search + opt-in §3.1 HP biquad |
| **AC-3** (Dolby Digital) | ✅ ~95% — full decode + downmix (90+ dB vs ffmpeg) | 🚧 ~75% — acmod 1/2/3/6/7 + LFE + rematrix + transient detector + DBA + 5-fbw coupling + E-AC-3 indep + dep substream encode |
| **AC-4** (Dolby) | 🚧 ~50% — A-SPX + DRC + DE walker + 60 ETSI Huffman codebooks + ASPX_ACPL_1/2/3 + 5_X/7_X channel walkers + mono/stereo/joint short-frame sf_data(ASF) walk | — |
| **MIDI** (SMF) | ✅ ~95% — SMF Type 0/1/2 → PCM via 32-voice mixer + DAHDSR + pitch bend + GM modulator chain + SF2 (sm24/stereo/mod-env/RBJ LPF) + SFZ + DLS Level 1/2 + `SmfPlayer::with_instrument`; lacks RP-001 file-format spec coverage | — synthesis only |
| **Shorten** (.shn) | ✅ ~95% — all 6 internal_ftype variants + DIFF0-3 predictors + adaptive Rice; bit-exact ffmpeg silence | ✅ ~90% — DIFFn predictor pick + Rice-k closed-form + FN_ZERO; bit-exact roundtrip; lacks QLPC, BITSHIFT |
| **TTA** (True Audio) | 🚧 ~75% — two-mode adaptive Rice + 8-tap sign-LMS + fixed Stage-B predictor + inter-channel decorrelation + CRC32; bit-exact silence; LMS drift on non-trivial signals pending docs | — |
| **aptX** (classic + HD) | 🚧 ~70% — 4-band 2-stage dyadic QMF + Jayant ADPCM + codeword dither + 8-block parity-rotation sync; ~22 dB self-roundtrip; bit-exact blocked on Qualcomm-NDA QMF/quantizer tables | — |

</details>

<details>
<summary><strong>Video</strong> (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **MJPEG** | ✅ ~95% — baseline + progressive 4:2:0/4:2:2/4:4:4/grey + SOF9 arithmetic Q-coder | ✅ ~90% — baseline + progressive (SOF2 spectral selection) |
| **FFV1** | ✅ 100% — v3 all coder_types + 4:2:0/4:4:4 YUV + RGB+alpha + 9..16-bit; bit-exact ffmpeg | ✅ 100% — v3 range-coded + Golomb-Rice + 10-bit YUV + RGB+alpha; bit-exact ffmpeg |
| **MPEG-1 video** | ✅ ~95% — I+P+B | ✅ ~90% — I+P+B (half-pel ME) |
| **MPEG-2 video (H.262)** | ✅ ~70% — I+P+B + alternate_scan + q_scale_type; lacks field/interlaced, 4:2:2/4:4:4, dual-prime | ✅ ~60% — shares MPEG-1 encoder |
| **MPEG-4 Part 2** | ✅ ~85% — I+P+B-VOP + 4MV + ¼-pel + field-MV/DCT + GMC + data partitioning + RVLC | 🚧 ~70% — I+P+B + 1MV/4MV + intra-in-P + ¼-pel + single-warp GMC + DP + RVLC; lacks Inter4MV under DP, multi-warp GMC, Sprite VOP |
| **Theora** | ✅ ~95% — I+P (no B); 1080p + 4 corpus fixtures bit-exact | ✅ ~85% — I+P + INTER_MV_FOUR |
| **H.263** | 🚧 ~80% — I+P + half-pel + Annexes D/E/F/G/I/J/K/M/N; lacks L-W | 🚧 ~65% — I+P + diamond ME + Annexes F/J/D/N/G/I/M/K; lacks intra-in-P MVD, plus L/P/Q/R/S/T/U/V |
| **H.261** | ✅ ~95% — I+P QCIF/CIF + integer-pel + loop filter | ✅ ~85% — baseline I+P with ME ±15 |
| **MS-MPEG-4** (v1/v2/v3) | 🚧 ~25% — clean-room scaffold; v3 intra 3-tier ESC + custom intra-DC VLC; ffmpeg-DIV3 fixtures non-conforming per spec/13 (decoder bit-correct, fixture is the bug) | — |
| **H.264** | 🚧 ~75% — I/P/B + 4:2:0/4:2:2 + CAVLC + CABAC + DPB + B-pyramid POC + 8 SEI payload types + Level 1b sizing; lacks 4:4:4, MBAFF, SVC/3D/MVC | 🚧 ~55% — I + P (1MV/4MV, ¼-pel) + B-slice + weighted pred + 4:2:2/4:4:4 IDR + CABAC I/P + level_idc auto-derive; lacks B-slice CABAC encode, P/B at 4:4:4 |
| **H.265 (HEVC)** | 🚧 ~70% — I/P/B 8-bit + Main 10/12 + 4:2:0/4:2:2/4:4:4 + SAO + deblock + bit-depth-aware pipeline; HEIF/HEIC corpus 14/14 (tolerance tier); lacks 4:2:2 cu_qp_delta CABAC | 🚧 ~60% — Baseline I+P + Main P + B (mini-GOP=2) + AMP + Main 10/12 IDR + Main 4:4:4 IDR; lacks mini-GOP > 2 at 10/12-bit, P/B at 4:4:4 |
| **H.266 (VVC)** | 🚧 ~30% — 4:2:0 IDR intra + ALF/SAO/CC-ALF + P/B merge+skip + HMVP + MMVD + CIIP + BCW + PH pred_weight_table + BDOF (module landed, leaf-CU wiring pending); lacks GPM, AMVR, DMVR/PROF, affine | 🚧 ~5% — forward CABAC + DCT-II + flat quant only; residual emit pending |
| **VP6** | ✅ ~95% — full FLV playback (845/845 sample frames); Huffman coefficient path on `use_huffman==1` | 🚧 ~70% — keyframe + skip + inter + real DCT residual (43 dB internal) + ¼-pel ME + golden-frame refresh + INTER_FOURMV + Huffman coefficient encode |
| **VP8** | ✅ ~98% — I+P + 6-tap luma/chroma + per-MB loopfilter slab + persistent ref/mode_deltas; lacks mb_y≥3 P-MV divergence (4 ReportOnly) | 🚧 ~83% — I+P + B_PRED intra-in-P + SPLIT_MV + alt-ref/golden + Lagrangian RDO + segment QP/LF + per-frequency AC/DC deltas |
| **VP9** | 🚧 ~70% — keyframe + inter + segmentation + bit-accurate MV + compressed-header probs + show_existing_frame DPB; chroma bit-exact | 🚧 ~5% — round 1: profile 0 keyframe DC-only, ffmpeg cross-decode 45.87 dB Y on smooth fixture |
| **AV1** | 🚧 ~72% — OBU + range coder + all intra preds + CDEF + LR + inter MC + palette + multi-ref compound + super-res; SVT-AV1 48/48; lacks intrabc | 🚧 ~30% — forward range coder + intra DC + 4×4 DCT + streaming-precarry + partition/mode/TX emit; dav1d ≤64×64; lacks B-frames, full inter |
| **Dirac / VC-2** | ✅ ~90% — VC-2 LD + HQ intra + Dirac core-syntax intra/inter + OBMC + 7 wavelets + 10/12-bit; ffmpeg bit-exact 8-bit 4:2:2/4:4:4 + 10-bit 4:2:0 | 🚧 ~65% — HQ + LD intra + Dirac core-syntax intra + core-syntax inter (OBMC); ¼-pel ME camera-pan +25 dB Y; lacks 2-ref bipred, wavelet residue |
| **AMV video** | ✅ 100% — synthesised JPEG header + vertical flip | ✅ 100% — via MJPEG encoder |
| **ProRes** | ✅ ~95% — RDD 36 entropy + 8/10/12-bit (60-68 dB ffmpeg interop apcn + apch) + 4:4:4:4 alpha + interlaced (TFF/BFF + PAL 1080i50) + spec-compliant §7.5.1 level shift | ✅ ~90% — emits valid RDD 36; self-roundtrip ≥30 dB on all 6 profiles + interlaced + alpha + custom perceptual quant matrices (-20-29% bytes vs flat) |
| **EVC** (MPEG-5) | 🚧 ~40% — NAL + SPS/PPS/APS + §9.3 CABAC (Baseline + 51 Main init tables) + §8 intra (5-mode Baseline) + DCT-II + Baseline P/B inter + cbf!=0 residual + deblock; Main-profile tools pending; lacks RPL non-IDR, HMVP, ALF, DRA, IBC | — |
| **HuffYUV** / FFVHuff | ✅ ~70% — v2 yuv420p/422p/rgb24/bgra (LEFT/GRADIENT) + v3 gray8/yuv 444/422/420 8-bit; bit-exact ffmpeg on v2 yuv422p LEFT; lacks ≥9-bit, gbrp/gbrap, encoder, interlaced | — |
| **Lagarith** | 🚧 ~50% — SOLID modes (gray/color/RGBA) bit-exact via 5-frame ffmpeg AVI; ARITH modes Unsupported — blocked on docs (probability VLC + rescale tables missing from trace doc) | — |
| **Ut Video** | ✅ ~80% — ULRG + ULY2 with NONE/LEFT/MEDIAN bit-exact ffmpeg interop (6 fixtures); UL classic 8-bit family; lacks UQ 10-bit, UM SymPack, interlaced | — |
| **MagicYUV** | ✅ ~75% — all 7 8-bit format codes × 3 predictors (LEFT/GRADIENT/MEDIAN); 27/27 bit-exact ffmpeg interop; lacks 10/12/14-bit, interlaced, horizontally-tiled slices, encoder | — |
| **Cinepak** (CVID) | ✅ ~85% — V1+V4 codebooks + INTER skip-MB + 4:2:0 output; ~27 dB PSNR on testsrc; lacks 8-bit paletted, encoder | — |
| **SVQ1** (Sorenson) | 🚧 ~30% — frame-header + I/P/P-nonref + multistage QT walker; flat-fill output (~11 dB Y) — blocked on docs (§14.10/§14.11 L=4 + L=5 codebook bytes missing) | — |
| **Indeo 2** (RT21/IV20) | 🚧 ~15% — frame-header (`'RF'` magic + table selectors + dims) + structural pipeline; mid-grey placeholder — blocked on docs (143-symbol Huffman + four delta tables missing) | — |

</details>

<details>
<summary><strong>Image</strong> (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **PNG / APNG** | ✅ 100% — 5 color types × 8/16-bit + all 5 filters + APNG animation | ✅ 100% — same matrix + APNG emit |
| **GIF** | ✅ 100% — GIF87a/89a + LZW + interlaced + animation | ✅ 100% — GIF89a + animation + per-frame palettes |
| **WebP VP8L** | ✅ 100% — full lossless; 7/7 BitExact vs `dwebp` | ✅ ~85% — subtract-green + predictor + colour transform + RDO + K=4 meta-Huffman + near-lossless + palette RDO bias |
| **WebP VP8** | ✅ ~98% — via VP8 + bit-exact YUV→RGB + fancy chroma upsample; +1 LSB loopfilter rounding gap on a few P-frame edges | 🚧 ~75% — VP8 I-frame + ALPH + per-segment QP/LF; animated mixed lossy/lossless; webpinfo + dwebp cross-decode clean |
| **JPEG** (still) | ✅ ~95% — via MJPEG codec | ✅ ~90% — via MJPEG codec |
| **TIFF** (6.0) | ✅ ~65% — II/MM + 4 photometrics + 1/4/8/16-bit + None/PackBits/LZW/Deflate + horizontal-differencing predictor; bit-exact ImageMagick interop; lacks BigTIFF, tiles, CCITT, JPEG-in-TIFF, YCbCr/CMYK, multi-page, encoder | — |
| **BMP** | ✅ ~95% — 1/4/8/16/24/32-bit + V4/V5 + RLE4/RLE8 | ✅ ~80% — 24/32-bit (V5) |
| **Netpbm** (PBM/PGM/PPM/PNM/PAM) | ✅ ~95% — all 8 magics P1-P7 at 1/8/16-bit + 6 standard PAM TUPLTYPEs; lacks user-defined TUPLTYPE strings | ✅ ~95% — picks closest binary form per input PixelFormat; ASCII on demand |
| **ICO / CUR** | ✅ ~95% — multi-resolution + BMP/PNG sub-images + CUR hotspot | ✅ ~90% — emits BMP (PNG for ≥256×256) |
| **JPEG 2000** | ✅ ~85% — Part-1 baseline + multi-tile + MQ + EBCOT + 5/3 + 9/7 + JP2 + 5 progression orders + POC + HTJ2K (Part 15) FBCOT cleanup/SigProp/MagRef; HTJ2K 5/3 fixtures bit-exact; lacks 9/7 lossy pblk plumbing | ✅ ~80% — 5/3 lossless + 9/7 irreversible RGB + 5 progression orders + POC + PPM/PPT |
| **JPEG XL** | 🚧 ~45% — `jxlp` container + 2019 committee-draft Modular + 2021 FDIS through round 10 (ANS + LfGlobal + GlobalModular + cl_code + kRCT/kPalette/kSqueeze); blocked on inverse-palette negative-index docs gap | 🚧 ~25% — round 4 lossless modular + Gradient predictor + ANS + multi-group |
| **JPEG XS** | 🚧 ~70% — ISO/IEC 21122 Part-1 codestream + inverse 5/3 DWT + Annex C/D/F/G entropy + quant + colour transforms + multi-component (4:2:2/4:2:0) + multi-level DWT cascade + CAP-bit decoder | — |
| **AVIF** | 🚧 ~60% — HEIF→AV1 + grid + imir/clap/colr/pixi/pasp; standalone-friendly via `registry` feature; gated on AV1 decoder completeness | — |
| **SVG** | ✅ ~85% — full shape set + path + g + defs + gradients + text/tspan + filter pass-through + mask + clipPath + use/symbol + svgz inflate + animate/set@t=0 | ✅ ~80% — round-trips shape/stroke/fill/gradient/transform/mask/clipPath |
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
to start. The core `oxideav` library remains 100% pure Rust.

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
```

Two global flags help diagnose startup or codec issues:

- `--debug` enables debug log output to stderr through the `log` facade.
  Every crate that emits `log::debug!` flows through here — including
  `Registries::with_all_features` which prints one line per crate as it
  registers (the last line printed before a hang names the offending
  crate).
- `--debug-output FILE` redirects debug log output to a file instead of
  stderr (implies `--debug`; stderr stays clean).

`oxideplay --job <file>` runs a job where `@display` / `@out` binds
to the SDL2 player sink; other outputs (file paths) write to disk in
the same run.

## Building

> **First clone? Run `./scripts/update-crates.sh` before `cargo build`.**
> The workspace tracks only the aggregator glue (`oxideav-cli`,
> `oxideplay`, `oxideav-tests`); every per-format codec lives in its
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

Every per-format codec — and the aggregator `oxideav` itself — lives in
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
