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

> Each row below is a current-state summary. For round-by-round history, design notes, and per-feature trade-offs, see the per-crate `README.md` and `CHANGELOG.md` in `crates/oxideav-<codec>/`.

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
| **AAC** | ✅ AAC-LC (mono+stereo, M/S) + HE-AACv1 SBR + HE-AACv2 PS spec-accurate, 48 dB on HE-AACv1 reference. r17 wired `bs_limiter_gains` (Table 4.176). | ✅ AAC-LC + HE-AACv1 mono+stereo + HE-AACv2 PS. **r19 closes the LC interop investigation**: §4.6.2.3.3 SF gain, §4.6.1.3 inverse quant, §4.6.11.3.1 IMDCT 2/N — all spec-correct. Previously-claimed "~3.33× LC peak gap" was a peak-metric artefact (ffmpeg fills HF with PNS noise codebook 13). **RMS interop within ±5% of unity in all four directions** on 440 Hz mono. 166 tests + 1 ignored (HE-AACv1 SBR amplitude saturation, r20+ target). |
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
| **iLBC** (RFC 3951) | ✅ Narrowband 20 ms + 30 ms frames, RFC §4.6 six-PSSQ enhancer with §4.7 enhancer-delay synth shift (1 sub-block 20 ms / 2 sub-blocks 30 ms) | ✅ 20 ms + 30 ms frames (LPC analysis + LSF split-VQ + start-state coding + RFC §3.6 residual-domain 3-stage CB search + bit packer). **r19: residual-domain CB search + Table 3.2 bit-width caps + lMem=85 boundary CB + 30 ms LSF interpolation fix + §4.7 synth shift** lift voiced SNR from 9–11 dB to **22–25 dB** (sine 24.8/26.5 dB, voiced 22.1/24.5 dB). Opt-in HP pre-processing biquad (§3.1) suppresses DC + mains hum (+7 dB on DC-biased input). |
| **AC-3** (Dolby Digital) | ✅ Full decode + FFT IMDCT + §7.8 downmix. Sine 92.02 dB / transient 92.77 dB vs ffmpeg. | ✅ Encoder: rematrix + short-block emit + transient detection + §7.4 channel coupling. **r18: §7.2.2.6 + §5.4.3.47-57 DBA emission** — `deltbaie=1` block 0 with one segment per fbw channel at low-PSD band (+6 dB mask boost), `deltbaie=0` reuse on blocks 1..5; `compute_bap*` applies offsets, `tune_snroffst` accounts for syntax cost. ffmpeg cross-decodes (RMS 7944 vs self-decode 7958 on 440 Hz sine). 43 tests. |
| **AC-4** (Dolby) | 🚧 Full A-SPX front-end + DRC + DE + outer metadata walker. r17 wired ASPX_ACPL_2; r18 ASPX_ACPL_1 joint-MDCT residual layer; r19 lands the `5_X_channel_element` walker family (FiveXCodecMode + FiveXCodingConfig + Cfg3Five outer + LFE `mono_data(1)`). **r20 unblocks the ETSI Huffman-table audit** (`tests/etsi_table_validation.rs` parses `ts_10319001v010401p0-tables.c` and checks every codebook in `huffman_tables.rs` / `aspx_huffman.rs` / `acpl_huffman.rs` / `de_huffman.rs` / `drc_huffman.rs` byte-for-byte → 60 codebooks, 120 arrays, 0 divergences); also wires Cfg0 / Cfg1 / Cfg2 outer shells for the 5.X channel-element family + splits `parse_asf_psy_info_lfe()` from the regular psy_info parser (Table 106 column 4 `n_msfbl_bits`). 352 tests. Pending: `sf_data(ASF)` Huffman bodies for the multichannel elements + ASPX_ACPL_3 transform synthesis (Pseudocodes 117/118). | — |

</details>

