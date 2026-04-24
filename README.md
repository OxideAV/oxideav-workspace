# oxideav

A **100% pure Rust** media transcoding and streaming framework. No C libraries, no FFI wrappers, no `*-sys` crates — just Rust, all the way down.

## Goals

- **Pure Rust implementation.** Never depend on `ffmpeg`, `libav`, `x264`, `libvpx`, `libopus`, or any other C library — directly or transitively. Every codec, container, and filter is implemented from the spec.
- **Clean abstractions** for codecs, containers, timestamps, and streaming formats.
- **Composable pipelines**: media input → demux → decode → transform → encode → mux → output, with pass-through mode for remuxing without re-encoding.
- **Modular workspace**: per-format crates for complex modern codecs/containers, a shared crate for simple standard formats, and an aggregator crate that ties them together behind Cargo features.

## Non-goals

- Wrapping existing C codec libraries.
- Perfect feature parity with FFmpeg on day one. Codec and container coverage grows incrementally.
- GPU-specific acceleration (may come later through pure-Rust compute libraries, but never C drivers).

## Workspace layout

The workspace is a set of Cargo crates under `crates/`, grouped by role:

- **Infrastructure** — `oxideav-core` (primitives: Packet / Frame / Rational /
  Timestamp / PixelFormat / ExecutionContext), `oxideav-codec` (Decoder /
  Encoder traits + registry), `oxideav-container` (Demuxer / Muxer traits +
  registry), `oxideav-pipeline` (source → transforms → sink composition).
- **I/O** — `oxideav-source` (generic SourceRegistry + file driver +
  BufferedSource), `oxideav-http` (HTTP/HTTPS driver, opt-in via feature).
- **Effects + conversions** — `oxideav-audio-filter` (Volume / NoiseGate /
  Echo / Resample / Spectrogram), `oxideav-image-filter` (stateless
  single-frame Blur / Edge / Resize), `oxideav-pixfmt` (pixel-format
  conversion matrix + palette generation + dither).
- **Job graph** — `oxideav-job` (JSON transcode graph + pipelined
  multithreaded executor).
- **Containers** — one crate each for `oxideav-ogg` / `-mkv` / `-mp4` /
  `-avi` / `-iff`. Simple containers (WAV, raw PCM, slin) live inside
  `oxideav-basic`.
