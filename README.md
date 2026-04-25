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
| **AAC** | ✅ AAC-LC (mono+stereo, M/S, IMDCT) + HE-AACv1 SBR + HE-AACv2 PS spec-accurate. Round-5 output-scale fix (0.88 → 48.28 dB). **Round 6**: hybrid sub-QMF filterbank (§8.6.4.3) — QMF bands 0/1/2 split via 13-tap FIRs, +4.65 dB on 1 kHz stereo HE-AACv2 (19.26 → 23.91 dB). **Round 7**: IPD/OPD phase correction applied to PS mixing matrix (§8.6.4.6.3.2, Table 8.31 π/4 grid, 3-tap phase smoothing, complex envelope-border interpolation). Activates on streams emitting `enable_ipdopd=1` (afconvert's `aacp` doesn't). HE-AACv1 unchanged at 48.28 dB. **Round 9**: crate-wide clippy sweep — 641 → 0 warnings under `-D warnings` (7k-line `tables_data.rs` `include!`ed in a scoped module with inner `#![allow]`s; manual `write!` → `writeln!`, explicit `.clone_from()`, needless-range-loop rewrites). | ✅ AAC-LC (mono+stereo + PNS + intensity stereo + pulse data) + HE-AACv1 mono encoder. **Round 10**: **HE-AACv1 stereo encoder** — `SbrStereoEncoder` with per-channel QMF + downsampler + scalefactor state on a shared SBR header, wired into a CPE-shaped FIL/SBR payload (Table 4.66, `bs_coupling=0`). ffmpeg-interop accepts the stereo bitstream: L 34.2 dB SNR / R 5.9 dB at 48 kHz. **Round 11**: found the L/R asymmetry — SBR CPE write was emitting `env(L)/noise(L)/env(R)/noise(R)` (the *coupled*-branch layout) instead of the spec's Table 4.66 *independent*-branch order `env(L)/env(R)/noise(L)/noise(R)`. Internal round-trip passed because writer + parser were symmetrically wrong; ffmpeg's spec-compliant decoder was reading R envelope bits as R noise. **R-channel SNR 5.93 → 22.79 dB (+16.9 dB)**. L unchanged at 34.2 dB. **Round 12: HE-AACv2 (Parametric Stereo) encode landed**. New `SbrEncoder::set_emit_ps()` + `write_ps_data_noop()` (35-bit identity payload: IID=0/ICC=1, 10 bands each, no IPD/OPD) + `bs_extended_data` wrapper (`EXTENSION_ID_PS=2`, Table 4.112) + `HeAacV2Encoder` (downmix-to-mono path). ffmpeg 8.1 accepts, decodes at 48 kHz stereo with 24 kHz core. **Identity verified bit-exact**: 47104/47104 (100.00%) samples match L=R, max |L-R|=0 after 4096-sample warm-up. 130/130 tests. Next: real PS analysis (extract IID/ICC from input stereo). |
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
| **iLBC** (RFC 3951) | ✅ Narrowband 20 ms + 30 ms frames, enhanced pitch-emphasis variant | ✅ 20 ms + 30 ms frames (LPC analysis + LSF split-VQ + start-state coding + 3-stage analysis-by-synthesis CB search + bit packer). **Round 11**: opt-in HP pre-processing biquad (RFC 3951 §3.1 / Appendix A.28, 90 Hz cutoff) suppresses DC + 50/60 Hz mains hum. Self-roundtrip 9–12 dB SNR on synthetic voiced; HP filter delivers +2.34 dB on hum-corrupted input. |
| **AC-3** (Dolby Digital) | ✅ Full decode pipeline + FFT IMDCT + §7.8 downmix. Sine 92.02 dB vs ffmpeg. Transient fixture stuck at 15.55 dB — 4 rounds of investigation, not root-caused. **Round 11 hand-traced bndpsd/excite/mask/bap on frame 14 blk 0** against A52-2018 line-by-line — `calc_lowcomp` (round-10 hypothesis) is correct. Now ruled out: IMDCT short-block, coupling, rematrix matrix, dba, calc_lowcomp, mask, bap derivation, mantissa bit-cursor advance, long-block IMDCT, delay carry-over. **Critical new datapoint**: peak ratio our/ref ≈ **1/π = 0.318** across burst blocks (suggests a normalisation constant or sign convention). Round-12 leads (in-tree `INVESTIGATION_R11.md`): coupling-channel mantissa loop ordering vs §5.4.3.61 audblk(); snroffset bit-budget mismatch; phsflg refresh in coords-reused blocks. Three env-gated diagnostic probes (`AC3_TRACE_FRAME / _BLK / _MANT`) committed for the next hunt. **Round 12 hand-trace**: all three round-11 leads ruled out — cpl-channel loop ordering matches §5.4.3.61 exactly; snroffset budget formula matches §7.2.2.7; phsflg refresh per §5.4.3.18 correctly reused. Found unrelated bug (`parse_frame_side_info` double-called `unpack_mantissas` — only affected test paths reading `side.blksw`; live `decode_frame` was always correct, verified via new `AC3_TRACE_BITPOS` probe). Transient PSNR still 15.55 dB after 5 rounds. **Round-13 lead**: FBW `run_bit_allocation` step C (bins 22..132) — only major code-path not yet hand-traced, drives the mantissa-bit budget for the energy-dominant bins via `slowleak.max(...)` reset and `fastleak -= fdecay` accumulation. | ✅ **Round 7**: encoder quality lifted substantially via 4 fixes (group-synced mantissa emitter — biggest; per-block D15 refresh; backward D15 legaliser; accurate bit budget). PSNR our-enc → our-dec: sine 11 → **21 dB**, speech 19 → **32 dB**, stereo L/R 12/6 → **23/45 dB**. ffmpeg decodes all our output with zero errors. Short-block encode / coupling / rematrix pending. |
| **AC-4** (Dolby) | 🚧 Full A-SPX front-end (aspx_config/framing/delta_dir/hfgen_iwc + 18 Huffman codebooks + aspx_ec_data + master-freq-scale + QMF analysis/synthesis, 78-81 dB round-trip). **Round 8**: §5.7.6.4.2 per-envelope HF adjustment (Pseudocodes 76/80-83/90/91/95/106) wired end-to-end into `aspx_extend_pcm` — correctness test verifies output energy ratio follows parsed envelope deltas (e1 > 3×e0 for Δq=2 vs Δq=6). §5.7.6.4.3 noise generator (ASPX_NOISE 512-entry + `generate_qmf_noise`) + §5.7.6.4.4 tone generator (Table 196 SineTable + `(-1)^(sb+sbx)` sign) landed as standalone modules. **Round 9**: noise + tone wired into `aspx_extend_pcm` on top of the adjuster and threaded through cross-frame state (`Q_prev` envelope + QMF-band noise LCG index + tone phase accumulator) so continuity across frames is spec-conformant. FFT probe test asserts HF energy > 2× baseline at 6.2 kHz. **Round 10**: full **A-SPX limiter** (§5.7.6.4.2.2 Pseudocodes 96-101) + §5.7.6.3.1.5 sbg_lim derivation (P72-74) + full P95 branch coverage (sine_area_sb=0/!=0 × tsg_ptr exception). Audited all 18 A-SPX + 11 ASF Huffman codebooks against `docs/audio/ac4/ts_10319001v010401p0-tables.c` — already byte-exact. +19 tests (161→180). **Round 11: complex-covariance TNS landed end-to-end** — all five §5.7.6.4.1 pseudocodes: P85 preflat gains (least-squares 3rd-order poly fit), P86 complex covariance over `Q_low_ext` with `ts_offset_hfadj=4` look-back, P87 α₀/α₁ (`EPSILON_INV=2^-20` slack + `|α|≥4 → 0` fallback), P88 chirp factors via Table 195 `tabNewChirp` with attack/decay smoothing + `<0.015625` zero gate, P89 HF tile TNS with optional pre-flatten. New `aspx_tns.rs` module (~960 LOC) + per-channel `AspxTnsState` threaded into `AspxChannelExtState`. **Round 12: A.5 DRC Huffman codebook + parser landed** — Table A.62 `DRC_HCB` (255 entries, `cb_off=127`, diff range -127..+127) transcribed from `ts_10319001v010401p0-tables.c` lines 859-889 with provenance. New `drc_huffman.rs` + `drc.rs` modules implementing the full §4.2.14.5-10 DRC parser: `parse_drc_frame` (Table 70), `parse_drc_decoder_mode_config` (all three branches), `parse_drc_compression_curve` (Table 73), `parse_drc_data` (Table 74), `parse_drc_gains` (Table 75 with `ref_drc_gain` reset rules). 196→217 tests. Remaining: P92-94 sinusoid location, A.3 ACPL / A.4 DE tables, non-FIXFIX classes, outer metadata() walker. | — |