<details>
<summary><strong>Video</strong> (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **MJPEG** | ✅ baseline + progressive 4:2:0/4:2:2/4:4:4/grey | ✅ baseline + progressive (SOF2 spectral selection) |
| **FFV1** | ✅ v3: 4:2:0/4:4:4 YUV + RGB via JP2K RCT + `extra_plane` alpha + 9..16-bit RGB + 10-bit YUV. All 3 coder_types + cross-frame state retention for `intra=0`. ffmpeg interop bit-exact on long-GOP, 10-bit Golomb, state-delta `-context 1`. | ✅ v3 range-coded YUV + multi-slice + 10-bit YUV + RGB 8-bit via JP2K RCT + Golomb-Rice (`coder_type=0`) + 10-bit Golomb encode + RGB `extra_plane` alpha encode (4th Golomb-coded plane), bit-exact through ffmpeg decode. |
| **MPEG-1 video** | ✅ I+P+B frames | ✅ I+P+B frames (half-pel ME, FWD/BWD/BI B-modes, 43 dB PSNR) |
| **MPEG-4 Part 2** | ✅ I + P + B-VOP, 4MV direct, half-pel + quarter-pel MC, field-MV + field-DCT + alt-vertical-scan for interlaced B-VOPs. Bit-exact within rounding on testsrc. | ✅ I+P+B-VOP encoder (B-VOP-aware `-bf N`). 1MV/4MV + intra-MB-in-P fallback + **quarter-pel MC for both P-VOP and B-VOP** (verid=2 VOL). -17.5% bytes on sub-pel motion + bf=2 at constant Q. ffmpeg cross-decodes 36.60 dB. **r19 codec options**: `qp` / `qp_i` / `qp_p` / `qp_b` per-VOP-type quantiser knobs (1..=31, monotonic bytes-vs-quality across full sweep) + `g` GOP-size knob (1..=300). 134 tests. |
| **Theora** | ✅ I+P frames | ✅ I+P frames incl. INTER_MV_FOUR (45 dB PSNR, 3.7× vs all-I) |
| **H.263** | ✅ I+P, half-pel MC, Annex J deblock, Annex D URMV, Annex F (4MV + OBMC), Annex E SAC. 5-frame testsrc QCIF: 60–69 dB vs ffmpeg. | ✅ I+P + diamond ME. Annex F + Annex J SAC+VLC paths byte-identical roundtrip; per-GOB SAC resync for AP path (§E.5/§E.6 flush + §E.3 reset). **r12 Annex D (UMV) emit** — PTYPE bit 10 wired, ME extended to `[-63, +63]` halfpel + §D.2 sign-of-predictor cascade in `encode_mv_component_umv`; 5-frame testsrc QCIF: **51 dB self + 51 dB ffmpeg cross-decode** (matches the no-UMV path on motion that fits in baseline range — UMV-only when MVs need extended reach). 123 tests. |
| **H.261** | ✅ I + P on QCIF / CIF (integer-pel MC + optional loop filter); >66 dB intra, >68 dB clean P-chain vs ffmpeg. Long-clip drift documented since r5 was investigated in r15 — proven to not exist (dead-zone squashes IDCT roundoff). | ✅ Full Baseline I + P encoder with integer-pel ME ±15, FIL MTYPEs, MQUANT delta. testsrc QCIF: **39.37 dB byte-tight** vs ffmpeg. 80 tests. |
| **MS-MPEG-4** (v1 / v2 / v3) | 🚧 Clean-room implementer handoff. **r18 wires G4 + G5 `pri_A` / `pri_B` byte arrays** from `region_0569c0` (3800-byte cluster region, spec/99 §10.3): G4 (count_A=102, count_B=57, inter chroma + all-inter, default for v1/v2) is fully wired (pri_A 102 B at file 0x57630 + pri_B 102×u32 at 0x57698); G5 pri_A (intra-luma, count_A=102, count_B=66) is wired but G5 pri_B lives in a 408-byte gap between 0x57898 and 0x57a30 not yet extracted. New `g_descriptor` module exposes `(idx → (last, run, |level|))` post-VLC mapping; 36 new tests cross-check audit/01 §3.3 (G4 ESCL(b) LMAX) and §4.1 (G5 ESCL(a) LMAX). The canonical-Huffman bit-length array (which feeds the VLC walk itself) still lives in the shared 68 KB walker tree at file 0x3df40 (spec/99 §5.3) and is OPEN. Real-content (32×32 testsrc DIV3) PSNR unchanged at 5.30 dB Y. **197 tests** (+37 vs r17). | — |
| **H.264** | 🚧 **Spec-driven rewrite** (~45k LOC). All §7/§8/§9 core layers; I/P/B-slice reconstruction, POC 0/1/2, DPB + MMCO + RPLM, 6-tap luma / bilinear chroma MC, weighted + direct MV, deblock, CAVLC + CABAC, 4:2:0 + 4:2:2. **Decoder r5 — speed pass**: 2.01× end-to-end on `solana-ad.mp4` (1280×720 yuv420p High@3.1, 3960 frames, ~158 s of content) — wall-time **89.83 s → 44.76 s** via `oxideplay --vo hash --ao null`, hash digest unchanged (`da9c18e4008cfd37`, regression-checked). Three independent wins: (1) `src/simd/{scalar,chunked,portable}.rs` skeleton matching the `oxideav-mpeg4video` shape; chunked `interpolate_luma` rewrites the 6-tap quarter-pel kernel so the H-FIR row strip is computed once and fed to the V-FIR — diagonal-j position **22.6×** on the bench (3645 µs → 161 µs / 1000 16×16 blocks); (2) cached debug-env-var lookups (`OXIDEAV_H264_BIN_TRACE`, `OXIDEAV_H264_CABAC_DEBUG`, `OXIDEAV_H264_MB_TRACE`, `OXIDEAV_H264_RECON_DEBUG`, `OXIDEAV_H264_NO_DEBLOCK`, …) — they were each resolving via `getenv()` syscall on **every CABAC bin decode and every per-MB parse** (~140 M lookups in this clip); now `OnceLock<bool>`-cached on first access; (3) eliminated per-MB clone of the `CabacNeighbourGrid` in `parse_residual_cabac_only` (was cloning ~720 KiB per MB to dodge the borrow checker; replaced with a NLL re-borrow → ~10 TB of `_platform_memmove` removed across the file). Plus deblock-filter fast paths that bypass the `Picture::luma_at`/`set_luma` clipped accessors when the 8-sample window is fully inside the picture. **Decoder r4**: CABAC fix for High-profile B-slice content with `transform_size_8x8_flag=1`. Two related bugs — (1) Table 9-24 (ctxIdx 402..=459, Luma8x8 frame/field sig + last + level) was missing from the context init pipeline → cat-5 contexts collapsed to neutral `(0,0)` → arithmetic decoder drifted out of sync; (2) §9.3.3.1.1.9 cat=1/2 `transBlockN` derivation short-circuited to "unavailable" when neighbour had `t8x8=1`, ignoring the spec's "8x8 block is transBlockN, cbf inferred 1 from CBP" branch. On `solana-ad.mp4`: slice-skip errors **1900 → 0** end-to-end. **922 tests** (+3 SIMD bit-exactness pins). Pending: 4:4:4, MBAFF, Annex F/G/H. | 🚧 Clean-room Baseline encoder: I-only modes + I_NxN + luma/chroma AC + deblock + Lagrangian RDO + **P-slice with integer + half-pel + quarter-pel ME (§8.4.2.2.1 eq. 8-243..8-261)** + **r19 4MV (P_8x8 PL08x8)** + **r20 B-slice (B_L0 / B_L1 / B_Bi 16x16)**. ffmpeg interop bit-equivalent: 50.82 dB on 4-pixel-shift P, 49.40 dB ½-pel P, 53.61 dB ¼-pel P, **54.19 dB on bipred-midpoint B (max diff 0)**. Bipred uses §8.4.2.3.1 default weighted average `(L0+L1+1)>>1`. All sub-pel via the decoder's `interpolate_luma` so encoder/decoder agree bit-for-bit. 915 tests. Next: B_Skip / B_Direct, intra fallback in P/B, CABAC. |
| **H.265 (HEVC)** | ✅ I/P/B decode 8-bit + Main 10/12 in 4:2:0 + 4:2:2 (8/10/12-bit) intra+inter, bit-exact. SAO + deblock + scaling lists. HEIF/HEIC opt-in. r17 confirmed AMP already wired. **r18 `interSplitFlag` audit**: libx265 emits non-conformant bin at the spec-forbidden gate (`tr_depth=0 < MaxTrafoDepth=0` is false); empirical "read+use" path matched libx265's emission. **r19 `cu_skip_flag` ctxInc + merge `ref_poc` refresh** (§9.3.4.2.2 + §8.5.3.2.9): `PbMotion` grew an `is_skip` bit; `skip_ctx_inc` now reads neighbour `is_skip` (was approximating with `is_inter` → over-counted condTermFlag for non-skip merge / AMVP neighbours, biasing CABAC ctxInc by one slot). Companion `refresh_pb_ref_poc` rewrites a merge candidate's stale `ref_poc_{l0,l1}` against the current slice's RPL[ref_idx] so the grid stops poisoning downstream TMVP scaling. Lifts Main 10 inter PSNR from 25.54 → **33.57 dB** average (frames 1/2/3: 46.11/26.34/20.54 → 40.05/37.86/28.25; net SSE drops 6.4×) and Main 12 inter from ~25 → **27.67 dB**. `H265_TRACE_TT=1` trace dump still wired. 47 tests. Pending: 4:4:4 12-bit, MC half-pel edge / TMVP scan-order audit. | ✅ Baseline CAVLC I+P + Main-profile CABAC P-slice encoder. I 45 / P 31 dB via our decoder; ffmpeg accepts at 26.82 dB. |
| **H.266 (VVC)** | 🚧 4:2:0 IDR decodes end-to-end with color. Full intra + inverse transform + chroma reconstruction. Deblock + SAO + ALF + r17 CC-ALF apply (§8.8.5.7) + AlfFixFiltCoeff/AlfClassToFiltMap. r18 BDPCM intra mode (luma + chroma) — Tables 69/70/77/78 + 132 init/ctxInc, BdpcmDir mapping (0→ANGULAR18, 1→ANGULAR50), §8.7.3 eqs. 1145-1146 + 1153-1154 transform-skip dequant + accumulation. Gated on `sps_bdpcm_enabled_flag`. **r19 MIP (§8.4.5.2.2)** — boundary downsampling + 30 weight matrices (Tables 276..305) + transpose + upsampling end-to-end. **r19 CCLM (§8.4.5.2.14)** — 4:2:0 down-sampled-luma kernels (eqs. 366-369, both `chroma_vertical_collocated` branches), neighbour-sample kernels (eqs. 370-377 + bCTUboundary fallback eq. 373), eq. 400 divSigTable, 4-point min-max regression (eqs. 386-389), `(a, b, k)` derivation (eqs. 390-403), eq. 404 Clip1 predictor; wired for INTRA_LT_CCLM / INTRA_L_CCLM / INTRA_T_CCLM. 416 lib + 14 integration tests. Pending: P/B inter slices, dual-tree, ISP. | 🚧 Forward CABAC + forward DCT-II + flat quant landed (round-trip bit-identical against decoder). Residual emit + pixel output wiring still pending. |
| **VP6** | ✅ Full FLV playback (845/845 sample frames). | ✅ VP6F keyframe + skip + inter encoder (integer-pel SAD ME). **r20 fix — Buff2Offset spec compliance**: per `vp6_format.pdf` Tables 2 & 3 the field is the literal frame-buffer byte offset of partition 2 — r19 had emitted/parsed it with a +/-2 fudge (internally consistent but wrong on the wire). Both encoder + decoder now match the spec definition (verified by manual hex parse of the dump produced by `VP6_DUMP_INTER=1`). ffmpeg still rejects the inter packet body — partition layout is now exonerated, narrowing the suspect list to `DEF_MB_TYPES_STATS` pair ordering vs spec page 30 `VP6_BaselineXmittedProbs` (our pairs are reversed → "stay-same" probability differs from spec formula output, even though the encoder/decoder are self-consistent). New `tests/ffmpeg_interop.rs` external-ffmpeg guards (`ffmpeg_accepts_keyframe`, `ffmpeg_decodes_keyframe_in_two_tag_stream`; skipped without ffmpeg on PATH) + `inter_buff2_offset_is_spec_compliant` regression pin. r20 audit notes in `src/encoder.rs` head doc. 57 tests. |
| **VP8** | ✅ I+P frames (6-tap sub-pel + MV decode + ref management) | ✅ I + P frames, all 5 intra modes + SPLIT_MV + loop filter (42-51 dB PSNR) |
| **VP9** | 🚧 Keyframe + inter + segmentation + bit-accurate MV list + inter-mode ctx + compressed-header probs. **r19 lossless reconstruction audit**: §8.7.1.10 WHT, §8.6.2 reconstruct, §8.5.1 DC_PRED, §9.3.2 KF_PARTITION_PROBS layout, §6.4.6 default_intra_mode tree, §6.4.25 get_scan, §6.4.24 token initial-context all confirmed spec-correct. The 9.90 dB lossless figure is a systemic bool-decoder-misalignment problem, not a single broken kernel. New `vp9-lossless-c64-constant.ivf` diagnostic fixture decodes to **Y=61.90 dB / U,V=∞ dB** with only 29 byte-diffs in a single 4×4 region — isolates the drift to one mis-decoded `skip` bit on a 16×8 H_PRED block. New WHT round-trip unit tests (DC=16, DC=-1792, AC1) prove the lossless transform is bit-correct. Headline numbers unchanged from r18: lossless **Y 9.90 / U 10.80 / V 10.21 dB**, compound mean Y **10.72 dB**. Spec text §6252-6253 flagged as likely transcription bug. 159 tests (+4 r19). | — |
| **AV1** | 🚧 OBU + range coder + coeff decode + partitions + transforms + all intra predictors + CDEF + LR + inter MC + palette + skipModeAllowed + multi-ref DPB compound MC. r17 fixed §5.9.2 `ref_order_hint` over-read + §5.9.20 `lr_unit_extra_shift` inversion. r18 `parse_global_motion_params` (§5.9.24/§5.9.25/§7.20) three correctness fixes (ROTZOOM/AFFINE read order, TRANSLATION absBits, PrevGmParams plumbing). **r19 chain-walk diagnostic + AFFINE bit-count regression test.** Pinned the SVT-AV1 chain baseline at **38/48 Frame OBUs** with the canonical `Dpb::refresh_with_gm` chain wired through (was 35/44 in r18 against an older fixture; r19 regenerates the fixture at 2-second/48-frame length and reaches 38/48 with the same parser code, ratifying the r18 plumbing). New `tests/svtav1_chain_walk.rs` asserts the 38/48 floor and reports the first-fail (currently pkt 3 Frame #7) so any parser regression in `parse_uncompressed_header` shows up immediately. Failure root-cause investigated: bitstream bit-account is spec-correct through `reduced_tx_set` (bit 110), then `global_motion_params` reads `is_global=1, is_rot_zoom=1` for slot 1 and `is_global=1, is_rot_zoom=0, is_translation=0` (= AFFINE) for slot 2 — AFFINE needs ≥24 bits but only 5 remain in the 17-byte payload. Whether this is a `set_frame_refs()` (§7.8) gap or an upstream bit-miscount in our parser is the r20 work item. New `frame_header_tail::gm_tests::affine_minimum_bit_count_for_identity_prev` locks the 33-bit AFFINE-with-identity-prev expectation. Sacred SkipMode + 41.97 dB compound invariants preserved. 350 tests (+2). Pending (carried into r20): full primary_ref_frame=4 chain, palette finalization, TX shape coverage, loop-filter, intrabc. | — |
| **Dirac / VC-2** | ✅ VC-2 LD + HQ intra + Dirac core-syntax intra/inter + OBMC + full IDWT (7 wavelets) + arithmetic coder + 10/12-bit. ffmpeg-interop bit-exact: 8-bit 4:2:2 + 4:4:4 + 10-bit 4:2:0. | ✅ HQ + LD intra encoders. HQ bit-exact through ffmpeg (∞ dB). LD testsrc 49.63 dB through ffmpeg. Comprehensive matrix: all 7 wavelets × 3 chromas. 132 tests. |
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
| **JPEG 2000** | ✅ **Bit-exact vs OpenJPEG including RGB MCT** (r16 fixed T.800 §G.1: inv DC level shift was running BEFORE inv RCT/ICT instead of after; saturated R/G/B/Y went 573/768 mismatches → 0). Part-1 baseline + multi-tile + MQ + EBCOT + 5/3 + 9/7 IDWT + tier-2 + JP2 + all 5 progression orders + multi-layer + user precincts + POC + PPM/PPT (end-to-end). 128 tests. | ✅ 5/3 lossless + 9/7 irreversible RGB + all 5 progression orders + POC + PPM/PPT. Bit-exact through opj_decompress. |
| **JPEG XL** | 🚧 Signature + SizeHeader + partial ImageMetadata parse — Modular (MA-tree) and VarDCT pixel decode pipelines pending | — |
| **AVIF** | 🚧 End-to-end decode works (HEIF box walker → AV1 OBU handoff → `VideoFrame`). Flat-content fixtures decode cleanly; rich content gated by AV1 quality. r17 closed `bbb_alpha`/`kimono_rotate90` investigations (AV1 surfaces clean Unsupported TX-shape errors). **r18: MIAF brand validation** (typed `BrandClass` + 8 brand consts: `avif`/`avis`/`avio`/`mif1`/`msf1`/`miaf`/`MA1B`/`MA1A`; per ISO/IEC 23000-22 §7); end-to-end `imir` + `clap` round-trip tests; **`colr` surface** (nclx CICP triple OR ICC payload, grid → tile-0 fallback). Brand classifier correctly identifies `alpha_video.avif` as image+sequence, `red.avif` as Advanced (MA1A) vs others Baseline (MA1B). 58 tests. Pending: gated on oxideav-av1 TX-shape coverage + intrabc + grid hardening. | — |

</details>

<details>
<summary><strong>Trackers</strong> (decode-only by design) (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **MOD** | ✅ 4-channel Paula-style mixer + full ProTracker 1.1B effect set. **PT-fidelity rounds 14-16**: sample loop boundary, sample-swap-without-note, LED filter, extended period range (108-907), vibrato sign, E6/Dxy + Fxx 0x20, **EE pattern-delay no longer retriggers held notes** (per Pro-Noise-Soundtracker §[14]). Real-world testing harness with 11 invariant-driven tests. 89 unit + 39 integration tests. | — |
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
