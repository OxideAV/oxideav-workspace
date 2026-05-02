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
  Timestamp / PixelFormat / ExecutionContext + **DoS framework: `DecoderLimits`
  caps, `arena::ArenaPool` (Rc-based, single-threaded) + `arena::sync::ArenaPool`
  (Arc-based, Send + Sync) refcounted bump-allocator pools, refcounted `Frame`
  whose drop returns the buffer to the pool, `Decoder::receive_arena_frame()`
  trait method with default impl that wraps `receive_frame()` for true zero-copy
  per-decoder opt-in (h261, h263, vp6 ports done)**), `oxideav-codec` (Decoder /
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
  (`oxideav-mod`, `oxideav-s3m`) are decoder-only by design.
  New sibling crates landed this session: `oxideav-evc` (MPEG-5 EVC,
  ISO/IEC 23094-1), `oxideav-jpegxs` (JPEG XS, ISO/IEC 21122),
  `oxideav-midi` (Standard MIDI File + soft-synth scaffold).
  AVIF still register-but-refuses while gated on AV1 decoder completeness.
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
| **PCM** (s8/16/24/32/f32/f64) | ✅ | ✅ |
| **slin** (Asterisk raw PCM) | ✅ | ✅ |
| **FLAC** | ✅ bit-exact vs reference | ✅ bit-exact vs reference |
| **Vorbis** | ✅ all residue types (matches lewton/ffmpeg) | 🚧 floor1 envelope + point-stereo coupling + ATH floor + **trained VQ codebooks (in-tree LBG trainer + 4×256×16 books from LibriVox PD + Musopen Chopin CC0 corpus, 11.4% byte savings at matched SNR vs degenerate placeholder)**; lacks per-band point-stereo thresholds, floor0 LSP, lookup-codebook optimisation, bitstream-resident trained books in setup header |
| **Opus** | ✅ CELT mono+stereo + SILK NB/MB/WB mono+stereo all frame sizes + Hybrid | ✅ CELT full-band + SILK NB/MB/WB + Hybrid mono 20 ms (SWB 13 + FB 15) + **Hybrid stereo 20 ms (SWB + FB) + Hybrid mono+stereo 10 ms (SWB 12 + FB 14, via CELT LM=2 short-block path)** + SILK MB/WB stereo at 10/40/60 ms (RFC 6716 §3.2.1 1275-byte budget cap fixes libopus parse; ffmpeg + libopus cross-decode clean L/R RMS 0.07-0.79) |
| **MP1** | ✅ all modes | ✅ CBR + **VBR (two-phase psy-driven allocator: NMR-drop / bit ratio + energy/bit greedy fill, rolling controller for cumulative target convergence; 192 kbps target → 192.0 measured; silence avg 35.5 vs music avg 161.4 kbps on mixed input)** |
| **MP2** | ✅ all modes | ✅ CBR + VBR mono+stereo + **intensity-stereo joint mode (per-frame Pearson L/R correlation drives subband-bound choice from {4, 8, 12, 16}; -11.6% / -17.2% / -13.9% on highly-correlated stereo at q=0/1/2; Xing/Info frame on VBR; ffmpeg cross-decode clean)** |
| **MP3** | ✅ MPEG-1 Layer III M/S | 🚧 CBR + VBR mono+stereo + **MS-stereo joint mode (E_side/E_mid < 0.30 selection; 44.8% smaller on mono fold-down; ffmpeg cross-decode correct)**; lacks Bark-partition spreading psy-1, short/start/stop block selection, intensity-stereo encode, LSF MPEG-2/2.5 |
| **AAC** | 🚧 AAC-LC + HE-AACv1 SBR + HE-AACv2 PS; lacks LD/ELD, USAC | 🚧 AAC-LC + HE-AACv1 mono+stereo + HE-AACv2 PS; **r26 closed the long-running ffmpeg "No quantized data read for sbr_dequant" warning** (3 bugs: QMF analysis matrix `exp(...(2n+1))` per §4.B.18.2; time-delta envelope writer clamping to [0, 127]; flush() staging SBR FIL for inner AAC silence tail) + **PNS (Perceptual Noise Substitution per §4.5.2.4: peak/RMS ≤ 2.8 + RMS > 1e-4 + band-centre ≥ 4 kHz gate; 63.93% smaller on noise-rich fixture)** + **5.1 multichannel encode (chcfg=6, [SCE C, CPE L,R, CPE Ls,Rs, LFE], ffmpeg cross-decode clean, per-channel PSNR 22-36 dB)**; psychoacoustic model basic; lacks gapless padding tuning, 7.1 |
| **CELT** | ✅ full §4.3 pipeline | 🚧 mono + stereo intra-only long-block + short-block on transient detection + **per-band TF decisions (Viterbi search over TF_SELECT_TABLE, L1-norm cost per RFC 6716 §5.3.6)** + **dual-stereo Hybrid body for Opus stereo Hybrid** + **LM=2 short-block path (480-sample MDCT, configurable per-instance frame_samples/coded_n/lm; unlocks Opus 10 ms Hybrid)**; lacks libopus-bit-exact IMDCT, encoder-side comb pitch pre-filter |
| **Speex** | ✅ all NB modes 1-8 + WB 1-4 + UWB folding + intensity stereo + RFC 5574 in-band | ✅ full NB + WB ladder + UWB null + folding layers + RFC 5574 padding + Table 5.1 in-band |
| **GSM 06.10** | ✅ full RPE-LTP | ✅ full RPE-LTP (incl. WAV-49) |
| **G.711** (μ-law / A-law) | ✅ ITU tables | ✅ ITU tables |
| **G.722** | ✅ 64 kbit/s QMF + dual-band ADPCM | ✅ |
| **G.723.1** | ✅ 5.3k ACELP + 6.3k MP-MLQ | ✅ both rates |
| **G.728** | ✅ LD-CELP 50-order + ITU Annex B + §3.7 + §5.5 postfilter | ✅ |
| **G.729** | 🚧 CS-ACELP with non-spec codebook tables (audible but not bit-exact vs ITU) | 🚧 symmetric to decoder — same non-spec tables |
| **IMA-ADPCM (AMV)** | ✅ | ✅ |
| **8SVX** | ✅ | ✅ |
| **iLBC** (RFC 3951) | ✅ NB 20 ms + 30 ms + RFC §4.6 enhancer + §4.7 synth shift | ✅ 20/30 ms LPC + LSF split-VQ + RFC §3.6 residual CB search + opt-in §3.1 HP biquad. §3.6.2 perceptual weighting tested across 5 configs and disabled (regresses synthetic SNR; documented for future PESQ-based tuning) |
| **AC-3** (Dolby Digital) | ✅ Full decode + downmix (90+ dB vs ffmpeg) | 🚧 acmod 1/2/3/6/7 + LFE + rematrix + short-block + spec-§8.2.2 transient detector + per-channel `fsnroffst[ch]` tuning + coupling + DBA (24-45 dB cross-decode) + **E-AC-3 (Annex E) independent-substream encode (new bsi layout per §E.2.2.2 with strmtyp/substreamid/frmsiz/numblkscod, audfrm() block, errorcheck shrunk to 17 bits, ffmpeg cross-decode clean at 20.21 dB)**; lacks E-AC-3 dependent substreams, multichannel coupling (>2 fbw), per-block bit-pool tuning |
| **AC-4** (Dolby) | 🚧 A-SPX + DRC + DE walker + **ETSI Huffman tables (60 codebooks byte-for-byte against `docs/audio/ac4/ts_10319001v010401p0-tables.c`)** + ASPX_ACPL_1/2/3 transform synth (Pseudocodes 117/118/119) + multichannel `sf_data(ASF)` Huffman walk + **ASPX_ACPL_1/2/3 inner body walkers all wired** (joint-MDCT residual + companding_control + paired acpl_data_1ch); pending 7_X channel-element walker, mono/stereo short-frame `sf_data` walk | — |
| **MIDI** (SMF) | ✅ Standard MIDI File parser (Type 0/1/2, all channel-voice messages, sysex, meta events, running status, VLQ bounded ≤4 bytes) + **end-to-end SMF → PCM via 32-voice polyphonic mixer + tempo/division scheduler + full DAHDSR + pitch bend (RPN 0 range) + channel + poly aftertouch + sustain pedal + GM modulator chain + SF2 sm24 24-bit samples + stereo SF2 sample linking (cos/sin balance) + SF2 modulation envelope (gen 25-30) + SF2 RBJ-biquad LPF (gen 8/9/11) + exclusiveClass voice cuts** | — synthesis only |

</details>

<details>
<summary><strong>Video</strong> (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **MJPEG** | ✅ baseline + progressive 4:2:0/4:2:2/4:4:4/grey | ✅ baseline + progressive (SOF2 spectral selection) |
| **FFV1** | ✅ v3 all coder_types + 4:2:0/4:4:4 YUV + RGB+alpha + 9..16-bit; bit-exact ffmpeg | ✅ v3 range-coded + Golomb-Rice + 10-bit YUV + RGB+alpha; bit-exact through ffmpeg |
| **MPEG-1 video** | ✅ I+P+B | ✅ I+P+B (half-pel ME) |
| **MPEG-4 Part 2** | ✅ I+P+B-VOP, 4MV direct, half/quarter-pel, field-MV/DCT/alt-vertical-scan, GMC (S(GMC) routing wired), data partitioning (ARTS@L1, DC/MOTION marker decode) | 🚧 I+P+B + 1MV/4MV + intra-MB-in-P + quarter-pel + single-warp GMC + data partitioning (ARTS@L1, DC_MARKER 19-bit + MOTION_MARKER 17-bit) + per-VOP-type quantiser knobs + GOP knob; ffmpeg cross-decodes DP at 39-44 dB; lacks RVLC, intra-in-P MBs under DP, mid-VOP packet splits, Inter4MV under DP, DP+GMC, DP+B-VOPs, multi-warp GMC, ACE Sprite VOP, MPEG-4 Studio profile, scalability layers |
| **Theora** | ✅ I+P (Theora has no B-frames) | ✅ I+P incl. INTER_MV_FOUR |
| **H.263** | 🚧 I+P + half-pel + Annex J (deblock) + Annex D (UMV) + Annex F (4MV/OBMC) + Annex E (SAC) + Annex N (RPS) + Annex G (PB-frames decode) + **Annex I (advanced INTRA + AC prediction, INTRA_MODE VLC + INTRA TCOEF tables + alternate scans)**; lacks K-W, Annex M (Improved PB) | 🚧 I+P + diamond ME + Annex F/J/D/N + Annex G PB-frames + **r15 B-block residual emission (CBPB + MVDB + TCOEF VLC at BQUANT, B-half PSNR 28.8→55-57 dB)** + **Annex I (~21% smaller intra-rich, ~5× smaller flat content; ffmpeg 8.1 cross-decode clean)** (encoder + decoder symmetric); lacks intra-in-P MVD, Annex M, PB+Annex F, plus K, L, P, Q, R, S, T, U, V |
| **H.261** | ✅ I+P QCIF/CIF + integer-pel + loop filter | ✅ Baseline I+P with ME ±15 |
| **MS-MPEG-4** (v1 / v2 / v3) | 🚧 Clean-room scaffold; **G5 canonical-Huffman walker wired (file 0x3df40, format from `docs/video/msmpeg4/spec/11-walker-format-resolved.md`); v3 intra 3-tier ESC body (level-extension + run-extension + verbatim per `spec/04` §2.3); DC differential marker-bit fix (no MPEG-4 P2 §6.3.8 start-code-emulation guard in MS-MPEG4v3); v3 custom intra-DC direct-value VLC walker (4×120-entry tables at 0x05f*, format from spec/12 + spec/07 §5.4)**; testsrc2 32×32 PSNR rose 5.30 → 10.58 dB Y across rounds; testsrc 176×144 reaches 10.69 dB Y; remaining gap blocked on G5 descriptor extraction (spec/99 §9 OPEN-O4) before AC VLC tables decode correctly on real content | — |
| **H.264** | 🚧 ~45k LOC spec-driven rewrite; I/P/B + DPB + MMCO/RPLM + CAVLC + CABAC + 4:2:0/4:2:2 + B-pyramid POC ordering; lacks 4:4:4, MBAFF, Annex F (SVC), G (3D), H (MVC) | 🚧 Baseline-ish: I + P (1MV/4MV, ¼-pel) + B-slice 16x16/16x8/8x16 + B_Direct (spatial/temporal) + **B_8x8 mixed sub_mb_type (PSNR_Y 44.61 dB)** + **explicit weighted prediction (98.2% smaller on fade-in)** + **4:2:2 IDR emit (47.6 dB, chroma_format_idc=2, 8×16 chroma tile + 4×2 Hadamard DC + ChromaDc422 CAVLC)** + **4:4:4 IDR Intra_16x16 emit (42.25 dB, chroma_format_idc=3, profile_idc=244, chroma "coded like luma" per §7.3.5.3)**; ffmpeg interop bit-exact at 50-52 dB on B-frames; lacks CABAC encode, intra fallback in P/B, P/B at 4:2:2/4:4:4, multi-ref temporal direct, VUI/SEI tuning |
| **H.265 (HEVC)** | 🚧 I/P/B 8-bit + Main 10/12 + 4:2:0/4:2:2; SAO + deblock + scaling lists; HEIF/HEIC opt-in; pending 4:4:4 12-bit, broader B-slice merge/AMVP audit | 🚧 Baseline CAVLC I+P + Main CABAC P-slice + B-slice (TrailR mini-GOP=2) + per-CU SAD-RDO + **AMP partitions (−10.9% on high-contrast)** + **Main 10 IDR (45.05 dB ffmpeg, Qp'Y=38)** + **Main 12 IDR (45.04 dB ffmpeg, Qp'Y=50, profile_idc=4 RExt with max_12bit/max_422chroma/max_420chroma/lower_bit_rate constraint flags set per §A.3.7)**; ffmpeg cross-decodes I-P-B-P-B at 33-45 dB; lacks Nx2N / 2NxN, mvd_l1_zero_flag, mini-GOP > 2 at 10/12-bit, 4:4:4, RDO refinements |
| **H.266 (VVC)** | 🚧 4:2:0 IDR intra (BDPCM, MIP, CCLM, ISP) + ALF/SAO/CC-ALF + P/B merge+skip + §8.5.6.3 fractional-pel MC (8-tap luma + 4-tap chroma at 1/32-pel) + **HMVP (5-entry circular buffer per §8.5.2.6, merge-list pull-in)** + **temporal merge candidate (collocated MV at BR or centre 8×8-rounded position with same-CTB-row gate, §8.5.2.15 buffer compression `(mv>>4)<<4`, POC scaling per eqs 600-605, inserted after spatials before HMVP per §8.5.2.2 step 5)**; lacks pairwise-average, MMVD/CIIP/GPM, AMVR/BCW, BDOF/DMVR/PROF, dual-tree, full inter residual decode, affine/scaled-ref filter tables, explicit weighted prediction | 🚧 forward CABAC + DCT-II + flat quant only; residual emit + pixel output pending |
| **VP6** | ✅ Full FLV playback (845/845 sample frames) | 🚧 keyframe + skip + inter + **real DCT residual coding** (43 dB internal-decoder vs 18.86 dB MC-only baseline; ffmpeg cross-decodes end-to-end) + **quarter-pel sub-pel ME (29→35 / 35→37 dB Y on translating fixtures; ffmpeg interop 31.65 dB Y)** + **golden-frame refresh (configurable `golden_refresh_period`, per-MB pick by Lagrangian cost vs LAST/GOLDEN; 25% reduction on periodic-structure fixture; loop-back PSNR 45.12 vs 8.57 dB skip-from-prev baseline)**; lacks INTER_FOURMV, Huffman coefficient path |
| **VP8** | ✅ I+P (6-tap sub-pel + MV decode + ref management) | 🚧 I+P + all 5 intra modes + SPLIT_MV + loop filter + alt-ref / golden-ref planning + Lagrangian RDO + **per-MB-context MV-ref probabilities (two-pass; -13.4% / -22.9% on smpte/gray)** + **segment maps (variance-quartile classifier → per-MB segment ID + per-segment quant deltas, ffmpeg cross-decode clean; bit-saving config `[0,+2,+6,+12]` gives -14.2% bytes at sub-1 dB PSNR cost; spec-default `[-8,-4,0,+4]` shifts bits to perceptual win +1 dB on smooth half)**; lacks scene-cut adaptation, true look-ahead alt-ref synthesis |
| **VP9** | 🚧 Keyframe + inter + segmentation + bit-accurate MV + compressed-header probs; r22 fixed §6.4.3 `decode_partition` HORZ/VERT double-call (lossless-pattern Y: 10.41 → 47.70 dB, chroma bit-exact); **r24 §6.4.16 sub-8×8 inter mode-info per-4×4-sub-block iteration (decode_inter_block now does (idy, idx) walk for B4x4/B4x8/B8x4 — was reading exactly one inter_mode + assign_mv per ref slot per cell)**; pending residual asymmetry from per-sub-block `find_mv_refs` | — |
| **AV1** | 🚧 OBU + range coder + coeff + partitions + transforms + all intra preds + CDEF + LR + inter MC + palette + multi-ref DPB compound; SVT-AV1 chain 48/48 (full pass); `inverse_2d_spec` covers all 159 spec-allowed (TX_TYPE × TX_SIZE) pairs wired into all superblock call sites + **inter `tx_type` CDF reads (§5.11.45) with `Default_Inter_Tx_Type_Set{1,2,3}_Cdf` from §9.4 (inter Y 9.49 → 10.31 dB vs libdav1d, +0.82)** + **palette intra-within-inter path (read_palette_for_intra_within_inter gates per spec, MI grid stamping for cache, palette_screen.ivf decodes at 8.56 dB Y vs libdav1d ref)**; pending inter chroma `tx_type` (§5.11.40 TxTypes[][] tracking), intrabc, full loop-filter, full primary_ref_frame=4 chain | — |
| **Dirac / VC-2** | ✅ VC-2 LD + HQ intra + Dirac core-syntax intra/inter + OBMC + 7 wavelets + arithmetic + 10/12-bit; ffmpeg bit-exact 8-bit 4:2:2/4:4:4 + 10-bit 4:2:0 | 🚧 HQ + LD intra encoders + **Dirac core-syntax intra encoder (parse code 0x0C, AC-coded intra reference)** + Dirac core-syntax inter encoder (single-ref, integer-pel SAD ME ±16 pel, OBMC); **homogeneous core-intra + core-inter chain ffmpeg cross-decode is now hard-asserted** (intra 52.06 dB Y, inter 19.35 dB Y cross-decoded — closes the previous mixed-profile soft-skip); lacks sub-pel ME, OBMC overlap compensation, 2-ref bipred, wavelet residue |
| **AMV video** | ✅ (synthesised JPEG header + vertical flip) | ✅ via MJPEG encoder |
| **ProRes** | ✅ **RDD 36 entropy + slice/block scan + qScale + 8/10/12-bit (Yuv422P10/12Le, Yuv444P10/12Le, 60-68 dB ffmpeg interop on apcn + apch) + 4:4:4:4 alpha (§7.1.2 + Tables 12-14, ap4h interop clean) + interlaced (TFF + BFF, FieldStride mapping per §7.5.3, picture_structure ∈ {1,2}, ffmpeg `-flags +ildct` 40+ dB Y)**; encoder + decoder spec-compliant on §7.5.1 level shift `v = s/2^(b-9) − 256` (was off-by-2× pre-r4) | ✅ Emits valid RDD 36 frames; self-roundtrip ≥ 30 dB on all six profiles + interlaced + alpha + **custom luma/chroma quant matrices (perceptual JPEG K.1/K.2 lineage normalised to DC=2; 20-29% smaller packets at qi 2-16 vs flat-all-4; ffmpeg cross-decode 51 dB Y on perceptual matrices)** |
| **EVC** (MPEG-5) | 🚧 NAL + SPS/PPS/APS + slice_header + **full §9.3 CABAC engine + §7.3.8 slice_data walker + §8 intra prediction (5-mode Baseline) + §8.7.4 inverse DCT-II for nTbS ∈ {2,4,8,16,32,64} (the §8.7.4.3 64-point ambiguity resolved via closed-form `M[m][n] = round(64·√2·cos(π·m·(2n+1)/128))` cross-checked against printed tables) + §8.7.2/§8.7.3 dequant + per-CU intra/inter reconstruct + 8-bit yuv420p + Baseline P/B inter (8-tap luma + 4-tap chroma sub-pel, AMVP candidate list, default-weighted bipred) + cbf!=0 residual coding + EVC luma deblock**; `make_decoder` works end-to-end on Baseline IDR + P + B (PSNR=∞ on flat fixtures); lacks chroma deblock (eq. 1167-1182), Main profile init tables, RPL for non-IDR, HMVP/MMVD/AMVR/affine/ALF/DRA/IBC/ATS/ADCC | — |

</details>

<details>
<summary><strong>Image</strong> (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **PNG / APNG** | ✅ 5 color types × 8/16-bit, all 5 filters, APNG animation | ✅ same matrix + APNG emit |
| **GIF** | ✅ GIF87a/89a, LZW, interlaced, animation | ✅ GIF89a + animation + per-frame palettes |
| **WebP VP8L** | ✅ full lossless | ✅ subtract-green + predictor + colour transform; lacks ANS/WP entropy tuning |
| **WebP VP8** | 🚧 via VP8 decoder (VP8 decoder is complete) | 🚧 via VP8 I-frame + ALPH sidecar; gated by VP8 encoder completeness |
| **JPEG** (still) | ✅ via MJPEG codec | ✅ via MJPEG codec |
| **BMP** | ✅ 1/4/8/16/24/32-bit + V4/V5 + RLE4/RLE8 | ✅ 24/32-bit (V5) |
| **ICO / CUR** | ✅ multi-resolution + BMP/PNG sub-images + CUR hotspot | ✅ emits BMP (PNG for ≥ 256×256) |
| **JPEG 2000** | ✅ Bit-exact vs OpenJPEG (incl. RGB MCT); Part-1 baseline + multi-tile + MQ + EBCOT + 5/3 + 9/7 + tier-2 + JP2 + 5 progression orders + multi-layer + POC + PPM/PPT + **HTJ2K (Part 15) marker chain (CAP/PRF/CPF) + FBCOT entropy decoder (HT cleanup + SigProp + MagRef per Annex B, both CxtVLC tables verbatim from Annex C, MEL/UVLC/dual MagSgn-VLC streams; round-5 fixed 3 cleanup-decoder bugs: cq_non_first_linepair formula, γ_q exponent gate, U-VLC bit interleaving) + tier-2 LRCP walker that reuses Part-1 packet headers + multi-pass codeblock dispatch + 9/7 irreversible synthesis** (gated behind `htj2k` feature; 8×8 AZC fixture byte-exact; non-AZC HF-band magnitude reconstruction has remaining drift vs OpenJPEG — boundary-mirror / HF quant convention difference under investigation) | ✅ 5/3 lossless + 9/7 irreversible RGB + all 5 progression orders + POC + PPM/PPT |
| **JPEG XL** | 🚧 Signature + SizeHeader + ImageMetadata + ISOBMFF `jxlp` container + **2019 committee-draft Modular sub-bitstream pixel decode (ABRAC + BEGABRAC + MA-tree + 5 named predictors per Annex H) + 2021 FDIS migration in progress: round 1 ANS module (D.2-D.3.7 + both lookup tables, 5 documented spec typos in `project_jpegxl_fdis_typos.md`), round 2 FrameHeader + TOC + ImageMetadata refresh + general clustering, round 3 LfGlobal + GlobalModular + new modular_fdis.rs against ANS + live `JxlDecoder` factory**; cjxl 8×8 grey lossless fixture parses preludes correctly but pixel decode blocked on UNRESOLVED typo #6 (multi-leaf MA-tree symbol stream emits {8, 14, 113}, none is termination zero — root cause not yet found across 4 bisection rounds; Part 3/4 ISO docs confirmed methodology-only, conformance bitstreams now in `docs/image/jpegxl/conformance/`) | — |
| **JPEG XS** | 🚧 ISO/IEC 21122 (low-latency, SMPTE ST 2110-22) — **Part-1 codestream marker chain + inverse 5/3 reversible DWT (Annex E lifting) + Annex C precinct + packet entropy decode (significance + bitplane-count with raw/no-pred/vertical predictors + data + sign) + Annex D inverse quantization + length-driven slice walker + Annex F inverse colour transforms (RCT Cpih=1 + Star-Tetrix Cpih=3 with 4-step lifting cascade per F.5 + CTS + CRG markers) + Annex G (NLT linear/quadratic/extended 3-segment piecewise gamma + DC-shift) + multi-component decode (Nc>1, 4:2:2 / 4:2:0 subsampling) + multi-level DWT cascade (NL,x or NL,y > 1, picture-level cross-precinct gather) + CAP-bit decoder (star_tetrix / nlt_quadratic / nlt_extended / vertical_subsampling / cwd / lossless / raw_mode_switch flags)**; `make_decoder` works end-to-end on hand-built fixtures across all combinations | — |
| **AVIF** | 🚧 End-to-end decode (HEIF box walker → AV1 OBU); MIAF brand validation + `imir`/`clap`/`colr` (CICP+ICC) + multi-tile grid with tile-edge chroma ceil-div + `colr`/`pixi`/`pasp` grid→tile-0 fallback; gated by AV1 decoder completeness | — |

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
