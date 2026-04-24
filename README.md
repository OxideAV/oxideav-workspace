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
  Echo / Resample / Spectrogram), `oxideav-pixfmt` (pixel-format conversion
  matrix + palette generation + dither).
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
| **AAC** | ✅ AAC-LC (mono+stereo, M/S, IMDCT) + **HE-AACv1** (SBR: full bitstream + 64-band QMF analysis/synthesis + HF gen + HF adjust) + **HE-AACv2 Parametric Stereo spec-accurate** (QMF-domain upmix + full Huffman param decode + 3-link allpass decorrelator + §8.6.4.6.2 mixing matrix Ra/Rb with time-interpolation). Interop verified vs afconvert / libfdk_aac. Gaps: hybrid sub-QMF filterbank (low-freq stereo detail), IPD/OPD parsed but not applied, 34-band native resolution. | ✅ AAC-LC (mono+stereo + PNS + intensity stereo + pulse data) + HE-AACv1 mono encoder |
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
| **AC-3** (Dolby Digital) | ✅ Full decode pipeline + FFT-backed IMDCT (§7.9.4) + §7.8 downmix (LoRo, all 8 acmods). **PSNR vs ffmpeg on sine fixture: 92.02 dB** after fixing 3 transcription bugs in tables.rs (BAPTAB Table 7.16 range boundaries, MASKTAB Table 7.13 four rows, LATAB[151]). Transient fixture 15 dB global — state-carryover bug across block-switch boundaries (coupling / rematrix / exponent-strategy state not restored correctly) is round-5 target. | — |
| **AC-4** (Dolby) | 🚧 Sync / TOC / presentation / substream parse (ETSI 103 190-1) + `aspx_config` + `companding_control` + `aspx_framing` **wired into ASF walker** + `aspx_delta_dir` (Table 54) + `aspx_hfgen_iwc_1ch`/`2ch` (Tables 55/56) + `AspxHcb` Huffman scaffold with 18 Annex A.2 codebook metas. Stub decoder emits silence. **Blocked on docs**: codeword arrays live in `ts_103190_tables.c` inside the ETSI accompaniment ZIP, not in `docs/audio/ac4/`. | — |

</details>

