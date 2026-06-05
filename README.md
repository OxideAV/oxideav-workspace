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
  animate/set@t=0), `oxideav-pdf` (multi-page writer + Scene
  metadata via `/Info` dict; reader: bytes → Scene with xref +
  FlateDecode + content-stream operator parser + r35 inline-image
  extraction (ISO 32000-1 §8.9.7 BI/ID/EI framing)), `oxideav-raster`
  (vector→raster rendering kernel — scanline AA, bilinear/Lanczos2/Lanczos3 + Mitchell/Catmull-Rom/B-spline cubic image resampling,
  trapezoidal coverage, soft masks, patterns, filter primitives, ICC
  pipeline, bitmap cache keyed by `Group::cache_key`), `oxideav-ttf`
  (TrueType parser — cmap 0/4/6/12/14 incl. Variation Sequences, GSUB
  ligatures, GPOS kerning, COLR + CPAL + sbix tables, TTC subfont
  selection, AGL glyph-name→Unicode, full `name`-table accessor API), `oxideav-otf` (CFF / Type 2 charstrings incl. CID-keyed ROS/FDArray/FDSelect + arithmetic/stack/storage/conditional ops + Top-DICT FontMatrix/PaintType/CharstringType/StrokeWidth, ISOAdobe/Expert/ExpertSubset predefined charsets, cubic outlines; r222 GDEF + Coverage + ClassDef common-layout primitives + `GlyphClass` enum),
  `oxideav-scribe` (shaper with vector-first `Shaper::shape_to_paths`
  API — no rasterizer dep; trapezoidal horizontal AA, GPOS mark-to-mark,
  COLR/CBDT colour glyphs via raster bilinear/composer; bidi UAX #9 +
  USE still future work).