- **Codec crates** — one crate per codec family; see the
  [Codecs table](#codecs) below for the per-codec status. Tracker formats
  (`oxideav-mod`, `oxideav-s3m`) are decoder-only by design. Codec scaffolds
  that register-but-refuse (JPEG XL, JPEG 2000, AVIF) reserve their
  codec ids so the API surface stays forward-compatible.
- **Aggregator** — `oxideav` re-exports every enabled crate behind Cargo
  features. `Registries::with_all_features()` builds a registry covering
  every format compiled in.
- **Binaries** — `oxideav-cli` (the `oxideav` CLI: `list` / `probe` /
  `remux` / `transcode` / `run` / `validate` / `dry-run`) and `oxideplay`
  (reference SDL2 + TUI player).

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
Pull only `oxideav-core` (types), `oxideav-codec` (trait + registry),
and the codec itself:

```toml
[dependencies]
oxideav-core = "0.0"
oxideav-codec = "0.0"
oxideav-g711 = "0.0"   # or any other codec crate
```

```rust
use oxideav_codec::CodecRegistry;
use oxideav_core::{CodecId, CodecParameters, Frame, Packet, TimeBase};

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

The canonical walkthrough of the `send_packet` / `receive_frame` /
`flush` / `reset` loop lives in
[oxideav-codec's README](https://github.com/OxideAV/oxideav-codec).
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
| Matroska  | ✅ | ✅ | ✅ | MKV/MKA/MKS; DocType-aware probe; Cues-based seek |
| WebM      | ✅ | ✅ | ✅ | First-class: separate fourcc, codec whitelist (VP8/VP9/AV1/Vorbis/Opus); inherits Matroska Cues seek |
| MP4       | ✅ | ✅ | ✅ | mp4/mov/ismv brands, faststart, iTunes ilst metadata; sample-table seek |
| AVI       | ✅ | ✅ | ✅ | LIST INFO, avih duration; idx1 keyframe-index seek |
| MP3       | ✅ | ✅ | ✅ | ID3v2/v1 tags + cover art, Xing/VBRI TOC seek (+ CBR fallback), frame sync with mid-stream resync |
| IFF / 8SVX| ✅ | ✅ | — | Amiga IFF with NAME/AUTH/ANNO/CHRS |
| IVF       | ✅ | — | — | VP8 elementary stream container |
| AMV       | ✅ | — | — | Chinese MP4 player format (RIFF-like) |
| FLV       | ✅ | — | — | Flash Video — MP3/AAC/H.264 audio + VP6f/VP6a/H.264 video + AMF0 onMetaData |
| WebP      | ✅ | — | — | RIFF/WEBP (lossy + lossless + animation) |
| PNG / APNG| ✅ | ✅ | — | 8 + 16-bit, all color types, APNG animation |
| GIF       | ✅ | ✅ | — | GIF87a/GIF89a, LZW, animation + NETSCAPE2.0 loop |
| JPEG      | ✅ | ✅ | — | Still-image wrapper around the MJPEG codec |
| BMP       | ✅ | ✅ | — | Windows bitmap — DIB headers BITMAPINFOHEADER / V4 / V5, 1/4/8/16/24/32-bit; also exposes the DIB helpers used by ICO / CUR sub-images |
| ICO / CUR | ✅ | ✅ | — | Windows icon + cursor — multi-resolution, BMP and PNG sub-images |
| slin      | ✅ | ✅ | — | Asterisk raw-PCM: .sln/.slin/.sln8..192 |
| MOD / S3M / STM | ✅ | — | — | Tracker modules (decode-only by design; STM is structural-parse only) |

Cross-container remux works for any pair whose codecs don't require
rewriting (FLAC ↔ MKV, Ogg ↔ MKV, MP4 ↔ MOV, etc.).

</details>

### Codecs

<details>
<summary><strong>Audio</strong> (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **PCM** (s8/16/24/32/f32/f64) | ✅ all variants | ✅ all variants |
| **slin** (Asterisk raw PCM) | ✅ .sln/.slin/.sln16/.sln48 etc. | ✅ same — headerless S16LE |
| **FLAC** | ✅ bit-exact vs reference | ✅ bit-exact vs reference |
| **Vorbis** | ✅ matches lewton/ffmpeg (type-0/1/2 residue); 22-28× IMDCT via precomputed cosine + std::simd chunked f32x8 | ✅ stereo coupling + ATH floor |
| **Opus** | ✅ CELT mono+stereo (incl. transient/short blocks); SILK NB/MB/WB mono 10+20+40+60 ms; SILK stereo | ✅ CELT full-band + transient short-blocks + SILK NB/MB/WB mono 20 ms + SILK NB stereo (24-29 dB round-trip SNR) |
| **MP1** | ✅ all modes, RMS 2.9e-5 vs ffmpeg | ✅ CBR (greedy allocator, 89 dB PSNR on pure tone) |
| **MP2** | ✅ all modes, RMS 2.9e-5 vs ffmpeg | ✅ CBR mono+stereo (greedy allocator, ~31 dB PSNR) |
| **MP3** | ✅ MPEG-1 Layer III (M/S stereo) | ✅ CBR mono+stereo |
| **AAC** | ✅ AAC-LC (mono+stereo, M/S, IMDCT) + HE-AACv1 SBR + HE-AACv2 PS spec-accurate. Round-5 output-scale fix (0.88 → 48.28 dB). **Round 6**: hybrid sub-QMF filterbank (§8.6.4.3) — QMF bands 0/1/2 split via 13-tap FIRs, +4.65 dB on 1 kHz stereo HE-AACv2 (19.26 → 23.91 dB). **Round 7**: IPD/OPD phase correction applied to PS mixing matrix (§8.6.4.6.3.2, Table 8.31 π/4 grid, 3-tap phase smoothing, complex envelope-border interpolation). Activates on streams emitting `enable_ipdopd=1` (afconvert's `aacp` doesn't). HE-AACv1 unchanged at 48.28 dB. | ✅ AAC-LC (mono+stereo + PNS + intensity stereo + pulse data) + HE-AACv1 mono encoder |
| **CELT** | ✅ full §4.3 pipeline (energy + PVQ + IMDCT + post-filter) | ✅ mono + stereo dual-stereo (intra-only long-block; energy + PVQ + fMDCT) |
| **Speex** | ✅ NB modes 1-8 + WB via QMF+SB-CELP (+ formant postfilter); intensity stereo | ✅ full NB ladder (sub-modes 1-8, 2.15-24.6 kbit/s) + WB sub-mode-1 (QMF split, 16 kHz) |
| **GSM 06.10** | ✅ full RPE-LTP | ✅ full RPE-LTP (standard + WAV-49) |
| **G.711** (μ-law / A-law) | ✅ ITU tables | ✅ ITU tables (pcm_mulaw / pcm_alaw + aliases) |
| **G.722** | ✅ 64 kbit/s QMF + dual-band ADPCM (37 dB PSNR, self-consistent tables) | ✅ same roundtrip |
| **G.723.1** | ✅ full-synthesis stateful decoder (5.3k ACELP + 6.3k MP-MLQ) | ✅ 5.3k ACELP + 6.3k MP-MLQ (20-24 dB round-trip PSNR via own decoder) |
| **G.728** | ✅ LD-CELP 50-order backward-adaptive + ITU Annex B codebooks + §3.7 Barnwell window + §5.5 postfilter | ✅ exhaustive 128×8 analysis-by-synthesis |
| **G.729** | ✅ CS-ACELP (non-spec tables, produces audible speech) | ✅ symmetric encoder |
| **IMA-ADPCM (AMV)** | ✅ | ✅ (33.8 dB PSNR roundtrip) |
| **8SVX** | ✅ | ✅ via FORM/8SVX container muxer |
| **iLBC** (RFC 3951) | ✅ Narrowband 20 ms + 30 ms frames, enhanced pitch-emphasis variant | — |
| **AC-3** (Dolby Digital) | ✅ Full decode pipeline + FFT IMDCT + §7.8 downmix. Sine 92.02 dB vs ffmpeg. Transient fixture stuck at 15 dB — 2 rounds of investigation, not root-caused. | ✅ **Round 7**: encoder quality lifted substantially via 4 fixes (group-synced mantissa emitter — biggest; per-block D15 refresh; backward D15 legaliser; accurate bit budget). PSNR our-enc → our-dec: sine 11 → **21 dB**, speech 19 → **32 dB**, stereo L/R 12/6 → **23/45 dB**. ffmpeg decodes all our output with zero errors. Short-block encode / coupling / rematrix pending. |
| **AC-4** (Dolby) | 🚧 Full A-SPX front-end (aspx_config/framing/delta_dir/hfgen_iwc + 18 Huffman codebooks + aspx_ec_data + master-freq-scale + QMF analysis/synthesis, 78-81 dB round-trip). **Round 8**: §5.7.6.4.2 per-envelope HF adjustment (Pseudocodes 76/80-83/90/91/95/106) wired end-to-end into `aspx_extend_pcm` — correctness test verifies output energy ratio follows parsed envelope deltas (e1 > 3×e0 for Δq=2 vs Δq=6). §5.7.6.4.3 noise generator (ASPX_NOISE 512-entry + `generate_qmf_noise`) + §5.7.6.4.4 tone generator (Table 196 SineTable + `(-1)^(sb+sbx)` sign) landed as standalone modules with end-to-end FFT probe test (HF energy > 2× baseline at 6.2 kHz). Decoder-side wiring of noise/tone on top of adjuster still pending. Remaining: complex-covariance TNS (chirp/α0/α1), P92-94 sinusoid location, P96-101 limiter, non-FIXFIX classes, cross-frame `Q_prev` continuity. | — |

</details>

<details>
<summary><strong>Video</strong> (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **MJPEG** | ✅ baseline + progressive 4:2:0/4:2:2/4:4:4/grey | ✅ baseline + progressive (SOF2 spectral selection) |
| **FFV1** | ✅ v3: 4:2:0/4:4:4 YUV + RGB via JP2K RCT + `extra_plane` alpha + 9..16-bit RGB (§3.7.2.1 BGR exception) + 10-bit YUV. All three coder_types (0/1/2) + cross-frame state retention for `intra=0` (round 6). ffmpeg interop bit-exact on long-GOP, Golomb 10-bit decode, state-delta `-context 1`. | ✅ v3 range-coded YUV + multi-slice + 10-bit YUV + RGB 8-bit via JP2K RCT. **Round 8 Golomb-Rice encode** (`coder_type=0`) — BitWriter + put_ur/sr_golomb + put_vlc_symbol with adaptive k + run-mode look-ahead emission + slice wrapper. **ffmpeg bit-exactly decodes our Golomb output** on flat/checker/random/multi-slice 8-bit YUV 4:2:0. 10-bit Golomb encode wired but blocked by up-front check pending fixture. |
| **MPEG-1 video** | ✅ I+P+B frames | ✅ I+P+B frames (half-pel ME, FWD/BWD/BI B-modes, 43 dB PSNR) |
| **MPEG-4 Part 2** | ✅ I + P + B-VOP with 4MV direct mode, half-pel MC, quarter-pel MC in B-VOPs, decode→display reorder. Field-MV decode + field-DCT residual reorder + alt-vertical-scan + field-sample MC for interlaced B-VOPs. Frame-7 MB(3,3) divergence hand-traced 3 rounds; spec-level VLC bit-exact on both sides; root cause unresolved. | ✅ I+P-VOP (41-43 dB). **Round 8 B-VOP encoder** — frame buffering + decode-order emit + MODB+MBTYPE+forward/backward MVD syntax. `-bf N` wires via `CodecParameters::options["bf"]`. Self-consistency 38.78 dB; through ffmpeg 25.40 dB overall (I 43.78, P 25-31, B 23-25). B-MB residual emit with cbpb!=0 disabled (needs pairing with dquant side channel, ~+2 dB when landed). |
| **Theora** | ✅ I+P frames | ✅ I+P frames incl. INTER_MV_FOUR (45 dB PSNR, 3.7× vs all-I) |
| **H.263** | ✅ I+P pictures, half-pel MC, Annex J deblock, Annex D URMV (baseline + **PLUSPTYPE Table D.3** w/ UUI range, round 6), Annex F (4MV + OBMC). 5-frame testsrc QCIF: 60–69 dB vs ffmpeg. Annex E SAC core (819 LOC, round 5); MB-layer VLC→SAC swap deferred. Long-clip ~1 dB/P-frame drift diagnosed as IDCT LSB accumulation (not OBMC-specific). | ✅ I+P pictures, diamond-pattern ME, 46 dB on sliding-gradient. **Round 7**: **Annex F encoder emission** (4MV + OBMC) — 3-pass (decide/emit/reconstruct), SAD-gated 4MV selection, H0/H1/H2 OBMC weights at encode matching decode. **ffmpeg decodes our Annex-F output at 40+ dB** (self-roundtrip 52-54 dB, ours-vs-ffmpeg cross-decode 40-99 dB). |
| **H.261** | ✅ I + P pictures on QCIF / CIF (integer-pel MC + optional loop filter); ffmpeg-PSNR harness: >66 dB intra, >68 dB clean P-chain | — |
| **MS-MPEG-4** (v1 / v2 / v3) | 🚧 Clean-room Implementer handoff per `docs/video/msmpeg4/` (spec/99 + 40 tables). **Round 8**: DC spatial predictor (§7.4.3 L/T/TL gradient, `|A-D|<|A-B|` → left else top) + AC-scan dispatcher (direction-based horizontal/vertical from DC predictor choice) + MCBPCY canonical Huffman wired into joint-VLC decode (cbp_cb/cbp_cr now real). Testsrc2 32×32 DIV3 vs ffmpeg: Cb 16.27 dB, Cr 16.06 dB, Y 5.30 dB. AC VLC gap (candidate VMAs `0x1c25fad0`/`0x1c25f6c8` still without Extractor dumps) is the unambiguous single remaining blocker. P-frames + v1/v2 I-frame also pending. | — |
| **H.264** | Full CABAC+CAVLC I/P/B slice decode (real-world MKV playback) | ✅ Baseline CAVLC (I+P, 49.9 dB) + Main-profile CABAC IDR (I-only, 41.6 dB) |
| **H.265 (HEVC)** | ✅ I/P/B slice decode, 8-bit 4:2:0 + SAO + deblock. Main 10 intra bit-exact. **Main 10 inter 24.19 → 24.77 dB** (round 7) after finding the real bug: **§8.5.3.2.8 TMVP BR/center positions needed 16×16 snap + same-CTB-row gate** — raw `(xPb+nPbW, yPb+nPbH)` was pulling TMVP from the wrong 4×4 cell. Frame 3 jumped 17→20 dB. Round 6's `split_transform_flag` 2Nx2N gate + §8.5.3.2.3 `NeighbourContext` partIdx suppression in merge/AMVP also landed. HEIF/HEIC decode (opt-in `heif` feature). Gaps: `interSplitFlag` force-split for non-2Nx2N (crashes in residual vs libx265), `part_mode` Nx2N bin-count bug, 12-bit, 4:2:2 / 4:4:4, AMP / scaling lists / tiles+WPP. | ✅ Baseline CAVLC I+P + **Main-profile CABAC P-slice encoder** (round 7) — integer-pel MVD, 2Nx2N only, DCT+flat-quant, local reconstruction matches our decoder pixel-exact. I 45 dB / P 31 dB via our decoder; ffmpeg accepts with zero errors at 26.82 dB. |
| **H.266 (VVC)** | 🚧 Full VVC front-end + CTU walker scaffold + leaf-CU syntax reader + §8.4.2 MPM list + §8.4.3 chroma derivation. **Round 8**: CBF reads (`tu_{y,cb,cr}_coded_flag` per §9.3.4.2.5 BDPCM/ISP-aware, Table 127 ctxInc) + `cu_qp_delta` (TR+EG prefix/suffix) + `cu_chroma_qp_offset` + last-sig-coeff position (context-coded prefix eqs 1555/1556 with luma offsetY `{0,0,3,6,10,15}`/chroma 20 + FL bypass suffix) + sub-block residual walker (reverse scan, `csbf_ctx_regular` neighbour rule, per-coeff sig_coeff_flag + abs_level_gt_1 + par + gt_3 + abs_remainder Rice/EGk + sign). `decode_ctu_full` orchestrates partitions → per-leaf syntax → per-leaf residual. 241 tests. `reconstruct_leaf_cu` still Unsupported — next: spec-exact `locNumSig`/`locSumAbsPass1`/`remBinsPass1` ctxInc threading, LFNST/MTS, §8.7.3 dequant, inverse transform. | ✅ **Round 8 encoder scaffold** — forward-bitstream emitter (VPS/SPS/PPS/PH/IDR) that parse-roundtrips through the decoder's front-end byte-for-byte. 8-bit 4:2:0, CTB=128, all tool flags off, empty coded-slice payload. PH emitted as standalone PH_NUT NAL. No residual / CABAC / pixel output yet. |
| **VP6** | ✅ Full FLV playback (845/845 frames of sample decode cleanly; range coder + MB-types + IDCT + MC + loop filter + vp6a alpha) | ✅ VP6F keyframe encoder with **round-8 AC coefficient emission** — forward 2D DCT-II + AC quantisation + zig-zag scan + run-length + bool-coded VP6_PCR_TREE walker + coefficient-model state mirroring decoder. Gradient content lifted **+10-14 dB vs DC-only** (Y 36-40 dB at QP 4-32 through ffmpeg decode). Known latent axis-transpose in our in-tree decoder (encoded streams fed to ffmpeg → correct orientation; self-decoded → transposed) — documented for follow-up. P-frames / MV encode / loop-filter emission / Huffman path pending. |
| **VP8** | ✅ I+P frames (6-tap sub-pel + MV decode + ref management) | ✅ I + P frames, all 5 intra modes + SPLIT_MV + loop filter (42-51 dB PSNR) |
| **VP9** | 🚧 Keyframe + inter + segmentation + bit-accurate §6.5 MV list + inter-mode ctx + full §6.3 compressed-header probs + §8.10 saved slots + `AboveNonzeroCtx` / `LeftNonzeroCtx` tracking. **Round 6**: fixed spec-correct `checkEob` reset after ZERO_TOKEN per §6.4.24. **Round 7**: ruled out 4 hypotheses (PARETO8 token-tree, intra vs inter dequant scales, CAT1-6 magnitude paths, skip_ctx hardcoded `bd.read(192)`). PSNR stuck at 10.96 dB across 3 rounds. Real issue appears deeper — likely uncompressed/compressed header parse mismatch that masks downstream reads. Inter frames nearly flat (std 2-6 vs ref 71.3) independent of keyframe state. Needs a different attack (bit-level trace from the header on known-simple content). | — |
| **AV1** | 🚧 OBU parse + range coder/CDFs + coeff decode + partition + transforms + all intra predictors w/ edge filter + §7.15 CDEF + Loop Restoration (Wiener+SGR) + inter MC (NEWMV/GLOBALMV) + ref-scaling. Round-5 chroma fix: gray U/V 12 → 48/45 dB. Rounds 6-7: read_block_tx_size + read_skip ordering + delta_q + delta_lf + use_intrabc + filter_intra_mode_info. **Round 8**: `palette_mode_info()` (§5.11.46, explicit Unsupported on activation mirroring use_intrabc) + inter `read_skip` reordered BEFORE `is_inter` per §5.11.18 + `read_var_tx_size()` recursive helper (§5.11.17) with `DEFAULT_TXFM_SPLIT_CDF` + TU stamping onto MI grid. **Gray P-frame Y +3.76 dB → 43.10** from the inter-skip-ordering fix. Testsrc/libaom fixtures now bail with palette Unsupported (aomenc defaults enable screen-content tools). Remaining: palette token decode §7.11.4, per-block `CurrentQIndex`/`DeltaLF[]` apply, filter-intra predictor wiring, var-tx residual-per-TU, `read_skip_mode` + `inter_segment_id`. | — |
| **Dirac / VC-2** | ✅ VC-2 LD + HQ intra end-to-end + Dirac core-syntax intra (VLC and AC paths) + core-syntax inter + OBMC motion comp + full IDWT (7 wavelets) + arithmetic coder + 10/12-bit output + frame-rate-aware timebase + pts passthrough. ffmpeg-interop tests: 8-bit 4:2:2 + 8-bit 4:4:4 + 10-bit 4:2:0. Gaps: VC-2 v3 asymmetric transforms (SMPTE ST 2042-1 ed-2 not in docs/), `Yuv422P12Le` / `Yuv444P12Le` variants not yet in oxideav-core. Round-8: `mean3` switched from `/3` to `div_euclid(3)` (floor, per §1.3). | ✅ HQ + LD intra encoders. **Round 8**: HQ path is **bit-exact through ffmpeg decode** (PSNR ∞ dB on 64×64 4:2:0 testsrc, gated at ≥48 dB). LD profile corrected to 0 per §D.1.1 + Annex C Level-1 conformant presets (`make_preset_sequence`). LD always emits non-ref 0xC8 (§D.1.1 prohibits 0xCC in LD). ffmpeg accepts LD streams but reads LL magnitude at ~12.7 dB PSNR — single LD slice field disagreement, not yet isolated. Multi-picture emitters landed for both profiles. |
| **AMV video** | ✅ (synthesised JPEG header + vertical flip) | ✅ (via MJPEG encoder, 33 dB PSNR roundtrip) |
| **ProRes** | 🚧 Self-roundtrip works (all six profiles with a simplified exp-Golomb entropy layer). FourCC dispatch for `apch` / `apcn` / `apcs` / `apco` / `ap4h` / `ap4x` is wired in MP4/MOV. **Real ffmpeg-produced `.mov` decode is blocked**: SMPTE RDD 36 (the authoritative bit-level spec) is not in `docs/video/prores/` — only Apple marketing whitepapers are. See `crates/oxideav-prores/SPEC_BLOCKED.md` for the unblock procedure. | ✅ Self-roundtrip encode at 44 dB PSNR (quant 4) — not interop-grade. |

</details>

<details>
<summary><strong>Image</strong> (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **PNG / APNG** | ✅ 5 color types × 8/16-bit, all 5 filters, APNG animation | ✅ same matrix + APNG emit |
| **GIF** | ✅ GIF87a/89a, LZW, interlaced, animation | ✅ GIF89a, animation, per-frame palettes |
| **WebP VP8L** | ✅ full lossless (Huffman + LZ77 + transforms) | ✅ lossless (subtract-green + predictor + colour transform, VP8X for RGBA) |
| **WebP VP8** | ✅ lossy (via VP8 decoder) | ✅ lossy (via VP8 I-frame + ALPH sidecar for RGBA) |
| **JPEG** (still) | ✅ via MJPEG codec | ✅ via MJPEG codec |
| **BMP** | ✅ 1/4/8/16/24/32-bit, BITMAPINFOHEADER / V4 / V5, RLE4 / RLE8 decompression | ✅ 24-bit + 32-bit with alpha (V5) |
| **ICO / CUR** | ✅ multi-resolution directory; BMP + PNG sub-images; CUR hotspot preservation | ✅ emits BMP sub-images (PNG sub-images for ≥ 256×256 per Vista spec) |
| **JPEG 2000** | 🚧 Part-1 baseline + multi-tile (§B.3) + MQ + EBCOT + 5/3 + 9/7 IDWT + tier-2 + LRCP / RLCP + JP2 wrapper. Two real bug fixes rounds 3+4 (MQ state-table `nlps`/`nmps` swap; T1 `pi`-flag §D.3.4). **Round 6**: built MQ trace harness — localized residual to forward 5/3 HH sub-band. **Round 7**: axis-order fix per T.800 §F.3.2 / §F.4.2 (VER→HOR forward, HOR→VER inverse). LL/HL/LH now bit-exact vs OpenJPEG at every MQ event; HH divergence pushed from event #14 to event #185 (12× more events match). Root cause NOT ε_b / M_b / missing_msb — verified numerically. OpenJPEG's two-step HH lifting rounds differently from literal §F-9/F-10. Multi-layer + user precinct grids + CPRL / PCRL / RPCL + Part-2 still pending. | ✅ 5/3 lossless + 9/7 irreversible RGB (forward RCT/ICT; JP2 box wrapper) |
| **JPEG XL** | 🚧 Signature + SizeHeader + partial ImageMetadata parse — Modular (MA-tree) and VarDCT pixel decode pipelines pending | — |
| **AVIF** | 🚧 **End-to-end decode now works** (round 5): HEIF box walker → AV1 OBU handoff → decoded `VideoFrame`. Flat-content AVIFs (`gray32`, `midgray`, `monochrome` real photo) decode cleanly. Rich content (`testsrc`, `checker`, `red` 4:4:4) is lossy because the AV1 decoder's intra path is ~11 dB PSNR on rich content — not an AVIF-side bug. `bbb_alpha.avif` trips an AV1 `symbol.rs:105` subtract-with-overflow (caught in test). `kimono_rotate90.avif` errors at unsupported `TX 64x18`. Grid items / alpha auxiliary / image transforms all wired at the container level; end-to-end awaits AV1 quality. | — |

</details>

<details>
<summary><strong>Trackers</strong> (decode-only by design) (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **MOD** | ✅ 4-channel Paula-style mixer + main effects | — |
| **STM** (Scream Tracker v1) | 🚧 Structural parse + **playback via shared mixer** (C3-relative `StmC3Pitch` + `StmSampleBody` `SampleSource` impl). Effects wired: Cxx set-volume, Axy volume-slide, Fxx speed/tempo. Hard-pan LRRL. Gaps: tone porta, arpeggio, Exy group, pattern break, vibrato. | — |
| **XM** (FastTracker 2) | ✅ Structural parse + full playback with shared mixer. Envelopes + fadeout + key-off (round 5). **Round 6 effects expansion**: vibrato (4xy/6xy + instrument autovibrato with sweep-ramp), tone portamento (3xy/5xy/Mx with shared memory), pattern jumps (Bxy/Dxy), restart-position, fine porta (E1x/E2x) + extra-fine (X1x/X2x), Exy subcommands (fine vol slide, note cut/delay), Kxy key-off, volume-column slides + tone-porta + vibrato + panning-slide, Axy/1xy/2xy continuous slides with FT2 zero-nibble memory, Fxy speed/BPM. Tests verify vibrato produces spectral sidebands ≥1.5× no-vibrato and tone-porta reaches target period. | — |
| **S3M** | ✅ stereo + SCx/SDx/SBx effects | — |

</details>

<details>
<summary><strong>Protocols, drivers & integrations</strong> (click to expand)</summary>

Not codecs or containers — these are the I/O surfaces and runtime integrations that surround them.

| Component | Role | Status |
|-----------|------|--------|
| **`oxideav-source`** | URI resolution + file reader + prefetching BufferedSource | ✅ `file://` driver; generic `SourceRegistry` for pluggable schemes |
| **`oxideav-http`** | HTTP / HTTPS source driver | ✅ `http://` + `https://` via pure-Rust `ureq` + `rustls` + `webpki-roots`; Range-request seeking |
| **`oxideav-rtmp`** | RTMP ingest + push | ✅ Server accepts incoming publishers (AMF0 handshake, chunk stream demux) + client pushes to remote servers; pluggable key-verification hook |
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
width, height)` which emits `Frame::Video(Rgba)` at the caller-
specified canvas size, one new frame per visible-state change.
Embedded 8×16 bitmap font covers ASCII + Latin-1 supplement; bold via
smear, italic via shear; 4-offset outline. No TrueType dep, no CJK.

In-container subtitles (MKV / MP4 subtitle tracks) remain a scoped
follow-up.

</details>

<details>
<summary><strong>Scaffolds</strong> — API registered, pixel/sample decode not yet implemented (click to expand)</summary>

| Codec | Status |
|-------|--------|
| **JPEG XL** | stub — registered, returns Error::Unsupported on decode/encode |

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

| Scheme | Driver | Notes |
|--------|--------|-------|
| bare path / `file://` | built-in | `std::fs::File` |
| `http://` / `https://` | `oxideav-http` (opt-in) | `ureq` + `rustls`, Range-request seeking |