<details>
<summary><strong>Video</strong> (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **MJPEG** | ✅ baseline + progressive 4:2:0/4:2:2/4:4:4/grey | ✅ baseline + progressive (SOF2 spectral selection) |
| **FFV1** | ✅ v3: 4:2:0 / 4:4:4 YUV + RGB via JP2K RCT + **`extra_plane` alpha** (`Yuva420P` + `Rgba` / `Rgba64Le`) + **9..16-bit RGB with RFC §3.7.2.1 BGR exception** (`Rgb48Le`) + **10-bit YUV** (`Yuv420P10Le` / `Yuv422P10Le` / `Yuv444P10Le`, bit-exact vs ffmpeg). Range-coded `coder_type=1` + custom state-transition delta `coder_type=2` + Golomb-Rice `coder_type=0`. Per-plane `qt_idx` selection from slice header. Multi-slice decode OK. Gaps: multi-slice encode, >8-bit encode, RGB encode, cross-frame state retention for `intra=0`, `initial_state_delta`, Golomb-Rice for >8-bit or with alpha. | ✅ v3 range-coded YUV (encoder still YUV-only) |
| **MPEG-1 video** | ✅ I+P+B frames | ✅ I+P+B frames (half-pel ME, FWD/BWD/BI B-modes, 43 dB PSNR) |
| **MPEG-4 Part 2** | ✅ I + P + B-VOP with 4MV direct mode, half-pel MC, quarter-pel MC in B-VOPs, decode→display reorder. **VLC desync fixed**: B-VOPs whose backward ref is an I-VOP (no MV grid) now default `co_located_not_coded` to true per encoder eliding convention. Full 12-frame test fixture now decodes (was 5/12). Interlaced B-MB parser invoked (bit reader stays aligned on `-flags +ilme+ildct` fixtures); field-based MC not yet applied. Overall floor 28-30 dB; frame-7 outlier at 22 dB is residual AC-decode misalignment (round-5 target). | ✅ I+P-VOP (41-43 dB PSNR, 21% vs all-I) |
| **Theora** | ✅ I+P frames | ✅ I+P frames incl. INTER_MV_FOUR (45 dB PSNR, 3.7× vs all-I) |
| **H.263** | ✅ I+P pictures, half-pel MC, Annex J deblock, Annex D URMV, **Annex F Advanced Prediction (4MV + OBMC)** — §F.2 per-block median MVs + §F.3 OBMC H0/H1/H2 weight matrices + §F.2 MVDCHR with Table F.1 sixteenth-pel → half-pel rounding. 5-frame testsrc QCIF: 60.86–69.16 dB vs ffmpeg. 15-frame clip drifts ~1 dB/P-frame (30 dB at 14-deep P-chain). Gaps: Annex D PLUSPTYPE form, Annex E SAC, Annex G PB-frames, Annex I/K/N/P/Q/R/S/T. | ✅ I+P pictures, diamond-pattern motion search (±15 pel range), 46 dB PSNR on sliding-gradient |
| **H.261** | ✅ I + P pictures on QCIF / CIF (integer-pel MC + optional loop filter); ffmpeg-PSNR harness: >66 dB intra, >68 dB clean P-chain | — |
| **MS-MPEG-4** (v1 / v2 / v3) | 🚧 Picture-header framing (DIV3/MP43/…) + 6-block intra MB walker (`decode_intra_mb`) wired — but intra AC VLC table is an `Unsupported` placeholder because `docs/video/msmpeg4/` has not yet extracted `0x5eed0` / `0x5eac8` as `(symbol, bit_length)` pairs. See `SPEC_BLOCKED`-style placeholder in `src/ac.rs`. | — |
| **H.264** | Full CABAC+CAVLC I/P/B slice decode (real-world MKV playback) | ✅ Baseline CAVLC (I+P, 49.9 dB) + Main-profile CABAC IDR (I-only, 41.6 dB) |
| **H.265 (HEVC)** | ✅ I / P / B slice decode, 8-bit 4:2:0 + SAO + deblock. **Main 10 intra is bit-exact vs ffmpeg**; Main 10 inter at 20.73 dB with first P-frame at 38.6 dB (drift compounds through DPB). Fixed two bit-depth QP bugs — `get_qp` now returns primed `Qp'Y = QpY + QpBdOffsetY` per §8.6.3 eq. 8-309; CuQpDelta wrap widens modulus to `52 + QpBdOffsetY` per eq. 8-283; `luma_mc_bi_combine` shift made bit-depth-dependent (`shift2 = Max(3, 15 - BitDepth)`). **HEIF/HEIC still-image decode** via opt-in `heif` feature. Gaps: 12-bit, 4:2:2 / 4:4:4, AMP / long-term refs / scaling lists / tiles+WPP; Main 10 inter drift bisection (prediction-before-residual clip order / 4×4 `trType` / TMVP collocated MV scaling); HEIF gaps: grid items, multi-extent iloc, alpha aux, transforms, image sequences. | — |
| **H.266 (VVC)** | 🚧 Full VVC front-end: NAL framing + VPS + **SPS full tail** (dpb, partition, ~60 tool flags, ref_pic_list_struct, subpic, VUI, sps_extension) + **HRD timing** (§7.3.5) + **PPS full tail** (§7.3.2.5: cabac-init, weighted-pred, wraparound, deblocking control, chroma tool offsets, 3 extension flags) + **stateful slice header** (§7.3.7, takes SPS + PPS + PhState) + **RefPicList construction** (§8.3.2, STRP pocBase chaining + LT msb-cycle + ILRP AU-sharing) + DCI + OPI + APS. Still pending: picture-header tail, `pps_no_pic_partition_flag=0` streams (per-pic tile/slice), VUI payload (H.274), slice-header tail past deblocking, `sps_range_extension()`, `ref_pic_lists()` in-header parsing. CTU walker / reconstruction not yet started. | — |
| **VP6** | ✅ Full FLV playback (845/845 frames of sample decode cleanly; range coder + MB-types + IDCT + MC + loop filter + vp6a alpha) | — |
| **VP8** | ✅ I+P frames (6-tap sub-pel + MV decode + ref management) | ✅ I + P frames, all 5 intra modes + SPLIT_MV + loop filter (42-51 dB PSNR) |
| **VP9** | 🚧 Keyframe + inter (single + compound ref, scaled refs, 8-tap MC, DCT/ADST 4/8/16/32) + per-block segmentation + bit-accurate MV-candidate list (§6.5) + inter-mode ctx derivation + **full §6.3 compressed-header probability-update decode** (all 18 sub-sections: tx_mode_probs, coef_probs, skip, is_inter, inter_mode, interp_filter, reference_mode, y_mode, partition, mv_probs + update_mv_prob + diff_update_prob + subexp delta) + **§8.10 saved slots across frames** (4 slots, keyframe/intra_only/reset resets). PSNR vs ffmpeg still stuck at 10.94 dB — round-4 agent proved §6.3 wasn't the dominant blocker; real bottleneck is **missing `AboveNonzeroContext` / `LeftNonzeroContext` tracking** in `decode_coefs` (every transform block starts from `initial_ctx = 0` instead of spec's above-left derived context). §8.4 `adapt_probs` backward adaptation + `UsePrevFrameMvs` + 10/12-bit + 4:2:2/4:4:4 also pending. | — |
| **AV1** | 🚧 OBU + sequence / tile parse, range coder + CDFs, coefficient decode + partition quadtree + transforms, all intra predictors with edge filter + upsample + **full §7.15 CDEF** + Loop Restoration (Wiener + SGR). **Inter MC wired**: NEWMV translational + GLOBALMV (identity + translation) + per-block interp filter + reference-scaling scaffold (identity short-circuits, ½× / ¼× project correctly). **Luma inter on flat-gray fixture within 2 dB of intra** (P-frame 37.95 vs K-frame 39.97 dB). Chroma edge carryover bug + testsrc intra ~11 dB are upstream bottlenecks that cap inter PSNR. Ref-MV list for NEAREST/NEARMV, warped MC, full compound, OBMC, film grain activation, HBD + chroma ref-scaling still pending. | — |
| **Dirac / VC-2** | ✅ VC-2 LD + HQ intra end-to-end + Dirac core-syntax intra (VLC and AC paths) + core-syntax inter + OBMC motion comp + full IDWT (7 wavelets) + arithmetic coder + 10/12-bit output + frame-rate-aware timebase + pts passthrough. ffmpeg-interop tests: 8-bit 4:2:2 + 8-bit 4:4:4 + 10-bit 4:2:0. Gaps: VC-2 v3 asymmetric transforms (SMPTE ST 2042-1 ed-2 not in docs/), `Yuv422P12Le` / `Yuv444P12Le` variants not yet in oxideav-core. | — |
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
| **JPEG 2000** | 🚧 Part-1 baseline + multi-tile (§B.3) + MQ + EBCOT + 5/3 + 9/7 IDWT + tier-2 + LRCP / RLCP + JP2 wrapper. **Two real bug fixes** across rounds 3+4: (1) MQ state-table `nlps`/`nmps` swapped vs T.800 Annex C Table C.2 (round 3), (2) T1 `pi`-flag: sigprop-probed samples need `pi[idx]=true` regardless of outcome per §D.3.4 — stray cleanup bit was drifting the MQ decoder (round 4). Interop PSNR: **spike4 bit-exact (un-ignored)**, opj16_l1 35.09 dB, opj32 29.79 dB. Remaining residual on opj16_l1 is a ±1 LSB divergence in LL sub-band values — likely a sign-direction / midpoint-deposit convention in magnitude refinement. Multi-layer + user precinct grids + CPRL / PCRL / RPCL + Part-2 still pending. | ✅ 5/3 lossless + 9/7 irreversible RGB (forward RCT/ICT; JP2 box wrapper) |
| **JPEG XL** | 🚧 Signature + SizeHeader + partial ImageMetadata parse — Modular (MA-tree) and VarDCT pixel decode pipelines pending | — |
| **AVIF** | 🚧 HEIF container parsed + `av1C` / `ispe` / `colr` / `pixi` / `pasp` + grid / irot / imir / clap + AVIS sample-table — pixel decode blocked at AV1 tile decode (rides [`oxideav-av1`](crates/oxideav-av1/)) | — |

