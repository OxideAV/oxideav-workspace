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
| **PCM** (s8/16/24/32/f32/f64) | ✅ | ✅ |
| **slin** (Asterisk raw PCM) | ✅ | ✅ |
| **FLAC** | ✅ bit-exact vs reference | ✅ bit-exact vs reference |
| **Vorbis** | ✅ all residue types (matches lewton/ffmpeg) | 🚧 stereo coupling + ATH floor; lacks point-stereo, full floor1 tuning, lookup-codebook optimisation |
| **Opus** | ✅ CELT mono+stereo + SILK NB/MB/WB mono+stereo all frame sizes | 🚧 CELT full-band + SILK NB/MB/WB; SILK MB/WB stereo only at 20 ms (lacks 10/40/60 ms); no Hybrid mode |
| **MP1** | ✅ all modes | 🚧 CBR only (no VBR) |
| **MP2** | ✅ all modes | 🚧 CBR mono+stereo only (no joint-stereo, no VBR) |
| **MP3** | ✅ MPEG-1 Layer III M/S | 🚧 CBR mono+stereo (no VBR/ABR, no intensity-stereo encode, no LSF MPEG-2/2.5) |
| **AAC** | 🚧 AAC-LC + HE-AACv1 SBR + HE-AACv2 PS; lacks LD/ELD, USAC, 4.0/5.1/7.1 channel layouts on PCE | 🚧 AAC-LC + HE-AACv1 mono+stereo + HE-AACv2 PS; **LC has 16× MDCT_FORWARD_SCALE mismatch vs ffmpeg** (clips ~82% on 24 kHz / 1 kHz, r23 target); psychoacoustic model basic; lacks PNS, gapless padding tuning, multichannel encode |
| **CELT** | ✅ full §4.3 pipeline | 🚧 mono + stereo intra-only long-block (no inter / short-block / transient encode) |
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
| **AC-3** (Dolby Digital) | ✅ Full decode + downmix (90+ dB vs ffmpeg) | 🚧 rematrix + short-block + transient + coupling + DBA; lacks E-AC-3 extension, per-block bit-pool tuning, channel-element-pair encode for 5.1+ |
| **AC-4** (Dolby) | 🚧 A-SPX + DRC + DE walker + ETSI Huffman tables (60 codebooks bit-validated) + ASPX_ACPL_1/2/3 transform synth (Pseudocodes 117/118/119); pending `sf_data(ASF)` Huffman bodies for multichannel | — |

</details>