</details>

<details>
<summary><strong>Video</strong> (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **MJPEG** | ✅ baseline + progressive 4:2:0/4:2:2/4:4:4/grey | ✅ baseline + progressive (SOF2 spectral selection) |
| **FFV1** | ✅ v3: 4:2:0/4:4:4 YUV + RGB via JP2K RCT + `extra_plane` alpha + 9..16-bit RGB (§3.7.2.1 BGR exception) + 10-bit YUV. All three coder_types (0/1/2) + cross-frame state retention for `intra=0` (round 6). ffmpeg interop bit-exact on long-GOP, Golomb 10-bit decode, state-delta `-context 1`. | ✅ v3 range-coded YUV + multi-slice + 10-bit YUV + RGB 8-bit via JP2K RCT. **Round 8 Golomb-Rice encode** (`coder_type=0`) — BitWriter + put_ur/sr_golomb + put_vlc_symbol with adaptive k + run-mode look-ahead emission + slice wrapper. **Round 9**: **10-bit Golomb encode unblocked** (fixture built, up-front gate removed — ffmpeg bit-exactly decodes our 10-bit YUV 4:2:0 output) + **RGB `extra_plane` alpha encode bit-exact** via JP2K RCT with alpha as a 4th Golomb-coded plane. |
| **MPEG-1 video** | ✅ I+P+B frames | ✅ I+P+B frames (half-pel ME, FWD/BWD/BI B-modes, 43 dB PSNR) |
| **MPEG-4 Part 2** | ✅ I + P + B-VOP with 4MV direct mode, half-pel MC, quarter-pel MC in B-VOPs, decode→display reorder. Field-MV decode + field-DCT residual reorder + alt-vertical-scan + field-sample MC for interlaced B-VOPs. **Round 9: 3-round-stuck frame-7 SOLVED via ISO/IEC 14496-2:2004 3rd-edition PDF** — the B-VOP macroblock layer uses Table 6-33 `dbquant` (2-bit fixed code, values {-2,-1,+1,+2}) not the interlaced P-VOP Table 6-32 `dquant` VLC. Our prior implementation parsed the committee-draft P-VOP VLC on B-VOPs, misaligning every subsequent bit in the MB layer. Frame-7 PSNR 22 → 67 dB (bit-exact within rounding) after swapping dispatch. | ✅ I+P-VOP (41-43 dB). **Round 8 B-VOP encoder** — frame buffering + decode-order emit + MODB+MBTYPE+forward/backward MVD syntax. `-bf N` wires via `CodecParameters::options["bf"]`. Self-consistency 38.78 dB; through ffmpeg 25.40 dB overall (I 43.78, P 25-31, B 23-25). **Round 11**: B-MB `cbpb!=0` residual emit now landed — MODB switches `01→00` when any block has residual; emits 6-bit cbpb mask + `dbquant=0` from 2004 Table 6-33 for non-direct modes (direct skips dbquant per §6.2.7); each coded block walks through `write_inter_ac` (Table B-17). **+1.20 dB lift** (39.09 → 40.29 dB avg, B-frames 37.5–38.9 → 40.3–40.5 dB), ffmpeg cross-verifies at 40.28 dB. **Round 12 sweep**: any `DIRECT_BONUS` in `[-1000, +40]` produces identical 40.29 dB (direct SAD wins by hundreds on every B-MB — `-200` was inert). Picked `0` to remove magic-number bias. **4MV-direct emit landed** (new `predict_luma_mb_4mv` + `chroma_mv_for_mb` per §7.5.9.5 + `sad_bidir_4mv`; dispatches when co-located P-MB was 4MV; dormant until P-VOP encoder also emits 4MV). **vti_bits fix**: threaded `vti_resolution = frame_rate.num` through VOP-header writers (was hardcoded `bits_needed(23)=5`); now round-trips at 15fps + 30fps. 119 tests pass. |
| **Theora** | ✅ I+P frames | ✅ I+P frames incl. INTER_MV_FOUR (45 dB PSNR, 3.7× vs all-I) |
| **H.263** | ✅ I+P pictures, half-pel MC, Annex J deblock, Annex D URMV (baseline + **PLUSPTYPE Table D.3** w/ UUI range, round 6), Annex F (4MV + OBMC). 5-frame testsrc QCIF: 60–69 dB vs ffmpeg. Annex E SAC core (819 LOC, round 5); MB-layer VLC→SAC swap deferred. Long-clip ~1 dB/P-frame drift diagnosed as IDCT LSB accumulation (not OBMC-specific). | ✅ I+P pictures, diamond-pattern ME, 46 dB on sliding-gradient. **Round 7**: **Annex F encoder emission** (4MV + OBMC) — 3-pass (decide/emit/reconstruct), SAD-gated 4MV selection, H0/H1/H2 OBMC weights at encode matching decode. **ffmpeg decodes our Annex-F output at 40+ dB** (self-roundtrip 52-54 dB, ours-vs-ffmpeg cross-decode 40-99 dB). |
| **H.261** | ✅ I + P pictures on QCIF / CIF (integer-pel MC + optional loop filter); ffmpeg-PSNR harness: >66 dB intra, >68 dB clean P-chain | ✅ **Round 11**: full Baseline I + P encoder (no MC). `H261Encoder` stateful sequence encoder; self-roundtrip 39.5–39.8 dB on testsrc QCIF; ffmpeg byte-identical decode. **Round 12: integer-pel ME + MC**. New `MTYPE_INTER_MC_CBP` (8-bit) + `MTYPE_INTER_MC_ONLY` (9-bit) per Table 2; `encode_mvd()` paired-representative VLC per Table 3. Full-window SAD ME over ±15 pels with `λ·(|mvx|+|mvy|)` short-MV bias. 4-mode P-MB decision (Skip / Inter / Inter+MC / Inter+MC+CBP+TCOEFF). Spec MVD predictor reset (§4.2.3.4) at MBs 1/12/23, MBA discontinuity, non-MC predecessor. Chroma at half-MV (truncate-toward-zero per §3.2.2). Self-decode + ffmpeg both **39.27 dB byte-tight** on testsrc QCIF (avg P-frame 380 B). 70 tests pass. Next: half-pel MC + loop filter (FIL MTYPEs), rate control. |
| **MS-MPEG-4** (v1 / v2 / v3) | 🚧 Clean-room Implementer handoff per `docs/video/msmpeg4/` (spec/99 + 40 tables). **Round 8**: DC spatial predictor (§7.4.3 L/T/TL gradient, `|A-D|<|A-B|` → left else top) + AC-scan dispatcher + MCBPCY canonical Huffman wired into joint-VLC decode. **Round 9: v3 P-frame decoder landed** — motion-vector VLC + residual macroblock + P-MB CBPY/CBPC plumbing on top of the v3 I-frame pipeline. **Round 10**: full intra-AC primary VLC pipeline (DC pred + AC walk + dequant + IDCT + pel write) wired behind opt-in `AcSelection::Candidate` switch. 64-entry codeword table from `region_05eed0.csv` (VMA `0x1c25fad0`) is byte-exact (Kraft sum = 1) but the `(last, run, level)` role attribution is the implementer's hypothesis (spec/99 §9 OPEN-O6). **Round 11 triangulation**: that VMA is most likely **v2 MCBPCY** (per spec/99 §0.1 row 8), NOT intra-AC. The real intra-AC tables are fused into the 68KB tiered-Huffman walker tree at `0x3df40` rather than published as clean code-length tables, so v3 PSNR remains spec-blocked without further binary RE. Pivoted: shipped v1 + v2 picture-header parsers + v1/v2 MCBPCY decoders. Both MCBPC alphabets extracted directly from `region_053140.hex` LUT (clean-room). 112 → 137 tests. **Round 12: v1/v2 MV decoder landed with notable spec correction**. The docs region was a 4096-byte truncation; full 16384 bytes recovered (SHA-256 verified) into `region_04ed30_full.hex`. Spec/06 §2.3 + spec/07 §3.3 both say "33 entries" — that's **wrong**: bias arithmetic `eax + ecx - 0x20` only yields signed [-32, +32] when raw idx ranges [0, 64] (**65 entries**). Build-time extraction recovers 65 canonical-Huffman triples (Kraft sum 1−4/2^13, 4 sentinel leaves at LUT slot 0). `mv::decode_mv_v1v2(br, predictor)` does two `decode_mvd_v1v2_raw` reads + bias subtract (-32) + predictor add + toroidal wrap. Full PSNR still gated by intra-AC + inter-AC VLC (OPEN-O4). 137 → 159 tests. | — |
| **H.264** | 🚧 **Spec-driven rewrite from scratch** (separate workstream, ~45k LOC). All §7/§8/§9 core layers: NAL/SPS/PPS/slice parsing, FMO (all 7 map types), I/P/B-slice reconstruction (I_PCM + all intra modes + P_Skip/P_L0/P_8x8 + B_Skip/B_Direct/B_L0/L1/Bi), POC types 0/1/2, DPB output ordering + bumping, sliding-window + MMCO (incl. MMCO-5 POC + prevFrameNum reset), RPLM, 6-tap luma + bilinear chroma MC, weighted pred, spatial + temporal direct MV, 4×4/8×8/Hadamard DC/chroma DC transforms, deblocking (§8.7 with spec Table 8-17 tC0), CAVLC + CABAC (engine + binarisations), 7 SEI types. 4:2:0 + 4:2:2 chroma. **16 integration conformance tests**; recent JVT-vector batches land **5/14/14/12 vectors pixel-exact vs ffmpeg reference**. Deferred: 4:4:4 + MBAFF + CABAC I_PCM termination; Annex F SVC / G MVC / H 3D-AVC are long-term phase-4. | 🚧 **Round 1 (clean-room rewrite)**: end-to-end Baseline I_16x16 IDR encoder — SPS+PPS+IDR slice + DC luma + DC chroma + forward 4×4 integer transform + Hadamard DC + encoder-side CAVLC + local reconstruction loop. Self-roundtrip 38.85 dB Y at QP 26 on 64×64 testsrc; **ffmpeg interop bit-exact** vs encoder's local recon. **Round 2** (round 11 of the workspace): all 4 I_16x16 modes (V/H/DC/Plane) with SAD-driven decision (§8.3.3) + all 4 Intra_Chroma modes by joint Cb+Cr SAD (§8.3.4) + chroma residual transmit. 64×64 diagonal at QP 26: 38.85 → 42.25 dB Y (+3.4 dB) AND stream got smaller (148 → 88 bytes). **Round 3: luma AC residual landed** (`cbp_luma=15` Intra_16x16). Encoder emits 16 `Intra16x16ACLevel` CAVLC blocks in §6.4.3 raster-Z order with shared per-picture `CavlcNcGrid` + decoder's `derive_nc_luma` path. Smooth content unchanged at 42.25/50.11 dB (predictor + DC already perfect — AC quantises to zero at QP=26). **Noisy 64×64 (high-frequency hash)**: ~14 → **37.63 dB Y at QP=26** (46.36 dB at QP=18). AC unlocks the texture detail predictors can't capture. ffmpeg interop still bit-exact (≤1 LSB) including the 3089-byte noisy stream. 758 lib + 7 integration tests. Next: I_NxN with 9 modes (biggest remaining lift on smooth content), then deblock. |
| **H.265 (HEVC)** | ✅ I/P/B slice decode, 8-bit 4:2:0 + SAO + deblock. Main 10 intra bit-exact. **Round 7**: §8.5.3.2.8 TMVP BR/center 16×16 snap + same-CTB-row gate (24.19 → 24.77 dB). **Round 9: AMVP POC scaling** (§8.5.3.2.7 `tb / td` distance ratio when reference POCs differ — applied to both spatial and temporal MV predictors), avg 24.77 → 25.54 dB, frame-3 21 → 22.3 dB. HEIF/HEIC decode (opt-in `heif` feature). **Round 10**: scaling-lists path was actually wired all along but documented as "rejected" — landed two byte-exact ffmpeg-match tests (intra + inter `scaling-list=default` 64×64 IDR per §7.4.5 / §8.6.3 eq. 8-309). `interSplitFlag` investigation traced to a CABAC desync several CUs past the parse point — not yet root-caused. **Round 11: 4:2:2 (`chroma_format_idc=2`) intra decode lands** — byte-exact (∞ dB) ffmpeg match on three single-CTU 64×64 fixtures (testsrc/gray/rgbtestsrc); 31 dB on 128×64 cross-CTU. Wires: §6.2 Table 6-1 helpers; §7.3.8.8 stacked-vertical chroma TBs (cbf_cb/cr expanded to `[u32;2]`); §7.3.8.10/11 chroma TB placement `(x/SubWidthC, y/SubHeightC)`; §8.4.3 Table 8-3 chroma intra mode remap; §8.6.1 QpC `min(qPi, 51)`; §8.7.2 chroma deblock parameterized. Decoder emits `Yuv422P` / `Yuv422P10Le`. 33 → 38 tests. **Round 12: 4:2:2 P/B inter decode landed**. §8.5.3.2.10 chroma MV derivation (`mvCLX[1] = mvLX[1] * 2 / SubHeightC`) inside `chroma_mc` / `chroma_mc_hp`. `(SubWidthC, SubHeightC)` plumbed through `motion_compensate_pb` + `RefPicture`. Bi-prediction worked unchanged once plumbing was correct. PSNR vs ffmpeg: P-slice gray ∞ dB; B-slice gray ∞ dB; textured testsrc P 20.17 dB / total 23.18 dB. 38 → 40 integration + 85 unit tests. Gaps: Main 12 (12-bit), `interSplitFlag` desync, AMP, 4:4:4, tiles+WPP. | ✅ Baseline CAVLC I+P + **Main-profile CABAC P-slice encoder** (round 7) — integer-pel MVD, 2Nx2N only, DCT+flat-quant, local reconstruction matches our decoder pixel-exact. I 45 dB / P 31 dB via our decoder; ffmpeg accepts with zero errors at 26.82 dB. |
| **H.266 (VVC)** | 🚧 Full VVC front-end + CTU walker scaffold + leaf-CU syntax reader + §8.4.2 MPM list + §8.4.3 chroma derivation. **Round 8**: CBF reads (Table 127 ctxInc) + `cu_qp_delta` + `cu_chroma_qp_offset` + last-sig-coeff position + sub-block residual walker (reverse scan, `csbf_ctx_regular` neighbour rule, full per-coeff flag chain). **Round 9**: spec-exact `locNumSig`/`locSumAbsPass1`/`remBinsPass1` ctxInc threading per §9.3.4.2.8 (neighbourhood `(xC+1,yC)` + `(xC,yC+1)` scan with `locSumAbs` clamp, 12 ctx slots for sig/gt1/par/gt3 with luma/chroma split) + **§8.7.3 dequant scaling-list apply** (per-block `rectNonTsFlag` + `bdShift` + `levelScale[rect][(qP+1)&3]` + user-scaling-list lookup with DC override). 280+ tests. **Round 10: `reconstruct_leaf_cu` lands — first actual VVC pixel output**. Wires §8.4.5.2.8 reference-sample fetch + §8.4.5.2.11/.12/.13 PLANAR/DC + cardinal angular intra (modes 2/18/34/50/66 — non-cardinal snaps to nearest cardinal as fallback) + §8.7.3 flat dequant + §8.7.4.1 separable inverse 2D DCT-II + §8.7.5.1 reconstruct+clip (eq. 1426). New `PicturePlane`/`PictureBuffer` (4:2:0 frame buffer) + `decode_picture_into` walker. Synthetic 32×32 / 64×64 IDR pictures round-trip cleanly for luma (chroma still mid-grey). **Round 11: chroma reconstruction lands — first color VVC frames**. Cb/Cr now flow through the same §8.4 + §8.7 pipeline at half-resolution for 4:2:0. New `chroma_pred_mode_for_predict` (§8.4.3, CCLM modes 81–83 collapse to PLANAR — deferred). §8.7.1 chroma QP identity mapping. §8.7.4.2 size-64 DCT-II enabled via existing antisymmetry reflections. 275 lib + 4 integration tests. **Round 12: §8.8.3 in-loop deblock landed (short-tap)**. New ~1031-LOC `deblock.rs` with Table 43 β'/tC' lookup (eqs. 1276/1278/1279/1345/1347/1348 with bit-depth scaling), boundary-strength derivation (intra→2, transform-coded→1, BDPCM-both→0), §8.8.3.6.2 luma decision (dE = 0/1/2 based on dpq + sp + sq + spq) + dEp/dEq for p1/q1, §8.8.3.6.7 short luma filter (strong eqs. 1375–1380 + weak eqs. 1381–1388), §8.8.3.6.10 chroma weak filter (eqs. 1421–1423), V-then-H pass per §8.8.3.1 ordering, 4×4 luma grid CU lookup. CtuWalker accumulates `DeblockCu` records during reconstruct; `apply_in_loop_filters(out)` consumes them. Verified seam smoothing on a 98↔102 stripe across two intra CUs at QP=32. 282 lib + 6 integration tests. Next: long luma filters (5/7-tap), chroma strong filter, SAO, ALF, CCLM, 2×2/2×4/4×2 chroma transforms. | ✅ **Round 8 encoder scaffold** — forward-bitstream emitter (VPS/SPS/PPS/PH/IDR) that parse-roundtrips through the decoder's front-end byte-for-byte. 8-bit 4:2:0, CTB=128, all tool flags off, empty coded-slice payload. PH emitted as standalone PH_NUT NAL. No residual / CABAC / pixel output yet. |
| **VP6** | ✅ Full FLV playback (845/845 frames of sample decode cleanly; range coder + MB-types + IDCT + MC + loop filter + vp6a alpha). **Round 9: latent axis-transpose fixed** — `IDCT_SCANTABLE` was applying `(v>>3)|((v&7)<<3)` to `ZIGZAG_DIRECT`, swapping row/col; the round-8 encoder compensated with its own transpose in `forward_dct8x8` so self-roundtrip worked by accident while ffmpeg-decoded output was correctly oriented but self-decoded output was transposed. Both paths now match spec orientation, self-decode matches ffmpeg decode. | ✅ VP6F keyframe encoder with **round-8 AC coefficient emission** — forward 2D DCT-II + AC quantisation + zig-zag scan + run-length + bool-coded VP6_PCR_TREE walker + coefficient-model state mirroring decoder. Gradient content lifted **+10-14 dB vs DC-only** (Y 36-40 dB at QP 4-32 through ffmpeg decode). **Round 9**: P-frame skip encoder (skip-run signalling at MB layer; flat-to-flat 63 dB). MV encode / loop-filter emission / Huffman path still pending. |
| **VP8** | ✅ I+P frames (6-tap sub-pel + MV decode + ref management) | ✅ I + P frames, all 5 intra modes + SPLIT_MV + loop filter (42-51 dB PSNR) |
| **VP9** | 🚧 Keyframe + inter + segmentation + bit-accurate §6.5 MV list + inter-mode ctx + full §6.3 compressed-header probs + §8.10 saved slots + `AboveNonzeroCtx` / `LeftNonzeroCtx` tracking. **Round 6**: fixed spec-correct `checkEob` reset after ZERO_TOKEN per §6.4.24. **Round 7**: ruled out 4 hypotheses (PARETO8 token-tree, intra vs inter dequant scales, CAT1-6 magnitude paths, skip_ctx hardcoded `bd.read(192)`). PSNR stuck at 10.96 dB across 3 rounds. **Round 10: 3-round-stuck PSNR root-caused** via bit-level boolean-decoder trace — §9.2.1 `init_bool()` was missing the post-f(8) marker read (§9.2.2 must be zero) that the spec mandates. A since-falsified code comment claimed VP9 had dropped the VP8-era marker; it had not. Fix: add the marker read at p=128 to both `BoolDecoder::new()` and `BoolEncoder::new()`. Per-frame Y std went from 1.7-2.6 → **8.5-15.4** (reference std=71.3) — output is now data-shaped instead of noise-floor flat. Compound PSNR nominally 10.96 → 10.79 dB but the marker fix exposed downstream coefficient-decode bugs. Also fixed `INV_MAP_TABLE` size (254 → 255 per spec). **Round 11: lossless keyframes now decode essentially bit-exact** — root cause was §9.3.2 partition context: both `block::read_partition` and `encoder::tile::PartitionCtx::lookup` were inverting `bsl` (using `tbl_bsl = 3 - bsl`), pointing 64×64 reads at the 8×8 row of the §10.4 table. Plus three additional fixes (lossless WHT dispatch, spec-correct inverse WHT, dequant i16 clamp). **Lossless gray fixture: ~25 → 66.77 dB**. Compound keyframe luma std 1.7 → 71.8 (ref 71.3). Compound *inter* frames regressed to 8.01 dB because the partition fix changed which mode/skip contexts are decoded. **Round 12: §9.3.2 inter-ref contexts implemented end-to-end** — `comp_mode_ctx` (5-ctx XOR/CompFixedRef tree), `comp_ref_ctx`, `single_ref_p1_ctx` (LAST_FRAME equality), `single_ref_p2_ctx` (GOLDEN_FRAME equality), `interp_filter_ctx` (4-ctx, sentinel 3 for intra/unavail). New `NeighbourInfo` (mirrors §6.4.11 LeftRefFrame/AboveRefFrame/Single/Intra) + `InterpFilters[r][c]` slot on `InterMiCell`. **Inter parser now byte-aligned to keyframe** (frame 1 std 35 → 71.8 matching ref). **Lossless inter clips: 66.77 dB** matching lossless keyframe. Compound 8.01 → 7.93 dB but the bottleneck is now shared keyframe (7.36 dB) + inter — inter is no longer the limiter. 144 → 154 tests. Next: lossy keyframe residual coef parsing (§6.4.21–6.4.26). | — |
| **AV1** | 🚧 OBU parse + range coder/CDFs + coeff decode + partition + transforms + all intra predictors w/ edge filter + §7.15 CDEF + Loop Restoration (Wiener+SGR) + inter MC (NEWMV/GLOBALMV) + ref-scaling. Round-5 chroma fix: gray U/V 12 → 48/45 dB. Rounds 6-7: read_block_tx_size + read_skip ordering + delta_q + delta_lf + use_intrabc + filter_intra_mode_info. **Round 8**: `palette_mode_info()` + inter `read_skip` reordered BEFORE `is_inter` + `read_var_tx_size()` recursive helper with `DEFAULT_TXFM_SPLIT_CDF`. Gray P-frame Y 39.34 → 43.10 dB. **Round 9**: **filter-intra predictor dispatch** (§7.11.2.3 — 5 filter modes × 8 intra edge combinations, 4×4 block unit, wired to `predict_intra`) + **per-block `CurrentQIndex` / `DeltaLF[]` apply** (§5.9.26 delta-q base qindex adjustment + §5.9.27 per-filter delta-lf propagated into `LoopFilterParams.loop_filter_level[]` when the frame header enables them). **Round 10: palette mode end-to-end** — §5.11.46 `palette_mode_info()` with `get_palette_cache` + cache-reference colour pickup + delta-encoded remainders + V-plane signed-delta path; §5.11.49 `palette_tokens()` anti-diagonal wavefront + per-pixel index decode; §5.11.50 `get_palette_color_context` neighbour scoring + bubble sort + multiplier hash; §7.11.4 `predict_palette` per-plane `palette[map[y][x]]` replacement. Bonus: discovered the existing palette default CDFs were stored in spec-cumulative form (`P(X≤i)`) but the local symbol decoder expects survival form (`P(X>i) = 32768 - cdf`); inverted all 13 palette CDFs. The previously-bailing `palette_screen.ivf` fixture now decodes (10.37 dB Y vs libaom — capped by var-tx residual-per-TU still pending). **Round 11**: audited non-palette default CDFs for the same survival-form bug — 9 hand-coded CDFs / 47 entries fixed. Surfaced a latent u8 overflow in `loopfilter::narrow::narrow_mask`. Implemented §5.11.36 `transform_tree` walker for inter luma residual. **Round 12: §5.11.10 `read_skip_mode` + §5.11.19 `inter_segment_id`** end-to-end with canonical §5.11.18 ordering (preSkip seg-id → skip_mode → skip → post-skip seg-id → delta_q/lf → is_inter → mode info). New `DEFAULT_SEGMENT_ID_PREDICTED_CDF` (3 ctx, uniform 16384 per §9.4); `DEFAULT_SKIP_MODE_CDF` wired through TileDecoder. Helpers: `seg_feature_active`, `decode_skip_mode`, `decode_seg_id_predicted`, `inter_segment_id`, `read_skip_mode_for_block`, `predicted_segment_id_from_prev` (§5.11.21 spatial-min over prev_frame). `MiInfo.skip_mode` + `InterBlockInfo.skip_mode/segment_id` for §9.4 ctx tracking. **`read_skip` ctx now sums above/left `Skips[][]` neighbours** (was hardcoded 0). Inter leaf gained spec-mandated `read_delta_qindex`/`read_delta_lf` calls. SEG_LVL_SKIP/GLOBALMV constants added. Bonus: BitReader `ns(1)` underflow fix (was crashing on SVT-AV1 fixtures). PSNR unchanged (fixtures have `skip_mode_present=false` + `segmentation.enabled=false`) but plumbing is bitstream-correct and ready. 317 → 323 tests. Next: derive `skipModeAllowed` from DPB's OrderHint trail. | — |
| **Dirac / VC-2** | ✅ VC-2 LD + HQ intra end-to-end + Dirac core-syntax intra (VLC and AC paths) + core-syntax inter + OBMC motion comp + full IDWT (7 wavelets) + arithmetic coder + 10/12-bit output + frame-rate-aware timebase + pts passthrough. ffmpeg-interop tests: 8-bit 4:2:2 + 8-bit 4:4:4 + 10-bit 4:2:0. Gaps: VC-2 v3 asymmetric transforms (SMPTE ST 2042-1 ed-2 not in docs/), `Yuv422P12Le` / `Yuv444P12Le` variants not yet in oxideav-core. Round-8: `mean3` switched from `/3` to `div_euclid(3)` (floor, per §1.3). | ✅ HQ + LD intra encoders. **Round 8**: HQ bit-exact through ffmpeg decode (∞ dB). **Round 9: 3-round-stuck LD magnitude bug SOLVED** — `slice_y_length` field was 1 bit too narrow on power-of-two `slice_bytes` values because Dirac's `intlog2` convention (⌈log2(n+1)⌉) differs from ffmpeg's bit-width reader (⌈log2(n)⌉ for n>1). Widening `slice_y_length` + `slice_c_length` to the Dirac convention lifted LD testsrc PSNR **14.27 → 49.63 dB** through ffmpeg decode; ffmpeg-interop gate now enforced at ≥48 dB. Multi-picture emitters retained. **Round 10: comprehensive regression matrix** — all 7 wavelet filters (Tables 15.1-15.7) round-trip bit-exactly across depths 1-3 + 40×24 non-square; 18-case HQ q=0 lossless matrix (6 wavelets × 3 chromas) + non-square dims + 8×8 minimum picture + depth=2 + 3-frame multi-stream + q=12 PSNR gate + Fidelity probe + LD across two wavelets; 3 new ffmpeg-interop tests (HQ q=0 4:2:2 + 4:4:4 + per-wavelet matrix). 119 → 132 tests. No bugs found — every wavelet achieves ∞ PSNR through ffmpeg on a smooth gradient. |
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
| **JPEG 2000** | ✅ **Bit-exact vs OpenJPEG on all test fixtures (round 9).** Part-1 baseline + multi-tile (§B.3) + MQ + EBCOT + 5/3 + 9/7 IDWT + tier-2 + LRCP / RLCP + JP2 wrapper. Two real bug fixes rounds 3+4 (MQ `nlps`/`nmps` swap; T1 `pi`-flag §D.3.4). Round 7 axis-order fix per T.800 §F.3.2/§F.4.2 (VER→HOR forward, HOR→VER inverse). **Round 9: HH divergence at event #185 SOLVED** — `ctxno_zc` pre-clamped `d = d.min(2)` before the HH Table D.1 match, collapsing labels 6/7/8 whenever a coefficient had 3 or 4 significant diagonal neighbours. **Round 11: multi-layer (§B.10) + RPCL progression (§B.12.1.3)** both landed bit-exact. Smart find: the per-code-block accumulator was already correct; only blocker was a defensive `cod.num_layers != 1` short-circuit. 6 new OPJ-interop fixtures, all bit-exact. 77 tests. **Round 12: user precincts + PCRL + CPRL all landed**. New `ResolutionLayout` model carries flat raster-ordered `Vec<Precinct>` per §B.6, each holding per-sub-band tag-trees and `(cx0, cy0, pcw, pch)` slices into the sub-band's global code-block grid. Code-block clamping per §B.7 (`xcb' = min(xcb, PPx − [r==0?0:1])`). Position-driven walkers compute precinct reference-grid origins (LL_r coords scaled by `2^(NL−r) × XRsiz/YRsiz`) and sort by `(ref_y, ref_x, ...)` per §B.12.1.3, skipping empty cells. **16 new precinct tests bit-exact vs `opj_decompress`** for 64×64 gray + 128×128 multi-tile gray + multi-layer 9/7 across all 5 progression orders (LRCP/RPCL/PCRL/CPRL) + 128×128 RGB cross-progression checks. 77 → 93 tests. Next: POC marker (§A.6.6), PPM/PPT packed packet headers, OPJ RGB MCT rounding. | ✅ 5/3 lossless + 9/7 irreversible RGB (forward RCT/ICT; JP2 box wrapper) |
| **JPEG XL** | 🚧 Signature + SizeHeader + partial ImageMetadata parse — Modular (MA-tree) and VarDCT pixel decode pipelines pending | — |
| **AVIF** | 🚧 **End-to-end decode now works** (round 5): HEIF box walker → AV1 OBU handoff → decoded `VideoFrame`. Flat-content AVIFs (`gray32`, `midgray`, `monochrome` real photo) decode cleanly. Rich content (`testsrc`, `checker`, `red` 4:4:4) is lossy because the AV1 decoder's intra path is ~11 dB PSNR on rich content — not an AVIF-side bug. `bbb_alpha.avif` trips an AV1 `symbol.rs:105` subtract-with-overflow (caught in test). `kimono_rotate90.avif` errors at unsupported `TX 64x18`. Grid items / alpha auxiliary / image transforms all wired at the container level; end-to-end awaits AV1 quality. | — |

</details>

<details>
<summary><strong>Trackers</strong> (decode-only by design) (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **MOD** | ✅ 4-channel Paula-style mixer + full ProTracker 1.1B effect set (arpeggio, 1/2/3xy porta up/down/tone, 4xy/6xy vibrato, 5xy tone-porta+volslide, 7xy tremolo, 9xx sample offset, Axy volslide, Bxx/Dxy jump/break, Cxx/Fxx set-vol/speed, every Exy sub-command: E1/E2 fine porta, E3 gliss, E4/E7 WF select, E5 finetune, E6 pattern loop, E9 retrig, EA/EB fine volslide, EC cut, ED delay, EE pattern delay). 16×36 Protracker period table + 32-entry Protracker sine table. Forward-only loops per MOD spec. | — |
| **STM** (Scream Tracker v1) | ✅ Structural parse + shared-mixer playback. **Round 9**: effect coverage brought to XM parity — tone portamento (Gxy with shared memory), arpeggio (Jxy), pattern jumps (Bxy/Cxy), Exy group (E1x/E2x fine porta, ED note-delay, EC note-cut), vibrato (Hxy + vibrato memory), volume-slide variants. Hard-pan LRRL retained. | — |
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

When the **winit + wgpu** video output is selected (`--vo winit`),
`oxideplay` ships an **egui on-screen overlay UI** (auto-hide after
~3 s of mouse idle during playback; stays visible while paused).
Mouse-driven controls cover play/pause, draggable seek bar, time
display, volume slider, mute, ±10 s skip, and a toggleable stats
panel. egui (0.34) + egui-wgpu + egui-winit are pure-Rust deps gated
behind the `winit` cargo feature, so SDL2 builds are unaffected.

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

> **First clone? Run `./scripts/clone-crates.sh` before `cargo build`.**
> The workspace tracks only the aggregator glue (`oxideav-cli`,
> `oxideplay`, `oxideav-tests`); every per-format codec lives in its
> own `OxideAV/oxideav{,-*}` GitHub repo and must be cloned into
> `crates/` first. `cargo build` on a bare checkout fails with
> `failed to load manifest for workspace member` until you do.

```
git clone https://github.com/OxideAV/oxideav-workspace.git
cd oxideav-workspace

gh auth login                 # one-time: clone-crates.sh uses gh API to list siblings
./scripts/clone-crates.sh     # populates crates/ with every OxideAV/oxideav{,-*} repo

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

- `scripts/clone-crates.sh` — clones every missing OxideAV sibling. Idempotent; safe to re-run.
- `scripts/update-crates.sh` — clones the missing ones AND fast-forwards already-cloned siblings to upstream tip via a single GraphQL call. Skips siblings whose upstream is already an ancestor of local HEAD and refuses to fast-forward when local commits have diverged, so in-progress work is preserved.

```
./scripts/update-crates.sh    # clone + fast-forward all OxideAV crates
```

CI runs `clone-crates.sh` at the top of each job (see
`.github/workflows/ci.yml`), so no crates.io resolution is needed there
either — the workspace builds whether or not a given crate has been
published yet.

`.gitignore` hides the cloned crate working copies so `git status` in
this repo only shows changes to the native members (`oxideav-cli`,
`oxideplay`, `oxideav-tests`). Changes inside a cloned crate are
committed against that crate's own repo, not this one.

## License

MIT — see [`LICENSE`](LICENSE). Copyright © 2026 Karpelès Lab Inc.