- **3D scenes & assets** — typed `oxideav-mesh3d` (Scene3D / Mesh /
  Material PBR / Skin / Animation / Camera / Light / AudioEmitter +
  area-weighted vertex-normal recompute + MikkTSpace-style tangent-space basis (Lengyel 2001) +
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
  `remux` / `transcode` / `run` / `validate` / `dry-run` / `convert`)
  and `oxideplay` (reference SDL2 + TUI player). Windows-codec
  forensic debugging now lives in [`KarpelesLab/univdreams`](https://github.com/KarpelesLab/univdreams)
  via `ud vfw {probe,decode,encode}` — see Windows codec sandbox below.

(`oxideav-job` and `oxideav-tracevfw` are retired — `oxideav-job`'s
functionality moved into `oxideav-pipeline`; `oxideav-tracevfw`'s
debugger CLI moved into `ud-cli` from univdreams, which also hosts
the underlying x86/PE/Win32 sandbox. Both archived on GitHub.)

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
| WAV       | ✅ | ✅ | ✅ | LIST/INFO + BWF `bext` + smpl/inst/plst + `fact` + `iXML` + `CSET` + RIFF MCI §3 23 LIST INFO sub-IDs + RF64/BW64 64-bit-extended form (EBU Tech 3306) |
| FLAC      | ✅ | ✅ | ✅ | VORBIS_COMMENT, streaminfo, PICTURE block; SEEKTABLE seek; CUESHEET round-trip; streaming Crc8/Crc16 validators |
| Ogg       | ✅ | ✅ | ✅ | Vorbis/Opus/Theora/Speex pages + comments; page-granule bisection + chained-link-aware duration + page-sync recapture + CRC-32 API + Skeleton 3.0/4.0 multi-stream keyframe-index seek + Theora granuleshift seek_to + branch-free checksum (~1.3 GiB/s); mux factory `open_with_skeleton` writes Skeleton BOS + per-stream fisbone + EOS at the control-section close |
| Matroska  | ✅ | ✅ | ✅ | MKV/MKA/MKS; Cues seek; SeekHead/Chapters/Attachments/subtitles; opt-in block lacing on write; EBML + per-Cluster CRC-32; typed Tag/TrackOperation/ContentEncodings/Video FlagInterlaced/FieldOrder/geometry quartet/Colour master + RFC 9559 §5.1.4.1.28 scalar children (matrix/range/transfer/primaries/chroma-siting/MaxCLL/MaxFALL)/SMPTE 2086/StereoMode/Projection/AlphaMode/UncompressedFourCC/BlockAdditionMapping read+write |
| WebM      | ✅ | ✅ | ✅ | First-class: separate fourcc, codec whitelist (VP8/VP9/AV1/Vorbis/Opus); inherits Matroska Cues seek |
| MP4       | ✅ | ✅ | ✅ | mp4/ismv; faststart; iTunes ilst; fragmented demux+mux (DASH/HLS/CMAF) + sidx/mfra/tfra/styp; AC-3/E-AC-3/DTS sample entries; subtitle/timed-text; CENC + saiz/saio + trgr + stdp + mehd + leva + tsel typed accessors + CENC scheme-decision router (cenc/cbc1/cens/cbcs) + seig sample-group entries; lacks AES-CTR/CBC decryption driver |
| MOV (QuickTime) | ✅ | — | ✅ | Apple QTFF + ISO BMFF meta + HEIF/HEIC item-properties + grid/iovl/tmap + fragmented-MP4 seek + DASH sidx/styp + saiz/saio + uuid + trgr + stz2 + stdp + imap + ssix + §8.7.6 padb typed accessors; ffprobe-accepted |
| AVI       | ✅ | ✅ | ✅ | AVI 1.0 + OpenDML 2.0 demux/mux; AVIX/dmlh/vprp + 2-field interlaced + VBR audio + LIST INFO + WAVEFORMATEXTENSIBLE + ODML keyframe seek + typed PaletteChange/TextChunk/AvihFlags/Idx1Flags + AVISUPERINDEX sub-type + per-stream strh fields (handler/SuggestedBufferSize/SampleSize/dwStart/wPriority/ChannelMask/Length) + dmlh.dwTotalFrames master frame-count override |
| Blu-ray (BD-ROM) | ✅ | — | — | `oxideav-bluray` Phase 2 — UDF 2.50 (ECMA-167 §14.9 FileEntry + §14.17 ExtendedFileEntry) + BDMV walk + `.m2ts` (192→188 strip) + `bluray://`; typed CPI EP_map + keyframe-aligned `TitleSource::seek_to(pts_90k)` + `StreamDecryptor` AACS hook + multi-angle PlayItem + chapters + STN_table → TrackCatalogue + STC PTS continuity + angle-change enumeration + in-place mid-stream `switch_angle_at`. Lacks HDMV opcode exec, BD-J |
| DVD-Video | ✅ | — | — | `oxideav-dvd` Phase 3 — ISO 9660 + UDF 1.02 + VIDEO_TS + IFO (VMGI/VTSI/TT_SRPT/PGCI/chapter materialiser + VOBU_ADMAP + TMAPTI + VTSI_MAT/VMGI_MAT video/audio/subpicture + karaoke MC ext attribute blocks) + VOB demux (MPEG-PS + Nav-Pack PCI/DSI + DSI_GI c_eltm typed PgcTime + AC-3/DTS/LPCM/subpicture router) + VOB → MKV + `dvd://` URI; Phase 3c VM (RegisterFile + RSM stack + SET-arith + CmpOps/SetOps + Type 4..6 compound) + SPU decoder + RGBA compositor + LPCM audio-pack + typed UOP mask + time-based seek. Lacks CSS auth |
| MP3       | ✅ | — | ✅ | demuxer LANDED (ID3v2/ID3v1 skip + Xing/Info VBR + CBR/VBR seek_to); r177 Decoder-trait stereo widening (independent + joint MS + intensity, planar AudioFrame) |
| IFF (EA IFF 85) | ✅ | ✅ | — | Whole `FORM/LIST/CAT` family — Amiga `8SVX` + `ILBM` (1..8-plane + 24-bit RGB, EHB/HAM6/HAM8, ByteRun1, HasMask, GRAB, SHAM, PCHG, CRNG/CCRT/DRNG, DEST destination-merge) + `ANIM` op-0/5/7 + Apple `AIFF / AIFF-C` (FORM/COMM/SSND walker, 80-bit IEEE-extended, PCM/ima4/ulaw/alaw, MARK + INST + COMT/AESD/APPL + MIDI + NAME/AUTH/(c)/ANNO + SAXL) |
| IVF       | ✅ | — | — | VP8 elementary stream container |
| AMV       | ✅ | ✅ | — | Chinese MP4 player format (RIFF-like) — clean-room demuxer + `AmvMuxer` + `seek_to` + lazy chunk-index cache + trailer-recovery + strict-mode sentinel validation (§2/§3 header + §3b audio WAVEFORMATEX + §4a/§4b chunk-payload shape validators) |
| FLV       | ✅ | ✅ | — | Flash Video — MP3/AAC/H.264 audio + VP6f/VP6a/H.264 video + Enhanced RTMP ExVideoTagHeader + AMF0 onMetaData/onXMPData/onCuePoint + Annex F encryption + E-FLV ModEx walk + multitrack body splitter + HDR colorInfo + 16 MB OOM guard + injection-robustness suite; muxer covers audio + §E.4.3 video tags + AVC seq-header/NALU/EOS + Enhanced-RTMP ExVideo/ExAudio + ExAudio multitrack writers + ModEx prefix emission + `onMetaData.keyframes` seek-table + HDR colorInfo writer (spec-range validated) |
| WebP      | ✅ | ✅ | — | RIFF/WEBP (lossy + lossless + animation; ANIM + ANMF emit) + §4.4 per-bundle inverse_color_indexing hoist |
| TIFF      | ✅ | ✅ | — | TIFF 6.0 single-image + BigTIFF + PhotometricInterpretation=5/8 CMYK + CIE L*a*b* decode/encode + CCITT T.4 2-D + T.6 (Group 4) fax decode/encode |
| PNG / APNG| ✅ | ✅ | — | 8 + 16-bit, all color types, APNG + gAMA/cHRM/zTXt + tRNS round-trip (typed Grayscale/Rgb/Palette; ct=4/6 rejected); metadata lacks only iCCP/iTXt |
| GIF       | ✅ | ✅ | — | GIF87a/GIF89a, LZW, animation + NETSCAPE2.0 loop + §23 disposal-method compositor + active-table palette iterator + stream-level GCE flag queries (transparency / user-input) + §25 PlainText typed iterator + §25.e printable/integer-cell-fit recommendation queries |
| JPEG      | ✅ | ✅ | — | Still-image wrapper around the MJPEG codec |
| BMP       | ✅ | ✅ | — | Windows bitmap — DIB headers BITMAPINFOHEADER / V4 / V5, 1/4/8/16/24/32-bit + BI_ALPHABITFIELDS (V3 four-mask alpha); also exposes the DIB helpers used by ICO / CUR sub-images |
| Netpbm    | ✅ | ✅ | — | All seven PNM magics + PAM (P1-P7); 1/8/16-bit; comment-tolerant ASCII + binary; user-defined PAM TUPLTYPE; ASCII (P1/P2/P3) hot-path rewrite + P7 PAM `GRAYSCALE` 16-bit row-level swap + P4 encode per-row memcpy (~20.7 GiB/s, ≈590× r228) |
| ICO / CUR | ✅ | ✅ | — | Windows icon + cursor — multi-resolution, BMP and PNG sub-images; body-dim `(0,256]` reject + CUR hotspot body-derived bound + dir wBitCount vs body biBitCount cross-check |
| slin      | ✅ | ✅ | — | Asterisk raw-PCM: .sln/.slin/.sln8..192 |
| MOD / S3M / STM | ✅ | — | — | Tracker modules (decode-only by design); XM vol-col panning-slide + instrument auto-vibrato byte selector + `+4` don't-retrigger flag; STM `9xx` set-sample-offset + S3M `Oxy` loop-aware sample-offset wrap |

Cross-container remux works for any pair whose codecs don't require
rewriting (FLAC ↔ MKV, Ogg ↔ MKV, MP4 ↔ MOV, etc.).

### Content protection

| Layer | Status | Notes |
|-------|:-------|-------|
| AACS  | ✅ Common 0.953 + BD-Prerecorded 0.953 | `oxideav-aacs` clean-room — KEYDB.cfg + MKB_RO/Unit_Key_RO parsers, Subset-Difference walk, Device-Key → PK → MK → VUK, AES-128-CBC Aligned Unit decryption, Title Key unwrap + Phase B SCSI MMC + Phase C Drive-Host AKE (ECDSA AACS curve + SHA-1 + AES-128-CMAC) + READ_DISC_STRUCTURE Format 0x81/0x82/0x83 + MKB ECDSA verify + BD-Prerecorded §2.3 Content Hash Table + signed Content Certificate verify + CRL parse/verify/lookup (PVB §2.7 Tables 2-2..2-5). Lacks AACS 2.0 |

</details>

### Codecs

> Each row below is a current-state summary. For round-by-round history, design notes, and per-feature trade-offs, see the per-crate `README.md` and `CHANGELOG.md` in `crates/oxideav-<codec>/`.

<details>
<summary><strong>Audio</strong> (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **PCM** (s8/16/24/32/f32/f64) | ✅ 100% | ✅ 100% |
| **slin** (Asterisk raw PCM) | ✅ 100% | ✅ 100% |
| **FLAC** | ✅ 100% — bit-exact vs RFC 9639 + CUESHEET → Chapter API + RFC 9639 §8.8 typed PICTURE accessor (parse + write) | ✅ 100% — bit-exact roundtrip + LPC order/window/precision search + closed-form Rice estimate + §8.6 PADDING writer + opt-in PADDING reservation + partitioned-Rice search O(1)-per-partition prefix-sum (~13-20% encoder speedup) |
| **Vorbis** | 🚧 ~85% (post-2026-05-20 orphan) — identification + comment + §3.2.1 codebook + §4.2.4 setup walker + §3.2.1/§3.3 VQ unpack + §8.6 residue (0/1/2) + §7.2 floor 1 + §6.2 floor 0 LSP + Vorbis window + §4.3.5 channel coupling + §4.3.7 IMDCT + streaming overlap-add | 🚧 ~35% — identification/comment/codebook/floor 1/floor 0/residue/mapping/mode header WRITE (§4.2.4 set complete); lacks audio-packet WRITE + setup-header splice |
| **Opus** | 🚧 ~35% — RFC 6716 range decoder + full SILK pipeline + §4.3 Table 56 CELT pre-band header + §3.1/§4.2 framing dispatch + §4.5 state-reset + §4.5.1.4 redundant-frame params + §4.3.2.1 CELT coarse-energy Laplace E_PROB_MODEL + §4.3.3 CACHE_CAPS50 + alloc-trim PDF + §4.3.3 band-boost decoder + §4.3.3 reservation block (initial-skip + 1/8-bit anti-collapse + skip/intensity/dual-stereo gating) + signalling gate; per-LM inter-mode `(α,β)` deferred | 🚧 scaffold |
| **MP1 / MP2** | ✅ Layer I + Layer II decode + §2.4.3.1 CRC-16 + frame loop + Annex D Phase-3 LTq + Model 2 spreading + free-format probe + ISO 13818-3 Annex B LSF Layer II allocation; allocator still pending D.1/D.3/D.4 | 🚧 ~88% — Layer I encoder + §C.1.5.2.7 bit-allocation + §C.1.3 polyphase analysis + §C.1.5.1.4 per-part scalefactor + §2.4.1.6/§C.1.5.2 top-level Layer II encoder + Mp1Layer2FrameEncoder stateful PCM-in + §2.4.1.8 ancillary_data tail (CRC-safe) + Mp1Encoder Layer I/II dispatch switch; pending Table C.4 SCFSI |
| **MP2** | 🚧 ~46% (post-2026-05-24 orphan) — Layer II header parser + frame sizing + Annex B tables + joint-stereo + scfsi + sample requantizer + LSF Layer II + Table C.4 SCFSI encoder + write_audio_data + §C.1.5.2.7 bit-allocator + encoder sample quantizer + frame-level `encode_frame` orchestrator + §D.1 Step 6 vf masking-function + Step 7 LTg global masker; lacks D.1 Steps 1-5 (#1262) + §D.2 Model 2 | 🚧 scaffold |
| **MP3** | ✅ ~100% — bit-exact decode + ID3v2/Xing seek + MPEG-2.5 framing | 🚧 ~96% — Phase-2 + long + pure-short + mixed-block per-band threshold-in-quiet + trait-API one-shot Annex D threshold-in-quiet factory + §D.1 Step 3 caller-supplied dB offset path + §D.1 Step 6 vf masking-function + Step 7 LTg global masker + §D.1 Step 4 critical-band-boundary Tables D.2a-f + §D.1 Step 4 masker-at-band placement + Step 5 decimation (LTq screen + 0.5-Bark tonal cluster collapse) + Step 7 nearby-masker Bark-window pre-filter + §C.1.5.3.2.1 spreading function; lacks Steps 1-3 FFT/SPL/tonality classifier + Model 2 + intensity-stereo |
| **AAC** | 🚧 Phase 1 — ADTS + raw_data_block walker + AudioSpecificConfig + program_config_element + §4.4.1 GASpecificConfig + Table 1.15 epConfig + §1.6.5 SBR/PS probe + §4.5.4.1 SWB tables + §4.6.13 pulse_data + §4.6.9.4 TNS clamp + §4.4.6 ics_body + §4.6.3 spectral codebook + HCB1/HCB2/HCB3/HCB4/HCB5; pending HCB6-11 + raw_data_block→ics_body | 🚧 scaffold — Phase-2 writers: section_data/ics_info/pulse_data/tns_data/scale_factor_data/DPCM/raw_data_block/Pce/gain_control/extension_payload; SBR pending QMF |
| **CELT** | 🚧 ~30% (post-2026-05-20 orphan) — RFC 6716 range decoder + §4.3 prefix + coarse-energy scaffold + bit-allocation fields + tf_change/select/spread + post-filter + de-emphasis + Walsh-Hadamard + cache_caps50 + dynamic-band-boost + initial-reservations walk + per-band minimums + trim_offsets + Table 55 + Table 57 static-allocation matrix + static-allocation search + §4.3.4.2 PVQ codebook + per-band shape decoder + §4.3.4.1 bits-to-pulses search + balance accumulator; blocked on docs Laplace | 🚧 scaffold |
| **Speex** | 🚧 ~37% — Ogg stream-header + NB + WB high-band + §5.5 in-band signalling + `BitWriter` + encoder-side write + 22 CELP companion-table accessors + NB LSP-VQ → Q10 LSP + §9.1 per-sub-frame LSP interp + NB 3-tap pitch-gain VQ + WB-HB 2-stage LSP MSVQ Q10 + NB+HB fixed-codebook (innovation) sub-vector lookup + per-mode dispatcher + NB adaptive-codebook 3-tap pitch-prediction index resolver + 145-sample `ExcitationBuffer` history; lacks §9.1 LSP→LPC + synthesis + UWB framing + mode-4 HB codebook binding | 🚧 scaffold |
| **GSM 06.10** | 🚧 ~35% — clean-room §5.3 RPE-LTP decoder + §4.4 in-band homing + §5.1 norm/div primitives | 🚧 §5.2.0..§5.2.13 + §5.2.18 — pre-processing + LPC analysis (autocorrelation/reflection/Schur) + §5.2.6 piecewise breakpoints + §5.2.7 LAR quantisation + coding + short-term analysis filter + LTP analysis clause + §5.2.13 weighting filter; `make_encoder` still `Unsupported` until §5.2.14..§5.2.17 + §1.7 packer |
| **G.711** (μ/A-law) | ✅ 100% | ✅ 100% |
| **G.722** | 🚧 clean-room SB-ADPCM decoder bring-up + BLOCK1/QMF predictor split + Table 19 RIL=11111 sign-anomaly fix + Figure 1 auxiliary-data channel + clause 2 transmission characteristics (typed normative-limits + dBm0 ↔ uniform-PCM bridge + idle-noise check) + clause 2.5.2 Figure 12 reconstructing-filter mask | 🚧 SB-ADPCM encoder bring-up + Mode-2/Mode-3 silence envelope round-trip + Appendix-II test-sequence harness (+ §II.3.2 Config-2 input #3) + QMF-bypass entry points |
| **G.723.1** | ✅ 100% | ✅ 100% — both 5.3k + 6.3k |
| **G.728** | 🚧 ~32% — clean-room decoder front-end: Annex A/B/C/D + Levinson + blocks 29-33 + AGC + §4.6 long-term comb (block 71) + short-term postfilter (block 72) + §4.7 pitch chain blocks 81/82/83/84 end-to-end driving (g_l, b, p) at the third vector of each frame + typed encoder scaffold + §3.9 E_j shape-energy table; lacks encoder pipeline | 🚧 scaffold |
| **G.729** | 🚧 ~14% — clean-room from staged trace #859: tables + serial parser + LSP-quantiser codebooks + corpus harness + §3.2.4 MA-predictor `fg` + LSP-frame reconstruction + per-subframe LSP interpolation + §3.2.6 LSP→LP conversion + §4.1 / Table-8 parameter unpacker (15 typed codewords + pitch parity) + §3.9.2 conjugate-structure gain-VQ decode reconstruction; lacks §4.1.1 bit-extraction glue + postfilter + Annex B DTX | 🚧 scaffold |
| **IMA-ADPCM (AMV)** | ✅ 100% | ✅ 100% |
| **MS-ADPCM / IMA-ADPCM (WAV)** | ✅ 100% | ✅ 100% — block-aligned WAV encoder for both nibble layouts |
| **OKI / Dialogic VOX** | ✅ 100% | ✅ 100% — symmetric §3 closed-form encode; mono-only via registry |
| **8SVX** | ✅ 100% | ✅ 100% |
| **iLBC** (RFC 3951) | ✅ 100% — NB 20/30 ms + §3.8 uneven-level-protection wire layout (3-pass class-1/2/3 pack/unpack; PSNR silence 95 dB / step-impulse 39 dB) + RFC 3952 §4.2 outbound SDP fmtp builders | ✅ 100% |
| **AC-3 / AC-4** (Dolby Digital / Dolby AC-4) | ✅ ~97% — AC-3 + E-AC-3 SPX/TPNP/AHT + §7.8.2 LtRt downmix + Annex D mix-level + WAVE_FORMAT_EXTENSIBLE + §7.10.1 CRC + augmented crc2 + typed BitStreamMode + E-AC-3 chanmap routing + CompressionGain + xbsi2/Surround EX/Headphone/AD-converter + AudioProductionInfo + TimeCode1/2/Presence + CopyrightInfo + typed DialNorm surface (§5.4.2.8/§5.4.2.16/§7.6) | 🚧 AC-3 ~95% + AC-4 IMS — 5_X SIMPLE/ASPX_ACPL_3 real γ₁..γ₄ per-band + write_aspx_data_{1,2}ch_real_envelope builders |
| **AC-4** (Dolby) | 🚧 ~98% — A-SPX + DRC + ETSI codebooks + 5_X/7_X ACPL_1/2/3 + cfg0/1/2/3 + LFE + SSF/SNF + SAP + Pseudocode 121 + IMS bitstream walker; lacks ETSI fixture RMS audit + object/a-joc | 🚧 IMS ~75% — v0/v2 TOC + mono/stereo/joint M/S + 5.0/5.1/7.1 SIMPLE Cfg3Five + 5_X SIMPLE/ASPX_ACPL_1/2/3 + real per-band α+β/α1+α2+β1+β2/γ5+γ6 + 7.0/7.1 SIMPLE/ASPX_ACPL_2 + ASPX envelope value-emitting helpers + encoder-side ASPX envelope extractor (P82/83 + P80/81 DPCM inverse); lacks γ1..γ4 + 7_X ACPL_3 β + Table-181 SAP residual + back-pair Lb/Rb |
| **MIDI** (SMF) | ✅ ~99% — SMF Type 0/1/2 → PCM via 32-voice mixer + SF2/SFZ/DLS + FF 01..07 text-meta iterator + smpte_offsets/FrameRate + channel-state seek primitive + sequencer_specifics (FF 7F) + sequence_numbers (FF 00 02) + midi_ports (FF 21) iterators; cargo-fuzz panic-free | ✅ SmfFile::to_bytes() / Track::to_bytes_chunk() mux-side writer + synthesis |
| **NSF** (NES) | 🚧 ~97% — full 6502 + IRQ/NMI + 5/5 2A03 APU + DMC DMA + six expansion chips + NSF v1/v2/NSFe + Dendy + Namco 163 + VRC7 OPLL pipeline + register semantics + KSR (Key Scale of RATE) + §4 KSL byte base table (YM2413 Table III-5) + Table III-7 decay-rate per-sample Q16 step + Decay/percussive-Sustain/Release lookup + MMC5 PCM Mode/IRQ + VRC6 sawtooth 14-step + E-clear accumulator zero; lacks Table III-7 attack-curve column + §7 LFO + rhythm mode | — synthesis only |
| **Shorten** (.shn) | 🚧 ~34% (post-2026-05-18 orphan) — `ajkg` magic + v2/v3 ulong + svar(n) + per-block function dispatch + VERBATIM/QUIT + DIFF0..3 + Rice residual + per-channel carry + running mean + QLPC predictor + `decode_stream` + `Decoder` trait + streaming decode + write_diff0/1/2/3_block + write_qlpc_block + write_zero_block predictor encoders (DIFF0..3 + QLPC + ZERO) + min_energy selectors + write_bitshift_command; lacks predictor-selection sequencer + BLOCKSIZE writer + #1267 | 🚧 scaffold |
| **TTA** (True Audio) | ✅ ~98% — TTA1 fmt=1/2 + password + ID3v1/APEv2 trailer + streaming + random-access decode API + ECMA-182 CRC-64 + duration-keyed player-API quartet (total_duration/seek_to_time/frame_iter_from_time/decode_from_time) + sample_range cargo-fuzz target | ✅ ~96% — TTA1 fmt=1/2 + password; bit-exact self-roundtrip |
| **APE** (Monkey's Audio) | 🚧 Phase 1 — 8-byte `MAC ` magic + decimal-coded version + 5 compression-level enum prefix parser + Display + 2040-input mutation harness; per-version header tail + IIR coefficients + residual `k` recurrence + range-decoder bounds + channel decorrelation all DOCS-GAP | 🚧 scaffold |
| **Musepack** | 🚧 SV7/SV8 — §2.5/§2.6 requantiser constants + stream-magic recognisers + SV8 packet outer-frame walker + SV7 mpc_huffman + CNS PRNG + §2.5 per-band sample-decode dispatcher + §2.6 reconstruction primitives + §2.4 SCF coding-method decoder + §2.3 band-type header loop walker + SV8 packet-stream walker (`PacketRef` / §3.2 SE termination) + typed §3.2 packet surface (`TypedPacket` per-key newtypes); lacks SV7 fixed-header field map + §2.3-VLC→§2.5-case remap + SV8 canonical-Huffman + 32-band synthesis | 🚧 scaffold |
| **Cook** (RealMedia) | 🚧 r11 — flavor table + cookie parser + 8 DSP parameter tables + open-time `DecodeConfig` + wire-level real-stream integration test + cookie→flavor multi-match API + selector-family classification + typed per-family GAP errors + stateful `CallSession` RADecode call-counter + PCM-cursor + structural `Driver`/`PreparedCall`/`decode_call` orchestrator + typed `CategoryIndex`/`CategoryParameters` parallel-table bundle (gain-step/gain-bias/level-count) + structural cookie-geometry guard (`validate_geometry`); lacks backend frame-decode | — |
| **WMA** | 🚧 r9 — patent-disclosed primitives (mid-side stereo + run/level walker + quantization-matrix differential coding + entropy-mode selector) + §6 codebook grid + escape disposition + §4 patent-disclosed quantization-band layout + §6 end-of-block terminator selector (explicit-ending vs implicit (N,1) event); lacks codeword Huffman tables / exponent partition / LSP codebook / sign-bit layout | — |
| **WavPack** | 🚧 ~89% (post-2026-05-18 orphan) — v4 block/metadata/decorrelation/entropy parse + LSB bit-reader + Golomb (base,add) + `parse_block` + AdaptiveMedians §3.2 + first PCM-producing decode_packed_samples_mono + stereo per-sample + EntropyInfo→AdaptiveMedians bridge + one-call `decode_samples()` + multi-block BlockIter + AudioBlockIter + `decode_stream` PCM composer + stream introspection + PackedCorrectionData (0x0B) typed view + CorrectionBlockIter + correction-block introspection; lacks hybrid 0x0B/0x0C consumer + float + multichannel + CRC + decorrelation prediction-loop + encoder | 🚧 scaffold |
| **APE** (Monkey's Audio) | 🚧 Phase 1 + polish — 8-byte `MAC ` magic + decimal-coded version + 5 compression-level enum + Display + 2040-input mutation harness + `CompressionLevel::ALL`/`iter()`/From/TryFrom/FromStr + const-fn accessors + `HeaderPrefix::new` constructor + `FILE_EXTENSION` + Default; per-version header tail + IIR coefficients + residual `k` recurrence + range-decoder bounds + channel decorrelation all DOCS-GAP | 🚧 scaffold |
| **DTS** (Core) | 🚧 ~49% — frame-sync header + 14↔16-bit pack/unpack + `iter_frames_14bit` + §5.4.1 ABITS/SCALES + Annex D §D.5.6 12-level BHUFF + §D.5.3/§D.5.4 small-Huffman + §D.1.1 RMS_6BIT + §D.1.2 RMS_7BIT + §5.3 SFREQ/AMODE/PCMR resolvers + §C.2.5 raCosMod 544-entry cosine-modulation matrix + §C.2.4 Sum/Difference Decoding + §C.2.3 Joint Subband Coding + §C.2.2 inverse-ADPCM 4-tap predictor + §C.2.1 Block Code mixed-radix unpacker; lacks subframe walker + §5.4 polyphase synthesis + DIALNORM | — |
| **aptX** (classic + HD) | 🚧 ~70% — 4-band QMF + ADPCM; bit-exact NDA-blocked + RFC 2361 §A.24 WAVE_FORMAT_TAG_APTX=0x0025 IANA tag + CODEC_ID_STR `aptx` registry | — |

</details>

<details>
<summary><strong>Video</strong> (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **MJPEG** | ✅ ~97% — baseline + progressive 4:2:0/4:2:2/4:4:4/grey + 12-bit YUV + SOF9 arithmetic + lossless SOF3 + RFC 2435 RTP/JPEG + §G.1.1 SOF2 4-component CMYK/YCCK + arith_decode fuzz + §3.1.7 restart-aligned packetization + 4-component lossless SOF3 P=8 (Adobe APP14 CMYK) decode + RGB24 baseline JPEG packed-RGB output + decode-free SOF/chroma/colour-hint inspector | ✅ ~97% — baseline + progressive + lossless SOF3 grey/RGB + DRI/RSTn + Pt 0..15 + 4-component CMYK encoder + 4-component lossless SOF3 P=8 + SOF0 Gray8 single-component lossy + SOF0 RGB24 baseline (Adobe APP14 + 'R'/'G'/'B' IDs) |
| **FFV1** | 🚧 ~87% — RFC 9043 decoder + demux + decode_frame driver (YCbCr v3-default 4:2:0 bit-exact end-to-end) + §4.6.6 per-slot state-buffer (YCbCr + RGB) + coder_type==2 + Golomb-Rice chroma-planes cursor fix + per-slot VLC sharing on Golomb-Rice RGB driver + §4.2.14-§4.2.17 Parameters tail parse + §4.9.3 per-Slice CRC validation gate (Reject default / Accept opt-in) | 🚧 ~97% — Slice Footer/Header + Golomb-Rice + YCbCr encoder + range-coded SliceContent + §4.7 RGB + RCT + unified encode_frame + §4.2 Parameters + §4.1 Quantization Table Set + §4.2.14-§4.2.17 tail emit (states_coded/ec/intra) |
| **MPEG-1 video** | 🚧 ~45% — sequence/GOP/picture/slice + macroblock walk + intra-DC + §2.4.3.7 dct_coeff walker + §2.4.4 dequantiser + §A 8×8 IDCT + IEEE P1180/D2 conformance + §7.3 mpeg2_inverse_scan + §6.2.6 MPEG-2 block(i) driver | 🚧 scaffold |
| **MPEG-2 video** | 🚧 ~74% — §6.2.x walk + §7.6.3.x PMV + §7.6.4-8 forming-predictions/combine/add-saturate + §7.4 inverse-quant + §A IDCT + §7.2.2 residual VLC + §7.3 inverse scan + §7.2.1 intra-DC + §6.2.5/§6.2.6 macroblock-block + §6.2.4 slice walker (opt-in `block_decoding_enabled` runs §6.2.6 `block(i)` per pattern_code) + per-slice §7.2.1 DC predictor carried + §7.6.6 skipped-macroblock + §6.2.5.1 macroblock_modes() + §6.2.5 motion_vectors + marker_bit + CBP wire-parse + §6.3.17.4 pattern_code[12] (4:2:0/4:2:2/4:4:4) + §7.6.3.1 PMV reconstruction wiring; lacks quant_matrix_extension downloads | 🚧 scaffold |
| **MPEG-4 Part 2** | 🚧 ~69% — I-VOP intra + inter texture + video_packet_header + §7.8.7.3 GMC + half/quarter-sample + Table 7-13 chroma MV + §7.6.9.5 B-VOP direct-mode + luma/chroma prediction + §7.6.5 chroma MVDCHR + §7.6.1.6 vector padding + §7.6.9.4 B-VOP chrominance MC + §7.6.5/Fig 7-34 spatial MV-predictor grid + §7.3 VOP reconstruction with [0,255] clip + §7.6.1.1 horizontal + §7.6.1.2 vertical repetitive padding + §7.6.1.3 extended padding; lacks §6.2.6.2 MV-body parser + MC driver + encoder | 🚧 scaffold |
| **Theora** | 🚧 ~57% — §6.1–§6.4 setup-header + Appendix B.2/B.3 VP3 tables + §6.4.x quant + DCT-token Huffman + §7.1–§7.5 frame walk + §7.5 motion vectors + §7.7.1 EOB Token + §6.4.1 LFLIMS + §7.7.2 Coefficient Token + §7.7.3 DCT Coefficient + §7.8.1 DC predictor + §7.8.2 Inverting DC Prediction + §7.9.2 Dequantization + §7.9.1 Predictors (intra/whole-pixel/half-pixel) + §7.9.3 Inverse DCT + §7.9.4 per-block reconstruction + §7.9.4 frame-level driver + §2.3/§2.4 coded-order Hilbert iterators; lacks §7.10 loop filter | 🚧 scaffold |
| **H.263** | 🚧 ~91% (post-2026-05-18 orphan) — §5.1-§5.4 baseline + §6 IDCT/MV/half-pel/INTER + Annex J deblock + Annex I AIC + Annex D UMV + Annex F 4-MV + OBMC + §5.1.4 PLUSPTYPE + Annex K §K.2 SS + AIC reconstruction + PLUSPTYPE inherited-state driver + custom-source-format GOB-layout + §K.2 SliceHeaderContext adapter + §K.2.1 SSTUF skipper + Annex G §5.3.3 MODB Table 11 + §5.3.4 CBPB 6-bit FLC PB-layer parsers + Annex M §M.4 Table M.1 Improved PB-frames MODB parser; lacks Annex K driver + PB-frame body integration | 🚧 scaffold |
| **H.261** | ✅ ~99% — I+P QCIF/CIF + integer-pel + loop filter + §5.4.1 BCH (511,493) t=1 correction + Annex B HRD + RFC 4587 RTP + RFC 3550 RTCP + §6.2.1 SDP offer/answer + cargo-fuzz `parse_rtp_payload` + Annex A IDCT-accuracy conformance + cargo-fuzz `parse_sdp_fmtp` + Annex D §D.2/§D.3 still-image sub-image transform hooks | ✅ ~98% — spiral+diamond ME + GQUANT-from-bitrate + BCH framing + RTP wrap + RTCP compound build/parse; 45 dB at 64 kbit/s QCIF |
| **MS-MPEG-4** (v1/v2/v3) | 🚧 ~50% — clean-room scaffold + `Macroblock4MvDecoder` 4-MV-per-MB tests + `GFamily` accessors + Figure 7-34 MV-predictor walk + 1-MV predictor via `predict_block_mv` + §7.6.5 4-MV-per-MB batch predictor + 4-MV neighbour-MB bordering-cell picker + 4-MV neighbour-state resolver + `Macroblock4MvDecoderNeighbours` + picture-wide `MvGrid` → `NeighbourSet` builder + G0..G3 packed-Huffman primary VLC wired; still lacks alt-MV VLC + 4-MV MCBPC | — |
| **H.264** | 🚧 ~83% — I/P/B + 4:2:0/4:2:2/4:4:4 + CAVLC + CABAC + DPB + 45 SEI types + fuzz-hardened + POC i64-staged + strict avcC parser + High-family extension trailer + CAVLC call-contract guards + Annex G MVC SEI 39/40/41/43 + NAL extension header (MVC/SVC/3D-AVC) + Annex H/I 3D-AVC SEI 50/51/54; lacks MBAFF, SVC/3D/MVC body | 🚧 ~83% — I+P (1MV/4MV, ¼-pel) + B + CABAC all chroma layouts + Trellis-quant RDOQ-lite; PSNR_Y 44.20 dB |
| **H.265 (HEVC)** | 🚧 ~62% — VPS+SPS+PPS + scaling-list + scan + §9.3 CABAC + slice header through §7.3.6.3 pred_weight_table + §7.3.6.2 ref_pic_lists_modification + §7.4.8 inter-RPS-prediction + §7.3.2.3.1 PpsExtensionFlags + §9.3.4.2 binarization + Table 9-49 split_cu/cu_skip + six Table 9-48 ctxInc + §7.3.2.2.1 SPS extension + §7.3.4 sao() per-CTU + §9.3.4.2.5 sig_coeff_flag + §9.3.4.2.6/.7 coeff_abs_level_greater{1,2} + §9.3.3.11 coeff_abs_level_remaining + §9.3.4.2.8 palette_run_prefix ctxInc; lacks Table 9-50 i=15 (#1414) + coeff_sign_flag + residual/IDCT | 🚧 scaffold |
| **H.266 (VVC)** | 🚧 ~73% — 4:2:0 IDR intra + ALF/SAO/CC-ALF + P/B merge+skip + HMVP + MMVD + CIIP + BCW + BDOF + GPM + AMVR + HBD + DMVR + affine + PROF + AMVP + SbTMVP + VPS + §7.3.10.10 amvr CABAC + decomposed §7.3.10.10 mvd_coding body parser (per-bin inspection surface) | 🚧 ~93% — forward CABAC + DCT-II + SAO/ALF/cu_qp_delta + MTT BT+TT RDO + P+B + sub-pel MC + multi-ref DPB + weighted bi-pred + §7.3.11.7 non-merge inter pre-residual + amvr_enc + §7.3.10.5 bcw_idx_enc + multi-CP-MV affine MVD + composite affine+AMVR+BCW dispatchers + reader-side composite walkers (affine + AMVR / + BCW) + decomposed §7.3.10.10 mvd_coding writer |
| **VP6** | 🚧 r22 — §13 static tables + §3 RawBitReader + §7.3 BoolCoder + §13.2.1 DC arithmetic + §13.3.1 AC coefficient arithmetic decoder + edge-clamped MC fetch + §13.3.3.1 decode_ac_zero_run BoolCoder walk + §11.1 MV-component arithmetic decoder + §11.2 per-frame MV-probability update (Table 13/14/15 walk) | 🚧 scaffold |
| **VP8** | ✅ 100% | ✅ 100% |
| **VP9** | 🚧 ~48% — §6.2 walk + §9.2 Bool decoder + §6.3 compressed-header primitives + §6.4.24 coeff + §8.6 dequant + §8.7 inverse transforms + §8.5.1 intra pred + §6.3.12 frame_reference_mode + §6.3.16 mv_probs outer sweep + §6.4.1 get_tile_offset + §6.4.2 decode_tile + §6.3 inter-arm `parse_compressed_header_inter` + §6.4 lines 2306-2311 tile-payload sizer (public `tile_payload_sizes`); lacks §6.2.5 inter-frame + §6.4.4 decode_block_apply + §8.4 loop filter | 🚧 scaffold |
| **AV1** | 🚧 ~95% — decoder feature-complete + standalone `decode_av1` entry + §6.7.2 Y-only monochrome + multi-SB dyn-Y dispatch up to 128×128 | 🚧 ~38% encoder — pixel-space YUV→IVF + 14-mode intra picker + §7.13.3 forward 2D + WHT lossless + forward quantize + §7.11.5.3 UV_CFL_PRED + base_q_idx>0 lossy + rectangular extents + monochrome encoder dyn driver + multi-super-block tiling (128 cap) + 4:2:0 YUV multi-super-block + §8.2.6 post-renormalisation probes + §7.13.3 rectangular TX_4X8/TX_8X4/TX_8X16/TX_16X8 forward dispatcher (× 2896 per-row post-scale); lacks remaining 10 rectangular TX sizes + §5.11.18 inter mode_info + RD picker |
| **Dirac / VC-2** | ✅ ~96% — VC-2 LD+HQ intra + Dirac core-syntax intra/inter + OBMC + 7 wavelets + 10/12-bit + bit-exact intra + fuzz oracle + Criterion bench + row-major slice + §12.4.4 extended_transform_parameters + §14.3/§14.4 fragmented-picture reassembler + v3 §10.5.2 Table 5 predicates + §14.5 fragmented_wavelet_transform DC-prediction kick | 🚧 ~97% — HQ+LD intra + Dirac core-syntax + adaptive sub-pel + 2-ref bipred + post-OBMC + rate-control + inter-encoder fuzz + VC-2 v3 symmetric/asymmetric extended_transform_parameters + §14.2 fragment-header parser |
| **AMV video** | 🚧 scaffold (orphan rebuild post-audit 2026-05-18; clean-room re-implementation pending) | 🚧 scaffold (orphan rebuild post-audit 2026-05-18; clean-room re-implementation pending) |
| **ProRes** | ✅ ~96% — RDD 36 entropy + 8/10/12-bit + 4:4:4:4 alpha + interlaced + RAW refused; ffmpeg interop 60-68 dB + cargo-fuzz + `idct8x8_dc_only` fast path + SHA-256 lockstep pin on 9 fixtures + 128×128 interlaced apcn + FIPS 180-4 §B.1/§B.2 self-check + §6.1.1 Tables 5/6/7 color-metadata reverse helpers | ✅ ~97% — RDD 36 all 6 profiles + interlaced + alpha + perceptual quant matrices + ffmpeg cross-decode + SHA-256 lockstep pin across every encoder entry + SHA-256 pin on 10/12-bit encoder paths |
| **EVC** (MPEG-5) | 🚧 ~92% — NAL + SPS/PPS/APS + §9.3 CABAC + §8 intra + DCT-II + P/B inter + RPL + HMVP + DPB + ALF + DRA + IBC §8.6 + §8.9.7 DraChromaDerived + §8.9.8 DraJoinedScaleFlag=1 + §7.4.3.1 SPS-signalled ChromaQpTable + SPS chroma-QP three-way adapter + DRA chroma-chain adapter + §8.5 AMVR + §7.4.7 MMVD distance/sign/offset + §8.5.2.3.9 bipred MMVD offset distribution + §8.5.2.3.9 entry-process signed POC scaling + §8.5.2.3.10 merge-candidate redundancy check + §6.5.1 tile-grid iterator; lacks Main-profile toolset (BTT/ADMVP/EIPD/ATS/affine) + #1278 §8.9.8 tableNum==0 branch + #1470 §6.5.1/§7.4.3.2 tile_id_val contradiction | — |
| **HuffYUV** / FFVHuff | ✅ ~97% — HFYU + FFVH FourCCs + 6 predictors + 8-bit + interlaced field-stride=2 + fast-LUT decoder + SWAR gradient post-pass + YUY2 LEFT macropixel-step + encode_roundtrip fuzz + Median tail-loop strip + LEFT-helper dedup + macropixel-step YUY2 Huffman-decode | ✅ ~96% — encoder symmetry × YUY2/RGB24/RGB32 + v1.x + v2.x ClassicV2/CustomV2 + YUY2 LEFT forward + forward_rgb_left_subtract_linear + dead-branch parity + macropixel-step YUY2 Huffman-ENCODE + histogram + verify bodies |
| **Lagarith** | ✅ ~95% — all 11 wire types + modern range coder + legacy adaptive-CDF + Fibonacci-Zeckendorf prefix + JPEG-LS Median + G-pivot decorr + zero-run RLE + pair-packed 513-entry CDF + modern RGB(A) first-column Rule B + deeper channel-body fuzz + lazy alpha-plane + early PixelFormatMismatch reject + packed-RGB(A) pack-loop branch hoist + frame-level type-1 size-guard wrappers (modern+legacy) + typed ChannelHeader accessor (spec/03 §2.1 + spec/06 §1.1) | 🚧 ~76% — encoder SOLID/RGB/RGBA/YV12/YUY2/legacy-RGB + Step-A/B/C `freqs[]` cache + per-channel header-form; byte-exact vs proprietary encoder Auditor-blocked |
| **Ut Video** | ✅ ~97% — 5 native FourCCs × 4 predictors + RGB inter-plane decorrelation + LUT-accelerated canonical Huffman + slice-parallel decode (5.63× at 720p) + criterion baseline + `Decoder` trait factory + Gradient/Median per-row branch-hoist + row-strided None + Left predictor refactor + content-adaptive trait-path predictor heuristic | ✅ ~96% — slice-parallel encode (3.28×) + content-fixture corpus + cargo-fuzz oracle |
| **MagicYUV** | ✅ 100% | ✅ 100% |
| **Cinepak** (CVID) | ✅ ~98% — frame header + multi-strip + V1/V4 codebooks + intra/inter + grayscale + Sega FILM demuxer + Saturn/3DO deviant + codebook_chunk_apply + `decode_vector_chunk` cargo-fuzz + criterion benches + `decode_multi_frame` cargo-fuzz + named seed-corpora + FILM PCM sample-shaping helpers (8-bit sign-magnitude/twos-comp + 16-BE + stereo deinterleave) | ✅ ~98% — stateful encoder + rolling codebooks + RDO + LBG + 3-axis grid picker + bitrate-target rate-control + keyframe-interval (34.18 dB PSNR; decode 4.4 GiB/s) + `EncoderOptions::vintage_compat` |
| **SVQ1/SVQ3** (Sorenson) | 🚧 r15 — SVQ1 framework + L=0..L=3 codebook payload + L=4/L=5 ABSENCE + saturating-clip + bit-mask helper LUTs + SVQ3 sub-pixel thirdpel interpolation arithmetic + SVQ3 macroblock transform + dequantization arithmetic + chroma-DC 2×2 scan + `place_coefficients_in_scan_order` generic placement helper; lacks intra-vs-inter ordering + stage interleave + 4×4 scan arrays + SVQ3 MV-VLC + #1256 svq3.c attribution scrub | — |
| **Indeo 3** (IV31/IV32) | 🚧 r18 — clean-room codec-frame header + bitstream + spec/02 picture-layer + spec/03 macroblock-layer + spec/04 VQ codebook + spec/06 entropy + spec/07 output + four cell-shape kernels + strip-context array + per-cell sub-array + spec/05 §1 mc_table + §2.2/§2.3/§3.3/§3.4 packed-MV + §5.4 cell-position + §4.2 ping-pong bank + §4.1 strip pixel-buffer arena + §4.3 source-pointer plumbing + §5.5 chroma-plane scaling + §5.6 MC fetcher→VQ residual chapter boundary surface + picture-layer → strip-context typed `PlaneDecodePlan` bridge; lacks §7.2 boundary fix-up + §7.3 reverse decomposition + MC inner loop | — |
| **Indeo 2/4/5** | 🚧 scaffold — pending clean-room workspace; Indeo 4/5 still sandboxed via `oxideav-vfw` | — |

</details>

<details>
<summary><strong>Image</strong> (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **PNG / APNG** | ✅ 100% — 5 colour types × 8/16-bit + APNG + sBIT/pHYs/tIME/bKGD/hIST/eXIf/sRGB/cICP/sPLT + Criterion benches + tRNS keyed transparency + APNG frame-scan bench + iCCP + iTXt round-trip + mDCV + cLLI HDR static metadata | ✅ 100% — sub-byte (1/2/4-bit) encode for colour type 0 / 3 |
| **GIF** | ✅ 100% — 87a/89a + LZW + interlaced + animation + disposal compositor + structured Application Extensions + Plain Text Extension + lenient mode + lazy Playback + animation-timing accessors + fluent AnimationBuilder + spec-derived fuzz corpus + §18.c.v Sort Flag accessors | ✅ 100% — per-frame palettes + `optimize_color_tables()` GCT/LCT hoisting + §7 Required Version enforcement |
| **WebP** (VP8 + VP8L) | ✅ 100% | ✅ 100% |
| **JPEG** (still) | ✅ ~95% — via MJPEG | ✅ ~90% — via MJPEG |
| **TIFF** (6.0) | ✅ ~98% — II/MM + BigTIFF + 7 photometrics + 1/4/8/16-bit + None/PackBits/LZW/Deflate/CCITT-MH/T.4-1D + tiles + multi-page + JPEG-in-TIFF (incl. CMYK) + PlanarConfiguration=2 + cargo-fuzz (7.7 M iter panic-free) + §SampleFormat (tag 339) decoder inspection | ✅ Gray8/16/RGB24/Palette8/CIELab/CMYK/YCbCr 4:4:4 (chunky) — None/PackBits/LZW/Deflate + Predictor=2 + PlanarConfiguration=2 + Bilevel CCITT-MH / T.4-1D + tiled chunky + tiled PlanarConfiguration=2 |
| **BMP** | ✅ ~97% — 1/4/8/16/24/32-bit + V4/V5 + OS/2 + RLE4/RLE8 + 3 fuzz targets + 31-test property sweep + V4/V5 colour-space + embedded ICC profile decode/encode | ✅ ~97% — top-down + biClrUsed-trimmed palette + `encode_bmp_with_icc_profile` + `encode_bmp_with_linked_icc_profile` (LCS_PROFILE_LINKED, UTF-16LE filename payload) + V5 + ICC encode accepts `Rgb565` + V5/linked-ICC writers accept indexed (Pal8) input |
| **Netpbm** (PBM/PGM/PPM/PNM/PAM) | ✅ ~95% — all 8 magics at 1/8/16-bit + 6 PAM TUPLTYPEs + cargo-fuzz harness + decoder pre-allocation OOM hardening + `read_be16_row` P5/P6/P7 16-bit + `swap_bytes_u16_row` LE→BE encode-side row helper (~48-50 GiB/s) | ✅ ~95% |
| **ICO / CUR / ANI** | ✅ ~98% — multi-res + BMP/PNG sub-images + CUR hotspot + ICONDIRENTRY validation + 256×256 PNG round-trip + standalone `read_ani_raw` + `biBitCount` reject + ANI `seq[]` step-index bounds-check + BMP body `biPlanes ∈ {0,1}` + biCompression {BI_RGB, BI_BITFIELDS} + biSize ∈ {40, 108, 124} reject | ✅ ~92% |
| **JPEG 2000** | 🚧 r20 (post-2026-05-20 orphan) — T.800 main-header + SOT/SOD + typed COC/QCC/POC/RGN/PLT/PPT + JP2 box + §B.10 tier-2 + §B.5 ResolutionLevel + §B.6 precinct + §B.7 code-block partition + §C.3 tier-1 MQ + 19 contexts + 5 packet iterators + POC + Annex F.3 inverse DWT + 4 fuzz targets + Annex E reassembly + Annex G MCT + §G.1 DC level-shift + §F.3.1 IDWT cascade + §D.5 segmentation symbol + Table A.19 + §D.7 vertically-causal + §D.6 selective arithmetic-coding bypass + §D.4.2 termination dispatch + Table D.9 schedule classifier; lacks §B.12 walker → BlockSource + §D.4.2 predictable-termination + HTJ2K | 🚧 scaffold |
| **JPEG XL** | 🚧 ~93% — ISO/IEC 18181-1:2024 lossless Modular + 7 fixtures pixel-correct + VarDCT scaffold + Gaborish/EPF/AFV pure-math + §C.8.3 per-block HF + PerPassNonZerosGrids + WP trace oracle (#799) + §C.5.4/§C.8.3 per-LfGroup varblock-walk + BlockContext() resolver + three-channel per-LfGroup varblock decode + multi-pass §C.8.3 outer loop + per-pass HF-header (`hfp`/histogram_offset = 495 × nb_block_ctx × hfp) routing; lacks WP fix (#799) + §C.7.2 histograms | — retired |
| **JPEG XS** | 🚧 ~82% — ISO/IEC 21122 Part-1 + 5/3 DWT + Annex C/D/F/G + multi-component + CAP-bit + high bit depth + 4:2:0 chroma at NL,y≥3 | 🚧 ~94% — Nc 1/3/4 + Sd>0 + RCT + Star-Tetrix + NL up to 8 + odd dims + vertical prediction + per-band Q + NLT + high-bit-depth Star-Tetrix lossless+lossy + per-slice Q[p] override + rate-budget Q[p] picker + rate-budget R[p] picker + joint per-slice Q[p] + R[p] rate-budget picker (bd∈9..=16) + per-precinct Q[p] override (Annex C.2 Table C.1) |
| **AVIF** | 🚧 ~92% — HEIF→AV1 + grid + imir/clap/colr/pixi/pasp + HDR + AV1 wrap + DoS caps + HEIF item-properties + auxC URN + rloc/lsel/iovl/grpl + iscl/rref + `mif1` + §4.2.2 tmap + ISO 21496-1 Annex C.2 GainMapMetadata + §5.2.5.3/§5.2.7 value-comparison + §8.2/§8.3 still-image profile audit + av1-avif v1.2.0 §3 AVIS audit + AVIS sequence-track profile audit + §8.6.6 Edit List + §6.5.18 crtt + §6.5.19 mdft creation/modification-time item-properties + `inspect_avis` aggregator | — |
| **DDS** | ✅ ~99% — DDS_HEADER + DXT10 + uncompressed (10 layouts) + BC1-7 + BC6H all 14 modes + mipmap + 6-face cubemaps + DX10 arrays + volume textures + 132-entry DXGI table + daily cargo-fuzz + 40-case injection-robustness + saturating-math + Criterion benches | ✅ ~96% — uncompressed + BC1-5 + BC7 all 8 modes + BC6H_UF16/SF16 all 14 modes + box-downsample mip chains + cubemap/array + BC6H second LSQ refinement pass in 17-bit unq space (+1.75 dB PSNR) |
| **OpenEXR** | 🚧 ~92% — magic + 8 required attrs + HALF/FLOAT/UINT + NO_COMPRESSION/ZIP/ZIPS/RLE + tiled ONE_LEVEL + sub-sampled chroma + single-part deep scanline/tiled + multi-part deep scanline/TILED + multi-part flat TILED ONE_LEVEL/MIPMAP/RIPMAP + single-part deep tiled MIPMAP/RIPMAP + multi-part deep tiled RIPMAP + multi-part mixed (scanline + tiled per-part) + typed inspectors for int/double/string/v2i/v3i/v3f/m33f/m44f/chromaticities attribute payloads; PIZ blocked on docs trace | ✅ ~96% — RGBA scanline + single-part deep tiled + multi-part deep TILED + multi-part flat MIPMAP/RIPMAP + single-part deep tiled MIPMAP/RIPMAP + multi-part deep tiled RIPMAP + multi-part mixed scanline+tiled |
| **Farbfeld** | ✅ 100% — streaming reader + DoS hardening + `magick` black-box cross-validator + SIMD-friendly hot-path BE swap (~10× parse / ~9× encode; parse 3.6→39 GiB/s, encode 4.7→46 GiB/s) | ✅ 100% |
| **HDR** (Radiance RGBE) | ✅ ~99% — new-RLE + old-RLE + 8 axis-flag combos + shared-exponent + EXPOSURE/COLORCORR/PRIMARIES/VIEW + apply_exposure/colorcorr + luminance_lm_per_sr_per_m2 + uncompressed scanline R+W + `HdrLimits` + fuzz + effective_primaries() + chromaticity-derived RGB↔XYZ matrices | ✅ ~98% — new/old/auto-RLE + XYZE↔RGB + 8 tonemap ops + CRLF + zero-copy `reorient_for_axis_flags` |
| **QOI** | ✅ 100% — byte-exact vs all 8 reference fixtures + criterion decode bench (540 MiB/s gradient, 1.55 GiB/s solid-RUN) + encode_roundtrip cargo-fuzz | ✅ 100% — byte-exact vs reference encoder + encoder cursor-write hot path (1.85× RGBA gradient) + caller-owned-buffer `_into` variants |
| **TGA** | ✅ 100% — typed §C.6.4 KeyColor + §C.6.5 PixelAspectRatio + §C.6.6 GammaValue + §C.6.7 SoftwareVersion accessors + footer-walker helpers | ✅ 100% |
| **ICER** (JPL) | 🚧 ~78% — Mars-rover heritage; bit-plane scan + compressed/uncompressed segments + 8 filters + IPN 42-155 §III.B context model + §III.E lenient multi-segment decode (DSN packet-loss tolerant) | ✅ ~84% — quota encoding + auto wavelet selection + R-D byte-budget + per-segment §III.D uncompressed fallback + criterion bench sweep over segments ∈ {1,2,4,8} + PSNR-target rate-control (`with_quality_target`, bisection over byte budget) |
| **WBMP** | ✅ 100% — Type 0 + WbmpLimits DoS caps + adversarial fuzz sweep + caller-selectable MonoBlack/MonoWhite decode polarity | ✅ 100% — accumulator-flush pack in `encode_wbmp_from_dither` (8-pixel batching, partial-byte tail) |
| **PCX** (ZSoft) | ✅ ~98% — 1/2/4/8 bpp planar + packed-bits + 24 bpp RGB planar + grayscale flag + DCX multi-page + DCX `Demuxer` + fuzz-hardened + Criterion bench + 1bpp × 3 planes (8-colour EGA RGB) decode + window-origin + screen-size round-trip; lacks 4bpp × 4 planes EGA RGBI | ✅ ~94% — 8 write paths + DCX + framework `Encoder` Rgba/Rgb24/Gray8 + Bgr24/Bgra/MonoBlack/MonoWhite + `encode_pcx_1bpp_3planes_ega_rgb` + `encode_pcx_24bpp_window_dpi_screen`; lacks framework `PixelFormat::Pal8` |
| **ILBM** (Amiga IFF) | ✅ ~94% — BMHD/CMAP/CAMG/BODY + ByteRun1 RLE + EHB + HAM6/HAM8 + PBM + SHAM + PCHG + ANIM op-0/op-5 + CRNG/CCRT + DRNG (DPaint IV extended range); lacks ANIM op-7/op-8, DEEP true-colour | ✅ ~84% — IlbmMuxer parity + masking + ANIM op-5 + CRNG/CCRT/DRNG encoder |
| **PICT** (Apple QuickDraw) | ✅ ~99% — v1 + v2 opcode walkers + drawing rasteriser + DirectBitsRect packType 0..4 + Region + clip + pen-size + Compressed/UncompressedQuickTime skip + indexed PixMap + §A-3 reserved v2 opcode skip + v1 §A-3 Table A-3 completion + structured Picture Comments ($00A0/$00A1) + §A-3 fontName/lineJustify/glyphState structured capture into PictTextState; lacks text rasterisation + embedded CompressedQuickTime 0x8200 JPEG | ✅ ~94% — `PictBuilder` + every v2 drawing-command family + §A-3 Indexed-PixMap + structured `PictHeader::{ExtendedV2,V2}` parser + §A-3 emitter + r236 fontName/lineJustify/glyphState builders |
| **SVG** | ✅ ~99% — full shape set + path + gradients + text + mask + clipPath + use/symbol + svgz + SMIL animate/set/animateTransform + CSS3 Selectors L3 + `@import`/`@font-face`/`@keyframes` + Media Queries L4 + viewBox + 18 filter primitives (+ §15.9 feBlend) + CSS Values L4 + CSS Easing L2 + SVG 2 pathLength/`<view>`/fragment-identifier/`<switch>`/`<marker>`/context-fill+stroke/`<a>` hyperlink/display+visibility/title+desc/metadata/text-anchor/textPath/clip-rule/shape-rendering/text-rendering | ✅ ~88% — round-trips full shape graph + PreservedExtras + `<view>` re-emit |
| **PDF** | ✅ ~99% — bytes → Scene via xref/xref-streams/ObjStm + `/Prev` + `/Encrypt` R=2..6 + public-key + PKCS#7 + `/Sig` AcroForm + Doc-Timestamp + text extraction + Linearization + Tagged-PDF + EmbeddedFiles + §12.6 actions + 5 stream filters + §8.11 Optional Content + §14.13 Associated Files + 6 §12.5.6 annotation subtypes + Watermark/Redact/PrinterMark/TrapNet; Movie/Sound/Screen/3D/RichMedia remain `Other` | ✅ ~99% — PDF 1.4/1.5 multi-page + paths/gradients/opacity/clip + RGBA + xref-stream + ObjStm + Linearization + `/Encrypt` + public-key + `/Sig` + AcroForm + annotation writer + embedded files + RFC 3161 Doc-Timestamp + §12.5.6 Line/Polygon/PolyLine/Caret/Popup writer |

</details>

<details>
<summary><strong>3D scenes & assets</strong> (click to expand)</summary>

> The typed Scene3D / Mesh / Material PBR / Skin / Animation / Camera / Light / AudioEmitter model lives in `oxideav-mesh3d`, with `Mesh3DDecoder` / `Mesh3DEncoder` traits and a `Mesh3DRegistry` that's parallel to `oxideav-core::CodecRegistry`. Per-format crates register into it. `oxideav-meta::populate_mesh3d_registry(&mut Mesh3DRegistry)` walks every enabled format's `register()`. Lazy bytes flow through `AssetSource` (with a `raw_storage` pass-through hook for archive-backed sources, e.g. ZIP-stored USDZ textures + audio).

| Format | Decode | Encode |
|--------|--------|--------|
| **STL** (ASCII + binary) | ✅ ~99% — ASCII + binary + per-face attrs + 16-bit colour + multi-`solid` + topology + 9-step repair pipeline + `repair_translate_to_positive_octant` + `repair_make_winding_consistent` + `repair_split_t_junctions` + `ValidationReport::defect_total`/`defects_by_rule` accessors + `Bbox::point`/`merge`/`expanded_by`/`intersect`/`intersects`/`contains_bbox` AABB-lattice helpers + `inspect_binary_header` pre-decode triage hook (no Scene3D) | ✅ ~99% — both formats + attribute pass-through + `EncodeStats` + configurable float precision |
| **OBJ** (+ MTL) | ✅ ~98% — full Wavefront grammar + MTL (Phong + Wavefront-PBR + map_* + typed refl) + smoothing/display attrs + free-form geometry + `xyzrgb` per-vertex colour + Bezier/B-spline/NURBS/Cardinal/Taylor `curv` + `surf` 2D-surface tessellation + cargo-fuzz + `curv2` 2D trimming-curve + `scrv` special-curve + MTL `illum` decomposition + multi-patch Bezier `surf` decomposition + `con` connectivity + `call`/`csh` general directives (capture-only); lacks surface-aware tri-edge-constrained re-meshing | ✅ ~96% — symmetric + negative-index encoder + polyline rejoin |
| **glTF 2.0** (+ .glb) | ✅ ~98% — JSON + .glb + full PBR + 12 KHR_materials extensions + skin + skeletal animation + sparse accessors + morph-targets (incl. quantized per KHR_mesh_quantization) + 12 spec-MUST validators + KHR_texture_transform + KHR_mesh_quantization + KHR_node_visibility + KHR_xmp_json_ld at 5/7 spec surfaces + KHR_animation_pointer + KHR_materials_variants + KHR_texture_basisu (KTX2 indirection round-trip, with/without fallback); lacks KHR_audio_emitter (spec not yet staged) + KHR_draco_mesh_compression + KHR_meshopt_compression | ✅ ~93% — symmetric round-trip incl. KHR_xmp_json_ld declarations and packet preservation |
| **USDZ** (+ USDA) | ✅ ~94% — ZIP STORED walker + USDA parser + UsdGeomMesh + UsdPreviewSurface PBR + UsdUVTexture + xformOp + UsdMediaSpatialAudio + variantSet + LIVRPS variant-selection + composition-arc round-trip + sublayer + references/payload + inherits/specializes + reader-side CRC-32/ISO-HDLC + `.usdc` Crate bootstrap parser + §3b CompressedBuffer/Chunk + §4.1 TokensSection + §4.2 STRINGS + §4.3 FIELDS framing + §4.4 FIELDSETS framing + §4.5 PATHS leading-prefix parser (numPaths + repeat invariant); lacks §4.5 trailing 3-buffer body (DOCS-GAP) + FIELDS value-rep type-codes + UsdSkel* + UsdGeomSubset | ✅ ~88% — symmetric writer + zero-re-encode pass-through + variant writer + composition-arc writer |
| **FBX** | 🚧 ~92% — binary + ASCII container + object-graph + mesh + animation + deformers + Material/Texture/Video + bind pose + LayerElementMaterial/Color + Properties70 P-record grammar + multi-UV-set + Light + Camera NodeAttribute + ASCII FBX writer (`write_ascii_document`, banner option, parse-write-parse closure) + bind-pose `bone_to_parent` derivation. Lacks: multi-LayerElementNormal | ✅ ~58% — symmetric binary writer + opt-in zlib deflate |
| **Alembic** | 🚧 0% — Sphinx API reference + Python examples staged at `docs/3d/alembic/`; on-disk Ogawa binary needs Wayback PDF recovery (Imageworks 2010-2012 manuals 404 today) or commissioned trace | — |

Cross-format integration: `oxideav-cli-convert` exposes a 3D conversion path through `oxideav_meta::populate_mesh3d_registry` — `oxideav convert in.obj out.gltf` (or `--probe` for structural inspection). `crates/oxideav-tests/tests/mesh3d_*.rs` runs the cross-format roundtrip suite. Convert verb has accumulated IM-compatible ops including `-resize` / `-thumbnail` / `-define` / r178 `-extent WxH±X±Y` (canvas re-window w/ source-order `-background` colour) / r184 `-monochrome` (gray + 2 colors + Floyd-Steinberg shorthand) / r222 `-roll ±X±Y` (IM-style circular pixel shift — columns right by `dx`, rows down by `dy`; negative = opposite direction), USDZ encoder + 3D→raster renderer (Gouraud + Phong + `-light` / `-camera` / `-projection` / `-fov` / `-bg`), `-render normal-debug|depth-debug` + `-aa N` supersampling, and multi-size ICO via `-define icon:auto-resize`. Black-box oracles in `tests/mesh3d_{usdz_apple,blender_assimp}_oracle.rs` cross-validate against Apple `usdzconvert` + Blender + assimp.

</details>

<details>
<summary><strong>Trackers</strong> (decode-only by design) (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **MOD / STM / XM** | ✅ ~97% MOD — 4-channel Paula mixer + full ProTracker 1.1B effects + FT-extension 8xx/E8x pan + XM E3x glissando + Lxy set-envelope-position + E4x/E7x vibrato/tremolo waveforms + cargo-fuzz; ✅ ~92% STM — `StmDecoder` real + STM `E6x` pattern loop + `EEx` pattern delay + `E9x` retrigger-note; ✅ XM — full playback decoder + Rxy multi-retrig per-nibble memory (y=0 reuses last speed, x=0 reuses last volume modifier) | — |
| **STM** (Scream Tracker v1) | ✅ ~85% — structural parse + shared-mixer playback; XM-parity effects (Gxy/Jxy/Bxy/Cxy/Exy/Hxy + 7xy tremolo + volume-slide variants); hard-pan LRRL | — |
| **XM** (FastTracker 2) | ✅ ~90% — structural parse + full playback; envelopes + fadeout + key-off; vibrato + tone porta + pattern jumps + fine/extra-fine porta + Exy/Kxy subcommands + volume-column slides | — |
| **S3M** | ✅ ~96% — stereo + full ST3 v3.20 effect set + per-channel effect memory + Dxy case matrix + S3x/S4x bit-2 retention + Qxy persistent-counter retrigger + Ixy persistent two-counter procedure + tick-0 Ixy firing + stored-vs-active volume split + Cxx row-≥64 ignore + Kxy/Lxy continue + +128 channel-mute + spec-correct default-pan + header-driven playback corrections + Vxx range + §Mixing MV-byte clamp + stereo×11/8 gain + PCM active-volume peak=63 + S00→SDx double-trigger; lacks AdLib FM synth | — |

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
The sandbox itself lives in
[`KarpelesLab/univdreams`](https://github.com/KarpelesLab/univdreams)
as the `ud-emulator` crate; `oxideav-vfw` is a thin bridge that
adds OS-aware codec discovery (`$XDG_DATA_HOME/oxideav/codecs/` +
cache) and registers ud-emulator-backed `Codec`s into
`oxideav-core::CodecRegistry`. VfW codecs expose both decode
(`ICDecompress*`) and encode (`ICCompress*`, `SandboxedVfwEncoder`)
through the sandbox; DirectShow filters are decode-only. Design contract in
[`docs/winmf/winmf-emulator.md`](https://github.com/OxideAV/docs/blob/master/winmf/winmf-emulator.md).

| Codec | Binary | Test fixture | `ICDecompress` | Notes |
|-------|--------|--------------|----------------|-------|
| Indeo 3 (IV31) | `IR32_32.DLL` | `cubes.mov` 160×120 | ✅ ICERR_OK | Integer ISA only |
| Indeo 5 (IV50) | `IR50_32.DLL` | `cat_attack.avi` 320×240 + 3 more | ✅ ICERR_OK 8/8 frames | MMX kernels active (1.5M-5M dispatches/frame post-r20 FloatingPointProcessor registry probe + EFLAGS.ID / RDTSC / Pentium II CPUID fixes) |
| Indeo 4 (IV41) | `IR41_32.AX` | `crashtest.avi` 240×180 + `indeo41.avi` 320×240 | ✅ ICERR_OK 8/8 frames each | MMX kernels active |
| MSMPEG4 v3 (DIV3) | `mpg4c32.dll` | wmpcdcs8-2001 reference binary | ✅ **DECODE 17/17 frames at 42.9 dB PSNR-RGB + ENCODE externally validated** — full ICCompress lifecycle wired; 176×144 BGR24 → 970-byte MP43 I-frame (78×); self-roundtrip 27.83 dB; AVI 1.0 wrap decodes through ffmpeg + mpv + ffprobe (mean 20.86 dB at q=5000). Covers I/P, skip-MB (~38%), alt-MV-VLC, AC-prediction. | 13 stubs + x87 ISA + DirectShow GUID + `ICINFO_SIZE = 568`; codec rejects non-BI_RGB output 4CC. |
| MSMPEG4 v3 DShow | `mpg4ds32.ax` | winxp | ✅ **Full GOP DirectShow decode + 20/20 across 16 fixture-runs** — covers 6/6 FOURCC variants (MP43/DIV3/DIV4/DVX3/AP41/COL1) routed through MP43 subtype; motion-pan-352×288 + skip-MB + AC-pred fixtures all green. | DirectShow IBaseFilter wrapper: COM scaffolding + ole32 stubs + HostIFilterGraph + HostIPin + HostIMemAllocator + HostIMediaSample + IMediaFilter. CLSID `{82CCD3E0-F71A-11D0-9FE5-00609778EA66}`. |
| WMV1/2 DShow | `wmvds32.ax` | winxp | CLASS_E_CLASSNOTAVAILABLE on default CLSID | Needs the shipped `wmvax.inf` filter CLSID; round-26+ |
| MSADDS audio | `msadds32.ax` | winxp | 🚧 **Pipeline driven through Receive, E_FAIL inside inner-decode (r70)** — PE-load + COM + dual-pin allocator handshake green; ffmpeg-derived extradata flips Receive HRESULT 0x8000FFFF → 0x80004005. r70 pinned actual bail JCC at `0xe282` (`cmp edi, [ebp+0x10]` / `jge → 0xe2bb`), EDI=0x748 = sample-count bound. r69 `0xea3a` hypothesis falsified; r63 helper_addref retired. | Same scaffolding as MP43; `AmtBlueprint::wma_*`; QueryAccept disasm at `docs/codec/msadds32-query-accept-validation.md` |

**Architecture** — the `ud-emulator` engine is a 4 GiB MMU + i386
integer ISA + MMX ISA (~50 opcodes) + x87 FPU (8-deep stack) +
PE32 loader + Win32 stub surface (kernel32 + user32 + msvcrt +
winmm + advapi32 + ole32 + vfw32) + **a COM dispatch layer**
(`Guid` parser + `ComObjectTable` ref-count bookkeeping + vtable
dispatch + class-factory cache covering IUnknown / IClassFactory /
IBaseFilter / IPin / IMemAllocator / IMediaSample / IFilterGraph)
for codecs that ship as DirectShow filters rather than VfW drivers
(`.ax` exposing `DllGetClassObject` instead of `DriverProc`). Both
ud-emulator and oxideav-vfw are `#![forbid(unsafe_code)]` — codec
DLL never runs on the host CPU, and the only `unsafe` boundary
other emulators have (mmap'd executable pages, JIT, longjmp)
doesn't exist here. **Provenance is not clean-room** — Microsoft's
API surface is public by design and explicitly licensable for
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

**Reproducible encode** — `Sandbox::with_rand_seed(u32)` (or `set_rand_seed` at runtime) seeds the sandbox-level `msvcrt!rand` LCG so codec calls that consult `rand`/`srand` are deterministic; default seed is 1 matching MSVC's pre-`srand` initial state. Two sandboxes seeded identically produce byte-identical encoded output. `mpg4c32.dll`'s VfW encode path does not currently consult `rand`, so the API is protection-only on this codec; any future codec that does will inherit deterministic behaviour automatically.

**Trace mode** — disabled by default behind a `trace` Cargo
feature (zero hot-path cost when off). When on, every memory
read/write to a watched range, every Win32 call (with arguments +
return value), and optionally every executed instruction emit
JSONL events. Schema documented in
`docs/winmf/winmf-emulator.md`. The reverse-engineering output is
the input format the project's
specifier→extractor→implementer round procedure consumes when
producing clean-room codec specs from scratch.

### Interactive debugger CLI — now `ud vfw` (univdreams)

The forensic debugger CLI that used to ship as `oxidetracevfw` has
moved to [`KarpelesLab/univdreams`](https://github.com/KarpelesLab/univdreams)
as `ud vfw {probe, decode, encode}`. univdreams' `ud-emulator` crate
is the upstream of this sandbox; `oxideav-vfw` is a thin Rust
adapter that registers ud-emulator-backed codecs into
`oxideav-core::CodecRegistry`. The full debugger surface
(per-instruction trace, memory watchpoints, PC breakpoints, GDB
Remote Serial Protocol server, JSONL trace sink, cascade-loaded
module-stub synthesis) is preserved one repo up. `cargo install
ud-cli` to use it.

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
| **`oxideav-videotoolbox`** | macOS | 🚧 H.264 + HEVC + ProRes + MJPEG + MPEG-2 + VP9 + MPEG-4 Pt 2 + AV1 (M3+) + VVC | 🚧 H.264 + HEVC + ProRes + MJPEG | Encoder knobs: bit_rate→AverageBitRate, quality→Quality, profile aliases (H.264 / HEVC main/main10/main4_2_2_10)→ProfileLevel; ProRes 6 fourCCs; data_rate_limits→DataRateLimits CFArray + constant_bit_rate→ConstantBitRate (macOS 13+). PSNR_Y: MPEG-2 ~61 / H.264 ~51 / HEVC ~54 / ProRes ~52 / MJPEG ~36 / AV1 ≥30 dB. |
| **`oxideav-audiotoolbox`** | macOS | 🚧 AAC LC + HE-AAC v1/v2 + AAC-LD/ELD + ALAC + iLBC + AMR-NB + AMR-WB + MP3 + FLAC + Opus | 🚧 AAC LC + HE-AAC v1/v2 + AAC-LD/ELD + ALAC + iLBC + FLAC + Opus | MP3 decode bit-exact ≈89.8 dB SNR; FLAC bit-exact 188 416/192 000 i16 @ 48k/2ch; ALAC S32 lossless contract (S16/S32 input, 24-bit output); Opus via `kAudioFormatOpus` (RFC 7845 OpusHead family 0/1/255 + RFC 6716 frame-duration mapping; ~26 dB SNR roundtrip). |
| **`oxideav-vaapi`** | Linux (Intel iGPU + AMD Radeon, via libva) | 🚧 H.264 | — stub | Codec id → VAProfile family map; `EntrypointMatrix` snapshot collapses per-device VLD/Enc capability probe FFI ~2×. Planned: HEVC + VP9 + AV1. |
| **`oxideav-vdpau`** | Linux (NVIDIA legacy / Nouveau) | 🚧 H.264 + HEVC + VP9 + MPEG-2 | — stub | Four `CodecInfo` entries (h264/hevc/vp9/mpeg2video) with `CodecCapabilities::video("<codec>_vdpau")` at priority 15 + max_size 8192² + container tags + libvdpau.so.1/libX11.so.6 pre-flight. |
| **`oxideav-nvidia`** | Cross-platform (NVENC + NVDEC) | 🚧 VP9 + AV1 + MPEG-2 | — | `Mpeg2NvDecoder` + MPEG-2 NVDEC factory (cuvidParser + `CudaVideoCodec::Mpeg2`); pre-flight `cuvidGetDecoderCaps` surfaces `Error::Unsupported` early → fallback to oxideav-mpeg12video; registered at priority 5 w/ QT/MP4 fourCC + Matroska codec-id. |
| **`oxideav-vulkan-video`** | Cross-platform (Vulkan VK_KHR_video_*) | 🚧 H.264 + HEVC + AV1 capability queries | — empty | HEVC + AV1 chained capability queries via `vkGetPhysicalDeviceVideoCapabilitiesKHR`; `sys.rs` adds StdVideo H.265 + AV1 type aliases + 4 sType discriminants + profile/anchor-level constants + 4 repr(C) Caps structs; `query_video_decode_h265_capabilities` (H.265 Main 8-bit 4:2:0) + `query_video_decode_av1_capabilities` (AV1 Main 8-bit 4:2:0). |

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
| **`oxideav-source`** | URI resolution + file reader + prefetching BufferedSource | ✅ `file://` + `mem://` + `data:` (RFC 2397) + `concat:` (mem://`/`data:`/`slice:` inner schemes) + `slice:<offset>+<length>!<inner>` byte-window + `FileScope` allow-list + `deny_dir` carve-outs + `file://` URI percent-decoding (RFC 3986 §2.1) |
| **`oxideav-http`** | HTTP / HTTPS source driver | ✅ `http://` + `https://` via pure-Rust `ureq` + `rustls` + `webpki-roots`; Range-request seeking; `HttpConfig` policy + RFC 7233 Content-Range/200-fallback/416 handling + RFC 9110 If-Range strong-validator + Content-Length cross-checks + HTTP-date 3 forms (IMF-fixdate/rfc850/asctime) + multipart/byteranges reject + Retry-After surfacing + RFC 7230 §3.2.4 obs-fold normaliser + `parse_headers` fuzz |
| **`oxideav-generator`** | Synthetic media source (`generate://...` URIs) + zero-input filters | ✅ audio synth (sine + chirp/FM/DTMF/multitone/ADSR/ringmod + 5-colour noise + `pwm` + `supersaw`/`saws` + `tremolo`/`trem`) + image (xc/gradient/pattern/fractal/plasma/noise/label + Perlin-2001 + Worley/cellular) + video (testsrc/smptebars/fractal_zoom/gradient_animate/zoneplate) |
| **`oxideav-rtmp`** | RTMP ingest + push | ✅ Server + client; AMF0/AMF3 parser/builder; Enhanced-RTMP v1 video + v2 audio + ModEx; pluggable key-verification; `rtmp://` PacketSource; symmetric teardown + client `poll_event` + v2 `MultichannelConfig` (24 SMPTE 22.2 positions) + Multitrack body + §E FLV file writer + `FlvReader<R: Write>` + NetConnection capability negotiation + §7.1.6 Aggregate Message routed end-to-end (`send_aggregate` + `next_packet` + `poll_event`) + ModEx TimestampOffsetNano (ns timebase) + typed `MessageStreamKind` accessor + §5 protocol-control invariant validator |
| **`oxideav-sysaudio`** | Native audio output | ✅ Runtime-loaded backends (ALSA, PulseAudio, WASAPI, CoreAudio, OSS); CoreAudio + WASAPI real HAL latency; output-device enumeration; per-device routing API on all backends; `StreamRequest::buffer_frames` honoured; `Driver::preferred_format` introspection on WASAPI/CoreAudio/ALSA; functional OSS `/dev/dsp` via dlopen'd libc (S16_LE negotiation). BT-aware; falls back to software estimate if HAL unavailable. |
| **`oxideav-pipeline`** | Pipeline composition (source → transforms → sink) | ✅ JSON transcode-graph executor; pipelined multithreaded runtime + `Executor::with_channel_caps` + `with_max_queue_bytes` byte-ceiling + `Progress::elapsed_micros` + `packets_skipped` + `packets_read` (demuxer-cumulative; wedged-decoder signature) + EOF Progress retry ride-out + `Progress::packets_copied` sink-cumulative (source-vs-sink lag = one subtraction) |
| **`oxideav-scene`** | Time-based scene / composition model | 🚧 data model for PDF pages / RTMP streaming compositor / NLE timelines + per-frame `Sample` + animation-track composition + `RasterRenderer` (bg solid/gradient + Rect/Polygon + `ObjectKind::Vector`) + `ObjectKind::Group` nested + SVG 1.1 path-data (M/L/H/V/C/S/Q/T/Z + relative + A arc) + `ObjectKind::Image(Decoded)` RGBA8 + `Background::DecodedImage(Arc<VideoFrame>)` + audio-cue mixing into `RenderedFrame.audio` |
| **`oxideav-audio-filter`** | Audio effects & conversions (streaming) | ✅ ~50 filters: classic + transient/spatial/restoration family + SlewLimiter + LR4 crossover + `true_peak_detector` + `state_variable` Chamberlin SVF + Criterion benchmark harness (7 scenarios) + `crest_factor_meter` + `stereo_correlation_meter` (Pearson coefficient, sliding-window) — see crate README for the catalogue |
| **`oxideav-image-filter`** | Single-frame image effects (stateless) | ✅ 130 filter types / 178 factory names — Gabor + Niblack adaptive local-statistics threshold + `CurveInterpolation::NaturalCubic` + `CentripetalCatmullRom` + `ChordalCatmullRom` (α=1) + `ReinhardExtended` tone-map — see crate README for the catalogue |
| **`oxideav-pixfmt`** | Pixel-format conversion + palette + dither | ✅ YUV↔RGB matrices (BT.601 / BT.709 / BT.2020 / BT.2100) + chroma subsampling + packed 4:2:2 (YUYV / UYVY) ↔ planar/RGB/RGBA + palette quantisation + Floyd-Steinberg dither + PQ + HLG + BT.1886 transfer functions + Porter-Duff alpha + `Ya8` (luma+alpha) + direct `NV12`/`NV21` ↔ `Rgb24`/`Rgba` + direct planar YUV ↔ planar YUV chroma resample (4:2:0/4:2:2/4:4:4 incl. JPEG full-range) + Criterion alpha bench |

</details>

<details>
<summary><strong>Subtitles</strong> (click to expand)</summary>

All text formats parse to a unified IR (`SubtitleCue` with rich-text
`Segment`s: bold / italic / underline / strike / color / font / voice /
class / karaoke / timestamp / raw) so cross-format conversion preserves
as much styling as each pair can represent. Bitmap-native formats (PGS,
DVB, VobSub) decode directly to `Frame::Video(Rgba)`. All text parsers
tolerate UTF-8 / UTF-16 LE / UTF-16 BE BOMs and CRLF / LF / lone-CR
line endings.

**Text formats** — in `oxideav-subtitle`:

| Format              | Decode | Encode | Notes |
|---------------------|:------:|:------:|-------|
| **SRT** (SubRip)    | ✅ | ✅ | `<b>/<i>/<u>/<s>`, `<font color>` hex + 17 named, `<font face size>` + structural tolerance (PEM preamble + duplicate-index + whitespace-only continuation lines) |
| **WebVTT**          | ✅ | ✅ | Header, STYLE ::cue(.class), REGION, inline b/i/u/c/v/lang/ruby/timestamp + cue-settings round-trip + full REGION block + §4.1 NOTE comment-block round-trip + §3.4 cue identifier round-trip via `vtt_cue_id.<idx>` metadata + §4.1/§3.3 strict signature + canonical timestamp enforcement |
| **MicroDVD**        | ✅ | ✅ | frame-based, `{y:b/i/u/s}`, `{c:$BBGGRR}`, `{f:family}` |
| **MPL2**            | ✅ | ✅ | decisecond timing, `/` italic, `\|` break |
| **MPsub**           | ✅ | ✅ | relative-start timing, `FORMAT=TIME`, `TITLE=`/`AUTHOR=` |
| **VPlayer**         | ✅ | ✅ | `HH:MM:SS:text`, end inferred |
| **PJS**             | ✅ | ✅ | frame-based, quoted body |
| **AQTitle**         | ✅ | ✅ | `-->> N` frame markers |
| **JACOsub**         | ✅ | ✅ | `\B/\I/\U`, `#TITLE`/`#TIMERES` headers |
| **RealText**        | ✅ | ✅ | HTML-like `<time>/<b>/<i>/<u>/<font>/<br/>` |
| **SubViewer 1/2**   | ✅ | ✅ | marker-based v1, `[INFORMATION]` header v2 |
| **TTML**            | ✅ | ✅ | W3C Timed Text, `<tt>/<head>/<styling>/<style>/<p>/<span>/<br/>`, tts:* styling + r171 IMSC 1.2: `<layout>` regions + `tts:textAlign` + 22 IR-unmodelled `tts:*` / `itts:*` style extras + 11 `ttp:*` / `ittp:*` parameter attrs + `HH:MM:SS:FF` / `<n>f` / `<n>t` against `ttp:frameRate` / `ttp:tickRate` |
| **SAMI**            | ✅ | ✅ | Microsoft, `<SYNC Start=ms>` + `<STYLE>` CSS classes |
| **EBU STL**         | ✅ | ✅ | ISO/IEC 18041 binary GSI+TTI (text mode only; bitmap + colour variants deferred) |

**Advanced text (own crate)** — `oxideav-ass`:

| Format              | Decode | Encode | Notes |
|---------------------|:------:|:------:|-------|
| **ASS / SSA**       | ✅ | ✅ | Script Info + V4+/V4 Styles (BGR+inv-alpha) + override tags + `\fn`/`\fe`/`\b<weight>`/`\r[<style>]` + `\pbo` + face-flag toggles + typed `\p<scale>` + `\fax`/`\fay` shear + `\an<n>` numpad alignment + `\1a` primary-fill alpha + `\blur<strength>` Gaussian post-step + `\iclip(rect)` + `\iclip(drawing)` + `\fsp<spacing>` letter-spacing baked into renderer |

**Bitmap-native (own crate)** — `oxideav-sub-image`:

| Format              | Decode | Encode | Notes |
|---------------------|:------:|:------:|-------|
| **PGS / HDMV** (`.sup`) | ✅ | ✅ | Blu-ray subtitle stream; PCS/WDS/PDS/ODS + RLE + YCbCr palette → RGBA + RLE codec property+negative sweep + PCS composition_state classified + routed to Packet keyframe flag |
| **DVB subtitles**   | ✅ | — | ETSI EN 300 743 segments + 2/4/8-bit pixel-coded objects |
| **VobSub** (`.idx`+`.sub`) | ✅ | — | DVD SPU with control commands + RLE + 16-colour palette + SP_DCSQ 0x07 CHG_COLCON length-skip + CHG_COLCON application (typed bands + per-pixel replacements during canvas paint) + per-DCSQ STM latching + FSTA_DSP forced-display surfacing |

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
and per-frame unsync, extended header with **CRC-32 [ISO-3309]
verification and emission** since r153, v2.4 data-length indicator,
encrypted/compressed frames recorded as `Unknown`, **r161 v2.4 §3.4
footer emission + strict trailer-validation on read** composable with
whole-tag/per-frame unsync + extended-header CRC) plus the legacy
128-byte ID3v1 trailer. Text frames (T\*, TXXX), URLs (W\*, WXXX),
COMM / USLT, and APIC / PIC picture frames are handled structurally;
less-common frames (SYLT, RGAD/RVA2, PRIV, GEOB, UFID, POPM, MCDI,
…) survive as `Unknown` with their raw bytes available.

The `oxideav-flac` container surfaces the extracted
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
