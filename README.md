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
| **AAC** | ✅ AAC-LC (mono+stereo, M/S) + HE-AACv1 SBR + HE-AACv2 PS spec-accurate, 48 dB on HE-AACv1 reference. r17 wired `bs_limiter_gains` (Table 4.176). | ✅ AAC-LC + HE-AACv1 mono+stereo + HE-AACv2 PS. **r17 disproved PCM-scale hypothesis. r18 disproved structural sbr_dequant + SBR-envelope hypotheses** by experiment — the warning is benign EOF cleanup (same fdkaac stream produces it); INT16_SCALE_SQ sweep 1.0..1e20 + forced env=-200 all produce identical ffmpeg-decoded amplitude. **Real ffmpeg-interop gap is in the AAC-LC core spectrum scale** (LC-only mid-stream amplitude ~0.6× expected; ffmpeg-decoded peak 32_768 vs expected 9_830 → ~3.33× factor). r19 target: §4.6.2.3 `gain = 2^(0.25*(sf-100))` SF_OFFSET, §4.6.1.3 `sign(q)*|q|^(4/3)`, §4.6.11.3.1 IMDCT 2/N scale. 152 tests + 1 ignored regression. |
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
| **MPEG-4 Part 2** | ✅ I + P + B-VOP, 4MV direct, half-pel + quarter-pel MC, field-MV + field-DCT + alt-vertical-scan for interlaced B-VOPs. Bit-exact within rounding on testsrc. | ✅ I+P+B-VOP encoder (B-VOP-aware `-bf N`). 1MV/4MV + intra-MB-in-P fallback + **quarter-pel MC for both P-VOP and B-VOP** (verid=2 VOL). -17.5% bytes on sub-pel motion + bf=2 at constant Q. ffmpeg cross-decodes 36.60 dB. 130 tests. |
| **Theora** | ✅ I+P frames | ✅ I+P frames incl. INTER_MV_FOUR (45 dB PSNR, 3.7× vs all-I) |
| **H.263** | ✅ I+P, half-pel MC, Annex J deblock, Annex D URMV, Annex F (4MV + OBMC), Annex E SAC. 5-frame testsrc QCIF: 60–69 dB vs ffmpeg. | ✅ I+P + diamond ME. Annex F + Annex J SAC+VLC paths byte-identical roundtrip; **per-GOB SAC resync for AP path** (§E.5/§E.6 flush + §E.3 reset). 114 tests. |
| **H.261** | ✅ I + P on QCIF / CIF (integer-pel MC + optional loop filter); >66 dB intra, >68 dB clean P-chain vs ffmpeg. Long-clip drift documented since r5 was investigated in r15 — proven to not exist (dead-zone squashes IDCT roundoff). | ✅ Full Baseline I + P encoder with integer-pel ME ±15, FIL MTYPEs, MQUANT delta. testsrc QCIF: **39.37 dB byte-tight** vs ffmpeg. 80 tests. |
| **MS-MPEG-4** (v1 / v2 / v3) | 🚧 Clean-room implementer handoff. **r17 honest re-attribution**: the previously-wired "v3 intra-AC candidate" (`region_05eed0`) has wrong entry counts vs spec/99 §5 (G5 needs `count_A=102, count_B=66`); G4/G5 canonical-Huffman code-length arrays are NOT in `tables/` — they're packed inside reference-binary regions whose constructor at VMA `0x1c210ee6` is not in `spec/`. Real-content (32×32 testsrc DIV3) decodes at 5.30 dB Y. **Still spec-blocked** until extractor pass captures G4/G5 runtime descriptors. 160 tests. | — |
| **H.264** | 🚧 **Spec-driven rewrite** (~45k LOC). All §7/§8/§9 core layers; I/P/B-slice reconstruction, POC 0/1/2, DPB + MMCO + RPLM, 6-tap luma / bilinear chroma MC, weighted + direct MV, deblock, CAVLC + CABAC, 4:2:0 + 4:2:2. 16 integration tests. Pending: 4:4:4, MBAFF, Annex F/G/H. | 🚧 Clean-room Baseline encoder: I-only modes + I_NxN + luma/chroma AC + deblock + Lagrangian RDO + **P-slice with integer + half-pel + quarter-pel ME (§8.4.2.2.1 eq. 8-243..8-261)**. ffmpeg interop bit-equivalent (50.82 dB on 4-pixel-shift, 49.40 dB ½-pel, 53.61 dB ¼-pel). All sub-pel via the decoder's `interpolate_luma` so encoder/decoder agree bit-for-bit. 906 tests. Next: 4MV, B-slices. |
| **H.265 (HEVC)** | ✅ I/P/B decode 8-bit + Main 10/12 in 4:2:0 + 4:2:2 (8/10/12-bit) intra+inter, bit-exact. SAO + deblock + scaling lists. HEIF/HEIC opt-in. r17 confirmed AMP already wired. **r18 `interSplitFlag` audit**: libx265 emits non-conformant bin at the spec-forbidden gate (`tr_depth=0 < MaxTrafoDepth=0` is false); current empirical "read+use" path is 25.54 dB avg / frame 1 = 46.11 dB, vs 20.18 dB for spec-correct force-split=1, vs EGk overflow for spec-correct skip-bin. Frame 2/3 P-slice drift is NOT in the split path (deferred). `H265_TRACE_TT=1` trace dump added. 46 tests. Pending: 4:4:4 12-bit, MC half-pel edge / TMVP scan-order audit. | ✅ Baseline CAVLC I+P + Main-profile CABAC P-slice encoder. I 45 / P 31 dB via our decoder; ffmpeg accepts at 26.82 dB. |
| **H.266 (VVC)** | 🚧 4:2:0 IDR decodes end-to-end with color. Full intra + inverse transform + chroma reconstruction. Deblock + SAO + ALF + r17 CC-ALF apply (§8.8.5.7) + AlfFixFiltCoeff/AlfClassToFiltMap. r18 BDPCM intra mode (luma + chroma) — Tables 69/70/77/78 + 132 init/ctxInc, BdpcmDir mapping (0→ANGULAR18, 1→ANGULAR50), §8.7.3 eqs. 1145-1146 + 1153-1154 transform-skip dequant + accumulation. Gated on `sps_bdpcm_enabled_flag`. **r19 MIP (§8.4.5.2.2)** — boundary downsampling + 30 weight matrices (Tables 276..305) + transpose + upsampling end-to-end. **r19 CCLM (§8.4.5.2.14)** — 4:2:0 down-sampled-luma kernels (eqs. 366-369, both `chroma_vertical_collocated` branches), neighbour-sample kernels (eqs. 370-377 + bCTUboundary fallback eq. 373), eq. 400 divSigTable, 4-point min-max regression (eqs. 386-389), `(a, b, k)` derivation (eqs. 390-403), eq. 404 Clip1 predictor; wired for INTRA_LT_CCLM / INTRA_L_CCLM / INTRA_T_CCLM. 416 lib + 14 integration tests. Pending: P/B inter slices, dual-tree, ISP. | 🚧 Forward CABAC + forward DCT-II + flat quant landed (round-trip bit-identical against decoder). Residual emit + pixel output wiring still pending. |
| **VP6** | ✅ Full FLV playback (845/845 sample frames). | ✅ VP6F keyframe + skip + inter encoder (integer-pel SAD ME). **r18 audit**: ffmpeg specifically rejects the inter packet (container + keyframe parse cleanly per ffprobe), own decoder accepts the same bytes — encoder/decoder internally consistent. All header / mode-update / coeff-update probs verified against spec Tables 1-3, 7-8, 22-24, 31-35. Divergence is past the picture-header section; suspects: per-MB MB-type tree walk OR `coeff_dccv`/`coeff_ract` carry-through (`parse_coeff_models key=false` branch). Reusable env-gated diagnostic dump (`VP6_DUMP_INTER=1`) + audit summary in src/encoder.rs left for r19. 54 tests. |
| **VP8** | ✅ I+P frames (6-tap sub-pel + MV decode + ref management) | ✅ I + P frames, all 5 intra modes + SPLIT_MV + loop filter (42-51 dB PSNR) |
| **VP9** | 🚧 Keyframe + inter + segmentation + bit-accurate MV list + inter-mode ctx + compressed-header probs. **r18 §9.3.2 default_intra_mode tracker switched to spec-literal `+0`** (≥8×8 path) after the r17 audit unblocked the comparison — r15 had picked `+1` because it scored best vs the gray fixture, exposed as degenerate by r17. New honest baselines: lossless **Y 9.90 / U 10.80 / V 10.21 dB**, lossy compound mean Y **10.72 dB**. Sub-8×8 path keeps `+1` anchor (reverting to spec-literal regresses compound ~1 dB) — documented asymmetry. Spec text §6252-6253 flagged as likely transcription bug. 155 tests. | — |
| **AV1** | 🚧 OBU + range coder + coeff decode + partitions + transforms + all intra predictors + CDEF + LR + inter MC + palette + skipModeAllowed + multi-ref DPB compound MC. r17 fixed §5.9.2 `ref_order_hint` over-read + §5.9.20 `lr_unit_extra_shift` inversion. **r18 `parse_global_motion_params` (§5.9.24/§5.9.25/§7.20)** three correctness fixes: ROTZOOM/AFFINE read order (translation idx 0,1 AFTER alpha pair), TRANSLATION `absBits = GM_ABS_TRANS_ONLY_BITS - !allow_high_precision_mv` (= 8 at qpel), full PrevGmParams plumbing through the DPB (`RefSlot.saved_gm_params` + `Dpb::refresh_with_gm` / `saved_gm_params_for`, default identity matrix `1<<16` diagonals). SVT-AV1 35/44 Frame OBUs (chain prerequisite for r19). Sacred invariants preserved. 348 tests. Pending: full primary_ref_frame=4 chain, palette finalization, TX 64×56 / 32×41, loop-filter, intrabc. | — |
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