<details>
<summary><strong>Video</strong> (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **MJPEG** | ✅ baseline + progressive 4:2:0/4:2:2/4:4:4/grey | ✅ baseline + progressive (SOF2 spectral selection) |
| **FFV1** | ✅ v3 all coder_types + 4:2:0/4:4:4 YUV + RGB+alpha + 9..16-bit; bit-exact ffmpeg | ✅ v3 range-coded + Golomb-Rice + 10-bit YUV + RGB+alpha; bit-exact through ffmpeg |
| **MPEG-1 video** | ✅ I+P+B | ✅ I+P+B (half-pel ME) |
| **MPEG-4 Part 2** | ✅ I+P+B-VOP, 4MV direct, half/quarter-pel, field-MV/DCT/alt-vertical-scan, GMC (S(GMC) routing wired) | 🚧 I+P+B + 1MV/4MV + intra-MB-in-P + quarter-pel + single-warp GMC + per-VOP-type quantiser knobs + GOP knob; lacks data partitioning, RVLC, multi-warp GMC, ACE Sprite VOP, MPEG-4 Studio profile, scalability layers |
| **Theora** | ✅ I+P (Theora has no B-frames) | ✅ I+P incl. INTER_MV_FOUR |
| **H.263** | 🚧 I+P + half-pel + Annex J (deblock) + Annex D (UMV) + Annex F (4MV/OBMC) + Annex E (SAC) + Annex N (RPS); lacks Annex G (PB-frames), I (advanced INTRA), K-W | 🚧 I+P + diamond ME + Annex F/J/D/N (RPS); lacks Annex G (PB), I, K, L, P, Q, R, S, T, U, V |
| **H.261** | ✅ I+P QCIF/CIF + integer-pel + loop filter | ✅ Baseline I+P with ME ±15 |
| **MS-MPEG-4** (v1 / v2 / v3) | 🚧 Clean-room scaffold; G4 VLC tables wired; G5 partial; canonical-Huffman walker tree at file 0x3df40 still OPEN; real-content (testsrc DIV3) still 5.30 dB Y | — |
| **H.264** | 🚧 ~45k LOC spec-driven rewrite; I/P/B + DPB + MMCO/RPLM + CAVLC + CABAC + 4:2:0/4:2:2 + B-pyramid POC ordering; lacks 4:4:4, MBAFF, Annex F (SVC), G (3D), H (MVC) | 🚧 Baseline-ish: I + P (1MV/4MV, ¼-pel) + B-slice 16x16/16x8/8x16 + B_Skip/B_Direct_16x16; lacks B_Direct_8x8, temporal direct, CABAC encode, intra fallback in P/B, weighted-pred, 4:2:2/4:4:4 emit, VUI/SEI tuning |
| **H.265 (HEVC)** | 🚧 I/P/B 8-bit + Main 10/12 + 4:2:0/4:2:2; SAO + deblock + scaling lists; HEIF/HEIC opt-in; pending 4:4:4 12-bit, broader B-slice merge/AMVP audit | 🚧 Baseline CAVLC I+P + Main-profile CABAC P-slice; lacks B-slice encode, merge-mode encode coverage, 10/12-bit, RDO refinements |
| **H.266 (VVC)** | 🚧 4:2:0 IDR intra (BDPCM, MIP, CCLM, ISP) + ALF/SAO/CC-ALF + P-slice merge+skip with integer-pel MC; lacks B-slices, fractional-pel MC, HMVP, temporal merge, MMVD/CIIP/GPM, dual-tree, full inter residual decode | 🚧 forward CABAC + DCT-II + flat quant only; residual emit + pixel output pending |
| **VP6** | ✅ Full FLV playback (845/845 sample frames) | 🚧 keyframe + skip + inter (integer-pel ME); ffmpeg interop on inter packets still failing — per-MB coefficient state machine alignment with ffmpeg's keyframe-derived defaults remains |
| **VP8** | ✅ I+P (6-tap sub-pel + MV decode + ref management) | 🚧 I+P + all 5 intra modes + SPLIT_MV + loop filter; lacks alt-ref / golden-ref planning, real RDO, segment maps |
| **VP9** | 🚧 Keyframe + inter + segmentation + bit-accurate MV + compressed-header probs; r21 made c64-constant lossless bit-exact (∞ dB); pending sub-8×8 mode-info path divergences | — |
| **AV1** | 🚧 OBU + range coder + coeff + partitions + transforms + all intra preds + CDEF + LR + inter MC + palette + multi-ref DPB compound; r21 SVT-AV1 chain 48/48 (full pass); pending TX shape coverage, palette finalization, intrabc, full loop-filter, full primary_ref_frame=4 chain | — |
| **Dirac / VC-2** | ✅ VC-2 LD + HQ intra + Dirac core-syntax intra/inter + OBMC + 7 wavelets + arithmetic + 10/12-bit; ffmpeg bit-exact 8-bit 4:2:2/4:4:4 + 10-bit 4:2:0 | 🚧 HQ + LD intra encoders only (no inter encode) |
| **AMV video** | ✅ (synthesised JPEG header + vertical flip) | ✅ via MJPEG encoder |
| **ProRes** | 🚧 self-roundtrip works (all six profiles, simplified exp-Golomb entropy); real ffmpeg-produced `.mov` decode blocked on SMPTE RDD 36 (see `crates/oxideav-prores/SPEC_BLOCKED.md`) | 🚧 self-roundtrip 44 dB at quant 4 — not interop-grade |

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
| **JPEG 2000** | ✅ Bit-exact vs OpenJPEG (incl. RGB MCT); Part-1 baseline + multi-tile + MQ + EBCOT + 5/3 + 9/7 + tier-2 + JP2 + 5 progression orders + multi-layer + POC + PPM/PPT | ✅ 5/3 lossless + 9/7 irreversible RGB + all 5 progression orders + POC + PPM/PPT |
| **JPEG XL** | 🚧 Signature + SizeHeader + partial ImageMetadata; Modular (MA-tree) and VarDCT pixel decode pipelines pending | — |
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