</details>

<details>
<summary><strong>Trackers</strong> (decode-only by design) (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **MOD** | ✅ 4-channel Paula-style mixer + main effects | — |
| **STM** (Scream Tracker v1) | 🚧 Structural parse + **playback via shared mixer** (C3-relative `StmC3Pitch` + `StmSampleBody` `SampleSource` impl). Effects wired: Cxx set-volume, Axy volume-slide, Fxx speed/tempo. Hard-pan LRRL. Gaps: tone porta, arpeggio, Exy group, pattern break, vibrato. | — |
| **XM** (FastTracker 2) | 🚧 Structural parse + **basic playback** (`XmPitch` with Amiga + Linear tables, per-file selection; `XmSampleHeader` `SampleSource` with byte-offset → sample-index for 16-bit; sample-map-per-note routing). Volume column + Cxx/Axy effects wired. Envelopes parsed but not rendered; vibrato/tone-porta/fadeout/Exy/Gxy pending. | — |
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
| **`oxideav-sysaudio`** | Native audio output | ✅ Runtime-loaded backends (ALSA, PulseAudio, WASAPI, CoreAudio); no C build-time linkage |
| **`oxideav-pipeline`** | Pipeline composition (source → transforms → sink) | ✅ JSON transcode-graph executor; pipelined multithreaded runtime |
| **`oxideav-scene`** | Time-based scene / composition model | 🚧 Scaffold — data model for PDF pages / RTMP streaming compositor / NLE timelines; renderer still stubbed |

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
+ per-frame unsync, extended header, v2.4 data-length indicator,
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