The HTTP driver is off by default in the library (`http` cargo feature)
and on by default in `oxideplay` and `oxideav-cli`.

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

## CLI

`oxideav` command-line verbs: `list`, `probe`, `remux`, `transcode`,
`run`, `validate`, `dry-run`. Inputs can be local paths or HTTP(S)
URLs.

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
```

`oxideplay --job <file>` runs a job where `@display` / `@out` binds
to the SDL2 player sink; other outputs (file paths) write to disk in
the same run.

## Building

```
cargo build --workspace
cargo test --workspace
```

The `oxideav` binary is produced by the `oxideav-cli` crate:

```
cargo run -p oxideav-cli -- --help
```

### Working with the sub-crates

Every per-format codec — and the aggregator `oxideav` itself — lives in
its own `OxideAV/oxideav{,-*}` repository. To build the workspace you
need all of them cloned into `crates/` — the root `Cargo.toml` globs
`crates/*` as members and points every `[patch.crates-io]` entry at
those local paths. No crates.io round-trip happens for any `oxideav-*`
dep during local dev or CI.

`scripts/clone-crates.sh` does the initial cloning.
`scripts/update-crates.sh` clones any missing ones AND fast-forwards
everything already cloned to the latest upstream tip via a single
GraphQL call. Run either after checking out this repo:

```
gh auth login                 # one-time: gh CLI needs to be authed
./scripts/update-crates.sh    # clone + fast-forward all OxideAV crates
cargo build --workspace
```

Both scripts are safe to re-run. `clone-crates.sh` only clones what's
missing; `update-crates.sh` skips repos whose upstream SHA is already
an ancestor of local HEAD and refuses to fast-forward if local commits
have diverged — your in-progress work is preserved either way.

CI runs `clone-crates.sh` at the top of each job (see
`.github/workflows/ci.yml`), so no crates.io resolution is needed in CI
either — the workspace builds whether or not a given crate has been
published yet.

`.gitignore` hides the cloned crate working copies so `git status` in
this repo only shows changes to the native members (`oxideav-cli`,
`oxideplay`, `oxideav-tests`). Changes inside a cloned crate are
committed against that crate's own repo, not this one.

## License

MIT — see [`LICENSE`](LICENSE). Copyright © 2026 Karpelès Lab Inc.
