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
  MIDI File + soft-synth + UMP/MIDI 2.0 packet container & protocol), `oxideav-pbm` (Netpbm: PBM/PGM/PPM/PNM/PAM),
  `oxideav-nsf` (NES Sound Format — 6502 emu + 2A03 APU); image-format
  bootstrap wave: `oxideav-dds`, `oxideav-openexr`, `oxideav-farbfeld`,
  `oxideav-hdr` (Radiance RGBE), `oxideav-qoi`, `oxideav-tga`,
  `oxideav-icer` (JPL Mars-rover), `oxideav-wbmp`, `oxideav-pcx`,
  `oxideav-pict` (Apple QuickDraw); `oxideav-iff` extended with ILBM.
  AVIF decodes end-to-end via `oxideav-av1` (pixel fidelity tracks the
  AV1 intra decoder).
- **Vector graphics + text** — `oxideav-svg` (read+write SVG; rounds 1-3
  ship full shape set + text/filters/masks/clipPath + use/symbol + svgz +
  animate/set@t=0), `oxideav-pdf` (multi-page writer + Scene
  metadata via `/Info` dict; reader: bytes → Scene with xref +
  FlateDecode + content-stream operator parser + r35 inline-image
  extraction (ISO 32000-1 §8.9.7 BI/ID/EI framing)), `oxideav-raster`
  (vector→raster rendering kernel — scanline AA, bilinear/Lanczos2/Lanczos3 + Mitchell/Catmull-Rom/B-spline cubic image resampling,
  trapezoidal coverage, soft masks, patterns, filter primitives, ICC
  pipeline, bitmap cache keyed by `Group::cache_key`, SVG2 stroke-linejoin miter-clip/arcs §13.5.5), `oxideav-ttf`
  (TrueType parser — cmap 0/4/6/12/14 incl. Variation Sequences, GSUB
  ligatures, GPOS kerning + per-script feature selection + coherent `Font::shape()` engine (all GSUB+GPOS lookup types, IGNORE_MARKS-aware, Arabic joining + mark attachment validated) + v1.1 FeatureVariations, COLR + CPAL + sbix tables, TTC subfont
  selection, AGL glyph-name→Unicode, full `name`-table accessor API + gvar IUP inferred-delta variable-glyph interpolation + set-axis-by-tag/named-instance API), `oxideav-otf` (CFF / Type 2 charstrings incl. CID-keyed ROS/FDArray/FDSelect + arithmetic/stack/storage/conditional ops + Top-DICT FontMatrix/PaintType/CharstringType/StrokeWidth, ISOAdobe/Expert/ExpertSubset predefined charsets, cubic outlines; r222 GDEF + Coverage + ClassDef common-layout primitives + `GlyphClass` enum + GPOS ValueRecord/ValueFormat + Lookup Type 1 single-adjustment + CFF2 §12 ItemVariationStore for variable fonts; r352 GPOS Lookup Types 1-9 + GSUB Types 1-7 incl. mark-to-ligature + contextual/chained via shared module),
  `oxideav-scribe` (shaper with vector-first `Shaper::shape_to_paths`
  API — no rasterizer dep; trapezoidal horizontal AA, GPOS mark-to-mark,
  COLR/CBDT colour glyphs via raster bilinear/composer; bidi UAX #9
  data-complete at Unicode 16.0 — Bidi_Class ranges + bracket pairs +
  mirror table; USE still future work).
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
| WAV       | ✅ | ✅ | ✅ | Full metadata-chunk family (BWF bext, LIST/INFO, iXML, smpl/inst, Acidizer acid, MCI cue/plst/adtl, ADM chna + BW64 axml/bxml/sxml XML carriers) + RF64/BW64 64-bit form (read + write; ds64/JUNK on-the-fly promotion) |
| FLAC      | ✅ | ✅ | ✅ | All metadata blocks (VORBIS_COMMENT / PICTURE / CUESHEET / SEEKTABLE) + §8 typed whole-chain parse/write; encoder five-window default apodization + compression-quality regression guards (−5% on 7.1/24-bit, bit-exact); decode verify-decoder MD5 (§8.2); muxer SEEKTABLE generation (§8.5, configurable density) |
| Ogg       | ✅ | ✅ | ✅ | Vorbis/Opus/Theora/Speex + chained streams + page-bisection seek + Skeleton 3.0/4.0 read AND write incl. keyframe-index fast-path seek + granulepos→playback-time mapping + §4 mixed grouping+chaining (unique-serial enforced) + nil-page + multi-page packet reassembly coverage + Skeleton-4.0 fishead time anchors (basetime/presentation start_time, chained segment-length index check) + Opus pre-skip granule semantics (RFC 7845 §4.3, seek-bisection axis) + FLAC-in-Ogg header-packet count + Vorbis-comment (RFC 9639 §10.1) |
| Matroska  | ✅ | ✅ | ✅ | MKV/MKA/MKS; Cues read/write-symmetric seek (CueBlockNumber/CueDuration) + SeekHead/Chapters/Attachments + lacing + CRC-32 + typed RFC 9559 element surface (Tags, Colour/HDR mastering, Projection, BlockAdditions read+write, TrackOperation read+write, ContentEncryption signing quartet read+write, TrackTranslate read+write, …) + full BlockGroup semantics (ReferenceBlock/Priority/CodecState/DiscardPadding) + SilentTracks + complete Chapters edition/atom tree incl. ChapterProcess — all read+write; Tags mux (Targets + recursive SimpleTag §5.1.8) symmetric with demux; EBML walker property-fuzzed + typed TrackIdentity (Name/Language/LanguageBCP47/CodecName/Flag{Enabled,Default,Lacing}/AttachmentLink) demux↔mux |
| WebM      | ✅ | ✅ | ✅ | First-class: separate fourcc, codec whitelist (VP8/VP9/AV1/Vorbis/Opus); inherits Matroska Cues seek |
| MP4       | ✅ | ✅ | ✅ | mp4/ismv; faststart + iTunes ilst + fragmented demux/mux (DASH/HLS/CMAF) + sidx/mfra + broad typed box-accessor surface + CENC AES-128 CTR/CBC decryption (all 4 schemes) + amve ambient-viewing-environment HDR metadata + btrt bit-rate box (buffer/max/avg on all sample entries) + prft producer-reference-time box (2022-edition NTP-flag annotations) + typed §10 sample-group description entries (roll/prol/rap/tele/sap/alst/rash) + §8.16.4 ssix subsegment-index emission (after each sidx) + §8.7.3 leva level-assignment write symmetry + §8.15.4.2 stvi StereoVideoBox demux+mux + §8.8.16 assp/trep fragmented-mux emission |
| MOV (QuickTime) | ✅ | — | ✅ | QTFF + ISO BMFF meta + HEIF/HEIC item properties + fragmented-MP4 seek + edit-list mapping (+ muxer edts/elst emission) + `cmov` compressed-movie decompression + §8.14 sub-track groups + §8.7.8/§8.7.9 saiz/saio + §8.6.1.3 ctts composition-offset muxer write + sound sample-description v0/v1 (fixed-ratio + VBR `-2`) + §8.10.1 udta movie/track metadata muxer write + tmcd timecode-track sample-data decode (start_timecode) + §12.3.3 timed-metadata sample entries (metx/mett/urim + txtC + btrt) + §12.4.2 hint-track hmhd + §12.6.3 subtitle sample entries (stpp/sbtt) + §8.9.3 typed sample-group description entries (tele/sap/rash/alst) with per-sample lookups + QuickTime Text + ISO BMFF stxt timed-text sample entries; ffprobe-accepted |
| AVI       | ✅ | ✅ | ✅ | AVI 1.0 + OpenDML 2.0; interlaced + VBR audio + LIST INFO + WAVEFORMATEXTENSIBLE + ODML keyframe seek + per-packet keyframe flags + idx1 `rec ` LIST entries round-trip + avih.dwReserved[4] reserved-array accessor + vprp typed VideoFormatToken/VideoStandard accessors + indexed-DIB baseline colour table (RGBQUAD bmiColors[]) + xxpc effective-palette resolution + OpenDML AVIMETAINDEX typed bIndexType (super/std-index) + non-conformant reserved-field diagnostics |
| Blu-ray (BD-ROM) | ✅ | — | — | UDF 2.50 + BDMV + `.m2ts` + `bluray://`; playlists / chapters / multi-angle + EP_map keyframe seek + AACS hook + HDMV nav title-engine (index.bdmv→MOBJ; inter-title Jump/Call/Resume, PSR4 seed) + PGS subtitle-segment parser (PCS/WDS/PDS/ODS + RLE) + Display-Set grouping + multi-ODS fragment reassembly + PGS renderer (palette resolution + window compositing) + HDMV navigation-command opcode decode + PSR/GPR register model + HDMV VM execution (Set/Compare/Branch interpreter + Movie-Object runner over Jump/Call/Resume); lacks IG button-state machine + BD-J |
| DVD-Video | ✅ | — | — | ISO 9660 + UDF 1.02 + IFO/VOB + `dvd://`; navigation VM (incl. PCI NSML_AGLI non-seamless angle jump) + SPU subpictures + RGBA compositor + time seek + VOB → MKV + DTS core frame-header decode + generic audio-substream header (FrmCnt/FirstAccUnit + access-unit offset) + PCI_GI vobu_isrc/c_eltm decode + PCI RECI raw-region capture + 16/20/24-bit LPCM width (bytes_per_sample ratio) + DSI nav-pack typed accessors (VOBU_SRI/SYNCI/SML_PBI); lacks CSS auth |
| MP3       | ✅ | — | ✅ | ID3v2/v1 + Xing/Info VBR + CBR/VBR seek; stereo decode via oxideav-mp3 |
| IFF (EA IFF 85) | ✅ | ✅ | — | `FORM/LIST/CAT` family — Amiga 8SVX + ILBM (EHB/HAM, palette-change chunks) + ANIM op-0/1/2/3/4/5/7/8 + true-colour FORM RGB8/RGBN/DEEP decode + encode (genlock-RLE + TVDC chunky) + Apple AIFF/AIFF-C + fuzz harness |
| IVF       | ✅ | — | — | VP8 elementary stream container |
| MPEG-TS   | ✅ | — | — | ISO/IEC 13818-1 transport stream — full Table 2-34 stream_type mapping (52 named) + DVB PMT ES descriptors (stream_identifier/teletext/subtitling/AC-3/E-AC-3/DTS) + per-PID 33-bit PTS/DTS unwrap; packet/PSI/descriptor walk (PAT/CAT/PMT/TSDT — all four 13818-1 PSI tables + DVB SDT service_descriptor + DVB EIT (present/following + schedule, EN 300 468 §5.2.4) with short + extended (§6.2.15 tag 0x4E) event descriptors + DVB NIT (network_name_descriptor, EN 300 468 §5.2.1) + DVB BAT (§5.2.2 + bouquet_name_descriptor) + DVB RST running-status (§5.2.7)); Table 2-17 PES header fully decoded incl. PES_extension body (private data, pack_header, packet-sequence counter, P-STD buffer); muxer: PSI fragmentation §2.4.4 + ISO_639_language_descriptor §2.6.18 + periodic PCR §2.7.2 |
| AMV       | ✅ | ✅ | — | Chinese MP4-player format — demuxer + muxer + seek + strict-mode validators + symmetric demux→pixels/demux→PCM conveniences + S16-mono audio stream params + §4b IMA/DVI-ADPCM audio decode (1116 blocks → 93.0 s; decode_audio_payload convenience + PCM ffprobe-validated) + §4a in-crate baseline-JPEG video decode to RGB (device-hardcoded quant/Huffman tables) + fuzz harness |
| FLV       | ✅ | ✅ | — | MP3/AAC/H.264 audio + VP6/H.264 video + Enhanced-RTMP extensions (incl. v2 audio-silence discard) + AMF0 metadata + multitrack + HDR colorInfo + fuzz; muxer covers tags / seek-table / cue-points / multitrack join + AMF0 Date (SCRIPTDATADATE) write + AMF3 value encoder + Enhanced-RTMP multichannel-config writer + fuzz |
| WebP      | ✅ | ✅ | — | RIFF/WEBP (lossy + lossless + animation; ANIM + ANMF emit) + §4.4 per-bundle inverse_color_indexing hoist |
| TIFF      | ✅ | ✅ | — | TIFF 6.0 single-image + BigTIFF + PhotometricInterpretation=5/8 CMYK + CIE L*a*b* decode/encode + CCITT T.4 2-D + T.6 (Group 4) fax decode/encode + tiled-image layout + float Predictor=3 (IEEE 16/32/64-bit gray+RGB, strip/tile/planar) + tiled JPEG-in-TIFF (Compression=7) + 4:4:4 YCbCr planar+predictor + planar CMYK coverage |
| PNG / APNG| ✅ | ✅ | — | 8 + 16-bit, all color types, APNG + gAMA/cHRM/zTXt + tRNS round-trip (typed Grayscale/Rgb/Palette; ct=4/6 rejected); region-aware APNG encoder (per-frame offset + delay + dispose/blend, fuzz-hardened) + sRGB linear-light colour management + bKGD §13.15 background compositing; metadata lacks only iCCP/iTXt |
| GIF       | ✅ | ✅ | — | 87a/89a + LZW + animation + NETSCAPE loop + disposal compositor + typed extension accessors + truecolor RGBA encode (median-cut + nearest-entry remap + Floyd–Steinberg dither + shared-palette animations) |
| JPEG      | ✅ | ✅ | — | Still-image wrapper around the MJPEG codec |
| BMP       | ✅ | ✅ | — | Windows bitmap — DIB headers BITMAPINFOHEADER / V4 / V5, 1/4/8/16/24/32-bit + explicit-mask BI_BITFIELDS / BI_ALPHABITFIELDS V3 encoder (5 presets, 32-bpp lossless) + OS/2 file-magic recognise-reject (BA/CI/CP/IC/PT); also exposes the DIB helpers used by ICO / CUR sub-images |
| Netpbm    | ✅ | ✅ | — | All seven PNM magics + PAM; 1/8/16-bit; ASCII + binary fast paths (up to ~50 GiB/s) |
| ICO / CUR | ✅ | ✅ | — | Windows icon + cursor — multi-resolution, BMP and PNG sub-images; body-dim `(0,256]` reject + CUR hotspot body-derived bound + dir wBitCount vs body biBitCount cross-check + ANI (RIFF/ACON) framework Demuxer (anih + seq/rate timeline → packet stream) |
| slin      | ✅ | ✅ | — | Asterisk raw-PCM: .sln/.slin/.sln8..192 |
| MOD / S3M / STM | ✅ | — | — | Tracker modules (decode-only by design) — see Trackers table |

Cross-container remux works for any pair whose codecs don't require
rewriting (FLAC ↔ MKV, Ogg ↔ MKV, MP4 ↔ MOV, etc.).

### Content protection

| Layer | Status | Notes |
|-------|:-------|-------|
| AACS  | ✅ Common 0.953 + BD-Prerecorded 0.953 | `oxideav-aacs` clean-room — full key-derivation chain (Device Key → VUK), Aligned-Unit decryption, SCSI MMC drive layer + Drive-Host AKE, MKB (incl. Type-4 verify-precursor/KCD Media-Key resolution)/Content-Certificate/CRL verification + GET CONFIGURATION / AACS Feature Descriptor host capability discovery. Lacks AACS 2.0 |

</details>

### Codecs

> Each row below is a current-state summary. For round-by-round history, design notes, and per-feature trade-offs, see the per-crate `README.md` and `CHANGELOG.md` in `crates/oxideav-<codec>/`.

<details>
<summary><strong>Audio</strong> (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **PCM** (s8/16/24/32/f32/f64) | ✅ 100% | ✅ 100% |
| **slin** (Asterisk raw PCM) | ✅ 100% | ✅ 100% |
| **FLAC** | ✅ 100% | ✅ 100% |
| **Vorbis** | 🚧 ~86% (post-2026-05-20 orphan) — headers + codebooks + floor 0/1 + residue 0/1/2 + channel coupling + IMDCT + floor-0 LSP + residue format-0 strided-scatter decode-path coverage + streaming overlap-add + §4.3.9 channel-order layout (1–8ch) + sample-exact PCM on all 12 fixtures (mono/stereo/5.1, q−1..q10, 22–96 kHz) via §3.2.1 canonical-Huffman lowest-free-codeword fix + all 17 fixtures exercised (chained-Ogg de-framing + metadata headers) + §4.3.1 malformed-packet recovery; lacks floor-0 fixture + Ogg/registration wiring | 🚧 ~45% — first END-TO-END PCM↔packet time-domain round-trip (~45 dB) + every setup-header + audio-packet body WRITE primitive (forward MDCT/windowing, coupling inverse, §6.2.2 floor 0 packet-body write, residue classification + VQ codeword packing) + §3.2.1 VQ-encode quantiser (nearest-codeword search) + §8.6.2 residue VQ-encode cascade planner + §6.2.2 floor-0 VQ-encode glue (coefficient→entry run) + §4.3 full audio-packet write driver + §7.2.4 floor-1 envelope→posts inverse (dB-table inverse + forward-MDCT analysis) → first END-TO-END encode→decode spectral round-trip (post-exact + ≥20 dB dB-index SNR) + §7.2.4 floor-1 render_curve (encoder-side curve synthesis) + non-flat floor-1 PCM round-trip ≥35 dB (residue-against-rendered-floor, ~44 dB) + §6.2.3 floor-0 render_curve; lacks floor-0 curve→LSP inversion (spec defines floor-0 decode only) + residue-from-spectrum VQ |
| **Opus** | 🚧 ~47% — §4.3.6→§4.3.7.2 CELT synthesis backend composed end-to-end (celt_synthesis: denormalise→IMDCT→OLA→de-emphasis) + RFC 6716 range decoder + full SILK pipeline + CELT side through coarse energy (per-LM inter α,β), allocation search, PVQ codebook + spreading + framing incl. Appendix-B self-delimiting + §4.3.7 inverse-MDCT transform core + weighted overlap-add + §4.3.4.1 pulse-cost cache (unblocks PVQ reallocation) + §4.3.6 band denormalisation + §3/§4 top-level `OpusDecoder::decode_packet` orchestration (TOC parse + frame split + mode routing) + mono SILK-only decode END-TO-END to real PCM (§4.2.7.9 LTP/LPC synthesis + §4.2.9 resample + §4.5.2 cross-packet reset) + §4.2.8 stereo SILK unmixing (stereo SILK decodes END-TO-END to PCM); + §4.3.2.1 Laplace symbol decode + Table-56 frame-prefix decode + CELT-only mono silence frame decodes END-TO-END to PCM (CeltSilence path) + §4.3.2.1 CELT coarse-energy reconstruction recurrence (non-silent CELT-only front half decodes coarse energy from the range coder) + SILK fixture-validated end-to-end (3 reference-encoder streams: NB-mono/WB-stereo/MB-60ms — §3.1 TOC routing + 48 kHz sample accounting + 440 Hz Goertzel dominance); lacks §4.3.3 allocation + §4.3.4 PVQ shapes + §4.3.2.2 fine energy for real PCM | 🚧 ~5% — scaffold |
| **MP1 / MP2** | ✅ ~99% — Layer I + II decode (PCM-bit-exact) + CRC-16 + free-format probe + ISO 13818-3 LSF | 🚧 ~90% — Layer I + II encoders end-to-end + Annex D Model-2 Table D.3a 32 kHz partition table (49 rows) + §C.1.5.2.5/Table C.4 perceptual SCFSI selection + complete Annex D Model-2 numeric table set (D.1a-c threshold-in-quiet + D.3a-c calc-partition + spreading operators + D.4a-c per-FFT-line absolute threshold, all rates); Model-2 allocator (FFT + clause D.2.4 combination + SMR) now wired into the Layer I encoder; Layer II Model-2 wiring + D.1d-f LTq still pending |
| **MP2** | 🚧 ~58% (post-2026-05-24 orphan) — Layer II header/sizing + requantizer + joint-stereo + scfsi + LSF + §2.4.1.6 intensity-stereo sample sharing + full §2.4.2.1 bit-allocation + §2.4.3 requant + §2.4.3.2 polyphase synthesis filterbank → PCM validated END-TO-END across the full Layer II mode×rate matrix (MPEG-1 mono/stereo 32/44.1/48 kHz + MPEG-2 LSF 16/22.05/24 kHz, per-frame) to ISO conformance bound (max abs ≤1 LSB) | 🚧 ~44% — multi-rate encode→decode round-trip + batch encode_all_frames + SCFSI + bit-allocator + quantizer + `encode_frame` + Annex D Model-1 chain through Step 9 incl. §D.1 Step 3/5(a) + Annex D LTq tables (D.1d/e/f) + §D.2 Model-2 calc-partition Tables D.3a/D.3b/D.3c (32/44.1/48 kHz) + spreading + §D.2.4 threshold loop g…n (32 kHz SMR) + §D.2.4 step-(l) absolute-threshold tables D.4a/b/c (32/44.1/48 kHz) + §D.1 Model-1 SMR driver (Step 1..9 → per-subband SMR_sb) wired into §2.4.2.1 `encode_frame` bit allocation (auto-SMR psychoacoustic-driven encode across all MPEG-1 rates + LSF fallback) |
| **MP3** | ✅ ~99% — bit-exact decode + free-format frames + ID3v2/Xing seek + 13818-3 Table B.2 LSF bands + full MPEG-2.5 (8/11.025/12 kHz; 11.025 kHz validated vs reference PCM) | 🚧 ~99.8% — full Layer III + joint stereo + MPEG-2 LSF/MPEG-2.5 encode (CBR/VBR/MS/CRC) + §2.4.3.2 LSF + §2.4.3.4.9.3 short-block (per-window) intensity-stereo incl. auto-MS + short + intensity combined + auto-block-type short granules with intensity (incl. intensity-only non-MS) + §C.1.5.2 LSF/MPEG-2.5 auto block-type + §C.1.5.3.2.1 Model-2-driven (pe>1800) block-type switching + §C.1.5.3 scfsi scalefactor-selection-info + §C.1.5.4.4.6 band-aligned SUBDIVIDE + Model 2 psychoacoustic threshold in the outer loop + named quality presets (Transparent/High/Standard/Fast, Model-2-driven at 32/44.1/48 kHz); lacks only MPEG-2.5 band tables (docs ask) |
| **AAC** | 🚧 ~70% — §4.6.16.2 RVLC error-resilient scalefactor coding + registered runtime `Decoder` trait (ADTS-framed; MP4-OTI-0x40 / WAVE-0x00FF·0x1601 / `mp4a`·`aac ` / `A_AAC` tags) + HE-AAC v1 base-layer + §4.6 block-order decode driver (SCE/CPE → PCM: §4.6.1.3/§4.6.2.3.3 dequant + §4.6.11 IMDCT/sine-KBD filterbank + overlap-add TDAC + §4.6.8.1 M/S + §4.6.8.2 intensity + §4.6.13 PNS + §4.6.9 TNS + §4.6.7 long-window LTP synthesis + §4.6.6 Main frequency-domain predictor (AOT 1, long windows)) + `decode::StreamDecoder` + §4.6.11 s16 PCM render (round-half-away + saturate + interleave): PNS-free fixtures byte-exact within 1 LSB, PNS fixtures <0.1% RMS vs reference + §4.6.18.3.2 SBR frequency-band tables (fMaster/high/low/noise) + §1.8.4.5 EP-tool CRC generator (CRC4-32, LATM crcCheckSum) + §4.4.2.8/§4.6.18.3.5 SBR (HE-AAC v1) bitstream/side-info decode (Tables 4.A.79–4.A.88 Huffman + sbr_header/grid/dtdf/invf/envelope/noise + envelope/noise DPCM reconstruction, all ISO-spec-sourced) + §1.7 LATM/LOAS transport framing (StreamMuxConfig/AudioMuxElement/AudioSyncStream) + LATM/LOAS→PCM decode driver (per-streamID AU decode + runtime carrier auto-detect); lacks byte-exact PCM (PNS RNG-phase) + SSR + short-window LTP + SBR QMF/HF back-end | 🚧 ~20% — Phase-2 writers for every syntax element + TNS decode tool; SBR pending QMF |
| **CELT** | 🚧 ~35% (post-2026-05-20 orphan) — end-to-end mono+stereo long-MDCT frame decode → PCM (`decode_celt_frame` + stereo per-channel coarse-energy → interleaved PCM) + range decoder + coarse-energy decode COMPLETE (RFC 6716 Appendix-A carve-out) + full allocation chain + PVQ codebook/spreading/split geometry + IMDCT/WOLA synthesis primitives + §4.3 frame-prefix decode driver (Table 56 walk to the fine-energy boundary) + §4.3.4.1 bit-exact pulse-cost cache (unblocks §4.3.3 reallocation) + §4.3.4 multi-band residual decode loop (decode_residual_bands) + §4.3.3 combine_band_allocation (static+boost+trim clamped to caps) + §4.3.3 combined-candidate allocation column search (cap-clamped bisection) + §4.3.7 long-MDCT synthesis spine + §4.3.6→§4.3.7 two-channel stereo synthesis chain (per-channel IMDCT/WOLA → interleaved L/R PCM) + §4.3.3/§4.3.4.1 documented alloc→pulses seam (allocation_to_pulses gate) + caller-input-free mono decode (decode_celt_frame_auto derives band pulse counts from the range coder); lacks interp_bits2pulses fine/shape split + §4.3.4.4 itheta mid/side + short-block reassembly (docs-gapped); 0.2.0 release pending pin sweep (#1648) | 🚧 ~6% — PVQ encode chain (search §5.3.8.1 + index encode §4.3.4.2, decoder round-trip) |
| **Speex** | 🚧 ~62% — §10.1 WB high-band per-sub-frame LSP interpolation + forced open-loop pitch-gain reconstruction into NB decode (modes 1/8) + closed NB decode loop (LSP→LPC + §8.4 e[n]=p[n]+c[n] excitation feedback into the adaptive codebook + synthesis filter → full-frame PCM) + log-domain excitation-gain grid + WB sub-band decode loop (embedded NB+HB §10.4 → both half-band signals) + top-level `SpeexDecoder` packet walk (multi-frame, mixed NB/WB) + UWB framing recursion + LSP base-vector / Q-format pin (NB `.25·i+.25` / HB `.3125·i+.75` rad) + `LSP_MARGIN` min-spacing clamp → LSP set bounded inside `(0,π)` by construction (always-stable filter, validated non-divergent on real q8 fixture); lacks QMF synthesis recombination + mode-4 HB codebook (both docs-gapped) | 🚧 ~5% — scaffold |
| **GSM 06.10** | 🚧 ~91% — clean-room §5.3 RPE-LTP decode + §4.4 homing + §1.7 unpack + 06.12 §6.1 receive-side comfort-noise (DTX silence-descriptor synthesis) + §6.2/§6.3.3.1 SEQ06H homing-frame conformance harness + Table 6.5/6.7 boundary pins | 🚧 ~95% — full §5.2 encode + §1.7 packer + §4.3 encoder homing (Table 4.1a/b bit-exact pin) + 06.12 §5.1/§5.2 transmit DTX comfort-noise (N=4 unquantised LAR/xmax averaging → SID parameter encoding); lacks bulk SEQ01-05 conformance corpus (unstaged ETSI ZIP) + §5.2 SID bit-packing (GSM 05.03 unstaged) |
| **G.711** (μ/A-law) | ✅ 100% | ✅ 100% |
| **G.722** | 🚧 ~86% — Table II-3 overflow transmit conformance + SB-ADPCM decoder + QMF + auxiliary-data channel + clause-2 transmission-characteristics conformance masks + Recommendation pseudo-code conformance vectors + Appendix-II.3.2 bit-exact RL#/RH# golden vectors (512-window per mode + full-16384-sample checksum) + Table II-2 Config-1 segment structure + bit-exact DcZero transmit anchor | 🚧 ~82% — SB-ADPCM encoder (transmit-QMF normalisation spec-corrected) + Mode 2/3 round-trip + Appendix-II test-sequence harness; lacks ITU disk-only Config-1 conformance corpus |
| **G.723.1** | ✅ 100% | ✅ 100% — both 5.3k + 6.3k; fuzz-hardened (4 targets) |
| **G.728** | 🚧 ~92% — LD-CELP decode: Annex A-D + Levinson + postfilter chain + ICOUNT=3 update stagger + Annex I §I.4.1 frame-erasure excitation extrapolation + §I.4.2 LPC softening + §I.4.5 gain-growth limit + Annex G §G.1.2/§G.1.3 fixed-point arith primitives + §G.2.1 reformulated backward vector gain-adapter (dB-table lookups) + §G.2.2 fixed-point variable-precision Levinson-Durbin recursion (SIMPDIV + fresh/resume) + §G.3.11 block-32 fixed-point synthesis filter + §G.3.20–23 fixed-point adaptive postfilter | 🚧 ~89% — analysis-by-synthesis loop complete, bit-exact lockstep with decoder incl. ICOUNT stagger + Annex G §G.3 fixed-point coder (blocks 11-21 codebook search/excitation); lacks §G.3 backward-adaptation + postfilter-coeff fixed-point blocks |
| **G.729** | 🚧 ~40% — tables + serial parser + full §4.1 per-frame parameter chain (LSP / pitch / FCB / gains incl. parity concealment) + §4.1.6 LP synthesis (adaptive/fixed excitation → first reconstructed-speech PCM) over 18 222 conformance frames + §4.2.2 short-term postfilter H_f(z) + §4.2.3 tilt compensation + §4.2.4 adaptive gain control + §4.2.1 long-term postfilter (integer-delay eqs 78–83) + §4.2.5 output high-pass H_h2(z) + ×2 up-scaling (full §4.2 post-processing cascade wired into decode chain) + §4.4 frame-erasure concealment (voicing classifier + gain/predictor attenuation eqs 93–96 + periodic pitch-delay repeat) + erasure-corpus end-to-end decode (concealment wired into serial decode + whole-corpus PCM conformance, both g729+Annex-A corpora) + Annex B DTX/CNG decoder (variable-length framing + SID unpack Table B.2 + §B.4.2.1 energy dequant + §B.4.1/§B.4.5 frame-type routing + §B.4.4 CNG excitation gain + Gaussian-mixture synthesis eqs B.19–B.26 wired into the stream decoder + end-to-end .bit→PCM, active speech bit-exact); lacks §4.2.1 1/8 fractional pass (tab_hup docs-gapped) + §B.4.2.2 SID-LSP VQ dequant (tables absent) | 🚧 ~5% — scaffold |
| **IMA-ADPCM (AMV)** | ✅ 100% | ✅ 100% |
| **MS-ADPCM / IMA-ADPCM (WAV)** | ✅ 100% | ✅ 100% |
| **OKI / Dialogic VOX** | ✅ 100% | ✅ 100% — mono-only |
| **8SVX** | ✅ 100% | ✅ 100% |
| **iLBC** (RFC 3951) | ✅ 100% | ✅ 100% |
| **AC-3** (Dolby Digital) | ✅ ~97% — AC-3 + E-AC-3 (SPX/TPNP/AHT + enhanced coupling decodes end-to-end incl. §E.3.5.5.1 cross-frame carrier edge + §E.2.3.3.15 default coupling-banding structure — basic stereo-coupled E-AC-3 now ~91 dB PSNR) + LtRt downmix + typed PremixCompression (premixcmpsel/drcsrc) + complete typed BSI/metadata accessor surface + §7.9.4.2/§8.2.3.2 α-correct short-block IMDCT/MDCT (32 kHz fixture: residual 4-bit blk-0 bit-alloc desync vs reference encoder) + §6.1.9/§7.6/§7.7 DRC control surface (partial-comp cut/boost, heavy-compr RF mode, dialnorm target) | 🚧 ~95% — full AC-3 encode; E-AC-3 metadata writers |
| **AC-4** (Dolby) | 🚧 ~98% — A-SPX + DRC + ETSI codebooks + ACPL cfg0..3 + SAP + IMS bitstream walker + §5.7/§6.2.5 A-JOC decode parameter-processing core (dry/wet matrix dequant + decorrelator pre-matrix) + §5.7.3.6 Table 49 A-JOC spatial-reconstruction driver (pre-matrix + cyclic decorrelator + dry/wet upmix → reconstructed QMF objects, decode chain complete bar Huffman); lacks `ajoc_huff_data()` codebooks (docs-gapped) | 🚧 ~80% — IMS v0/v2 mono → 7.1, all eleven ACPL coupling layers real (α/β/γ incl. β₃) + TIME-direction A-SPX envelope DPCM + 5_X ACPL_3 live real QMF-derived ASPX envelope + multi-envelope (transient-driven num_env) live ASPX emission + live 5_X ASPX_ACPL_2 real `aspx_data_1ch()` centre-carrier envelope + live 7_X ASPX_ACPL_2 real ASPX envelopes + mono multi-envelope live A-SPX (5_X/7_X ACPL_2 centre carrier) + 7_X Table-202 back-pair (Lb/Rb) ASPX envelopes + live aspx_tna_mode transient-noise A-SPX inverse-filtering (5_X ASPX_ACPL_3 + 5_X/7_X ASPX_ACPL_2) + 7.0 pure-ASPX + 7_X ASPX_ACPL_1 real per-sbg SIGNAL/NOISE envelopes + aspx_tna_mode; lacks object/A-JOC encode |
| **MIDI** (SMF) | ✅ ~99% — SMF 0/1/2 → PCM via 32-voice mixer + SF2/SFZ/DLS soundfonts + typed meta/sysex surface + GM2 reverb/chorus DSP send bus (CA-024 Schroeder reverb + delay-line chorus, CC91-95 effect-depth classifier); synth −20% wall bit-identical | ✅ ~95% — SMF writer + synthesis |
| **NSF** (NES) | 🚧 ~98% — full 6502 + 2A03 APU + six expansion chips + VRC7/OPLL pipeline incl. envelope ladder + rhythm mode + NSF v1/v2/NSFe + NSFe mixe per-device default mix + §8a VRC7/OPLL AM tremolo (silicon-measured 14-level truncated triangle, ≈4.8 dB) + VIB bit-exact (§8b 8×8 PM table) + Namco 163 sum/divide multi-channel mixing + YM2413 rhythm noise generator (23-bit x²³+x⁹+1 LFSR) + §7 EG rate-increment model (eg_shift/eg_select) + §9 phase-gen 10.9 fixed-point (correct VRC7/OPLL pitch) + end-to-end frame-render gate + §7 global-counter EG model wired into live decay/release + §7 global-counter-driven OPLL attack envelope (silicon-measured 12-level sequence) + VRC7 user-patch live-reload fix; lacks §7a attack-level recurrence + rhythm HH/SD/TOM/TCY noise-mixed phase (docs gap) | — synthesis only |
| **Shorten** (.shn) | 🚧 ~40% (post-2026-05-18 orphan) — reference-encoder-anchored byte-exact decode fixtures (F1/F2/F4) + v2/v3 decode complete (DIFF0..3 + QLPC + Rice + streaming) + whole-stream encode driver (encode_stream — QLPC auto-select + predictor sequencer, sample-exact round-trip) + full-band Rice-energy predictor selector (e∈0..=29) + lossy `-q N` encode (BLOCK_FN_BITSHIFT, `decode(encode_lossy(s,N)) == (s>>N)<<N`); lacks #1267 ambiguity resolution | 🚧 ~10% — scaffold |
| **TTA** (True Audio) | ✅ ~98% — TTA1 fmt 1/2 + password + trailers + streaming + random-access + §04 decorrelation pinned vs captured reference tape (31-row §7.1 + N>2 cascade + end-to-end pipeline) + fuzz; decode −18% wall bit-identical | ✅ ~96% — bit-exact self-roundtrip |
| **Musepack** | 🚧 ~66% — SV8 packet walker + all 21 Huffman tables + §3.4 classifier-driven band dispatcher + SV8 §6.2 band-resolution + §6.3 SCFI/DSCF + single-channel frame-body assembler (decode_sv8_band routes CNS/empty/escape arms) incl. large-coefficient escape + SV7 §2.5 band-decode ladder wired end-to-end (decode_sv7_band) + SV8 SH stream-header field-map + SV7 (`MP+`) fixed-header field-map (sv7_header) + SV8 §6.4.1 sparse-band case-1 + §2.6 reconstruction to per-granule-SCF dequantised subband samples + SV8 RG/EI replay-gain/encoder-info field maps + SV7 §2.6 frame-band assembler (`SubbandMatrix`) + §2.6 M/S-undo structure (msflag-gated) + §6.4.2 first-order grounded context model (q2/5–8) + SV8 §6.2 band-resolution-header subsystem (top-down delta + ctx rule, Max_used_Band log/delta, M/S enumerative bitmap) + SV8 §6.3 frame->reconstructed-SubbandMatrix bridge (sv8_reconstruct, signed per-granule SCF gains) + SV7 §5.3 grounded SCF decode (SCFI table + DSCF escape) + SV7 §5.1 grounded Res band-type header (closes Res→band_type remap gap) + SV7 single-channel frame-body assembler (decode_sv7_frame_channel → BandLevels); lacks M/S undo arithmetic + 32-band synthesis filterbank (both docs-gapped) | 🚧 ~5% — scaffold |
| **Cook** (RealMedia) | 🚧 ~33% — flavor/cookie parsers + every extracted DSP table behind typed range-guarded APIs + decode-session orchestrator + per-band quantiser primitives + backend frame-syntax codebook/vector-dim geometry + joint-stereo mirror-index rotation + MSB-first frame bit reader (spec/05) + §1 gain-control envelope (segment-count + √2 ladder) + §0–§3 frame-body decode orchestrator (gain application + §2.1 subband geometry) driven on a real RA Cook stream + §2.2 full per-band quantiser closed form (clip·round·divisor) + §3.1 dequant scale triple + spectral-coefficient assembly + §3.1 per-band symbol/coefficient grouping arithmetic + §3.1 per-band/spectrum reconstruction + §4.2 joint-stereo decouple + frame-body integration (FrameSpectrum mono/stereo); lacks §3 spectral-VLC walk (BSS codebook + §2.2 0x8f38 category-cost LUT docs-gapped) | — |
| **WMA** | 🚧 ~14% — patent-disclosed primitives (analysis/synthesis windows + codebook grid + quantization-band layout + §4 energy-derived quantization matrix + §5 open-loop stereo channel-coding decision + §8 decoder-tail stereo time-domain reconstruction + §8 single-channel decode-chain assembler (spectral→dequant→noisefill→synthesis) + §8 two-channel StereoDecoder decode-block chain + §6 entropy front-half assembled (run-level → coefficients spectral→dequant chain); lacks Huffman codeword tables + exponent partition + sign layout (docs-gapped) | — |
| **WavPack** | 🚧 ~94% (post-2026-05-18 orphan) — v4 block/metadata/entropy parse + full §4.2 entropy ladder + multi-block PCM composer + inverse entropy encoder (bit-exact round-trip) + typed-refuses joint-stereo/cross-decorrelation (flag bits 4/5) + §5 running block-CRC compute/verify + §3.1/§3.4/§3.5 decorrelation inverse-prediction weight arithmetic (apply/update_weight primitives) + §3.2/§3.7 decorrelation inverse-prediction loop (all terms, mono+stereo, round-trip-pinned) + mono-lossless decode_samples → reconstructed PCM (§3.7 reverse-storage + multi-pass assembly) + §5 decoded-CRC verify + extension-stream CRC accumulator (crc_x / ExtensionCrc) + §3.2/§3.7 stereo decorrelation prediction loop + joint-stereo undo wired into block decode + §5.6 stream-level CRC mute gate + §1 left-shift final-normalization fixup (sub-byte 12/20-bit depths, CRC folds pre-shift); lacks hybrid + float + multichannel | 🚧 ~12% — forward decorrelation (recorrelate mono/stereo: arithmetic inverse of decode, all fixed-lag/extrapolate/cross terms + multi-pass, round-trip-pinned) |
| **APE** (Monkey's Audio) | 🚧 ~8% — header-prefix parser + stereo channel-decorrelation + range-coder frequency model (both version variants) + per-level filter cascade + scalar constants + stage-1 x*31>>5 predictor (cleanroom tables); range-decoder state machine + per-version header tail DOCS-GAP | 🚧 ~5% — scaffold |
| **DTS** (Core) | 🚧 ~62% — §5.4.1 DYNF/RANGE (§D.4 multiplier, post-QMF) + §C.2.5 streaming QMF synthesis → PCM (inter-frame filter tail; reference-shape-exact) + CPF/SICRC side-info tail (JOINX==0) + frame header + 14↔16-bit pack/unpack + side-information subframe walker + Annex C reconstruction primitives + Annex D codebooks (incl. §D.6 block-code books + §C.2.1 table-look-up decoder) + §D.8 FIR tables + fused 32-band synthesis QMF driver (§C.2.5 QMFInterpolation, per-frame multi-channel) + §D.2 step-size tables + §5.5 inverse-quant scale composition + §D.5.7–§D.5.9 13/17/25-level audio books + §5.5 Primary Audio Data Arrays decode walk + §D.5.10/.11/.12 ABITS 8/9/10 audio-data Huffman code books + end-to-end raw-16-bit Core frame → planar S32 PCM decode (§5.3→§5.4→§5.5→§C.2.5, JOINX/DYNF/CPF==0 common case) wired into the registry `Decoder`; lacks JOINX joint-intensity + §D.10 VQ + §C.2.6 LFE + 14-bit reconstruction | — |
| **aptX** (classic + HD) | 🚧 stub — NDA-blocked; clean-room QMF + 4-subband quantiser source-of-record purged in the 2026-05-06 audit (trace docs failed clean-room separation). Awaiting a non-contaminated `docs/audio/aptx/` (public-primary-source tables or black-box observer trace) | — |

</details>

<details>
<summary><strong>Video</strong> (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **MJPEG** | ✅ ~97% — §K.7.2 hierarchical DCT progression (SOF0/1/2 + SOF5 diff, 1/3-comp RGB+YUV incl. P=12 12-bit YUV/grayscale/RGB + 4-comp CMYK/YCCK) + baseline + progressive + lossless (Huffman + arithmetic, incl. SOF11 subsampled YUV-class) + 12-bit + CMYK/YCCK + RTP/JPEG + DNL (SOF Y=0, T.81 §B.2.5) + Annex J hierarchical spatial-lossless decode (1/3/4-component, EXP ×2 bi-linear upsample, bit-exact) + §C Kraft-inequality DHT over-subscription guard + typed APP0/APP14/ICC views + fuzz | ✅ ~97% — baseline + progressive + lossless (Huffman + arithmetic, incl. SOF11 subsampled) + CMYK + RGB24 + grayscale paths |
| **FFV1** | 🚧 ~95% — 14-fixture reference decode corpus (8/10/12/16-bit, 4:2:2/4:4:4, grayscale, RGB, RGBA, multi-Slice 4/16-way, ctx-1, v0/v1) + RFC 9043 intra decode, both coders (range + Golomb-Rice), YCbCr + RGB/RCT + validation gates + multi-Frame session driver + §3.8.1.3/§3.8.2.5 non-keyframe coder-state carry + registry `Decoder` trait wired (codec id + AVI/MKV tags) + §4.4 v0/v1 single-Slice YCbCr + RGB/RCT decode end-to-end (both coders, bit-exact lossless self-roundtrip) + v0/v1 `Decoder`-trait carriage + §3.8.1.6 coder_type-2 single-stream decode+encode (custom state-table, both colour layouts, keyframe + non-keyframe, bit-exact); + v0 Golomb-Rice reference-fixture decode bit-exact (§3.8.2.2 run-mode per-Line loop + §3.8.1.1.1 Sentinel handoff); lacks Golomb v1 fixture decode + inter-frame delta coding | 🚧 ~98% — both coders + RGB/RCT + full Parameters emit via unified encode_frame + RGB non-keyframe coder-state carry + §3.8.2.2 run-mode first-sample encodability gate + framework Encoder trait registered + v0/v1 prologue + Golomb-Rice/RGB encode (symmetric) |
| **MPEG-1 video** | 🚧 ~64% — headers + macroblock walk + dct_coeff + dequantiser + P1180-conformant IDCT + quant-matrix state machines + §6.2.4 I-picture decode driver + §7.6 picture-level P/B MC reconstruction driver (frame-based AND §7.6.4 field-based forward/backward/bidirectional MC + skipped-MB → real pixels end-to-end); + §7.6.7.3 field-picture 16×8-MC end-to-end; lacks encoder (frame-only — field/dual-prime MC are MPEG-2) | 🚧 ~5% — scaffold |
| **MPEG-2 video** | 🚧 ~80% — full §6.2 syntax walk + §7 reconstruction primitives (PMV / inverse-quant / IDCT / skipped-MB) + extension parsers incl. scalable + copyright + §6.2.5.1 spatial_temporal_weight_code + Table 7-21 class resolution + §7.6.3 slice-level MV reconstruction wired across the walker (PMV bank + skipped-MB resets); + §7.7.3 spatial-scalable pipeline complete (§7.7.3.1/Table 7-15 spatial-upsample dispatch + §7.7.3.4 deinterlace + §7.7.3.5/.6 resample + §7.7.3.7 reinterlace) + §7.6.4 picture-level reconstruction driver feeding the pel reader (frame-picture P/B end-to-end to real pixels) + §7.6 field-picture simple field prediction + §7.6.3.6/§7.6.7.4 dual-prime MC (field- + frame-picture) + §7.6.6.3 field-picture B-skip → all §7.6 MC modes; lacks GOP reorder + enhancement-layer combiner | 🚧 ~5% — scaffold |
| **MPEG-4 Part 2** | 🚧 ~80% — §7.6 progressive P-VOP motion-compensation wired end-to-end (MvDriver luma+chroma) + §7.6.2/§7.6.9 end-to-end P-VOP/B-VOP macroblock reconstruction + I/P/B texture + GMC global-motion warp end-to-end (§6.2.5 sprite_trajectory + §7.8.4–7.8.6 warp geometry/luma+chroma reconstruction) + quarter-sample + full padding family + interlaced info + B-VOP MV bodies + §7.7.2.1 field-MV reconstruction + §7.6.2.2 quarter-sample field MC + §E.1.4.4 two-way RVLC Tcoef (forward + backward) + §E.1.4.4.2.1 RVLC strategy selection + §7.4.2 SA-DCT modified inverse scan + §A.3.2 inverse SA-DCT + §A.4.2 inverse ∆DC-SA-DCT + §7.3.5/Table 7-2 inverse-transform selection routing + §6.3.6 S(GMC)-VOP mcsel macroblock layer + §6.3.6/§7.8.7.1 mcsel-gated GMC routing into the §7.3 recon loop (warped vs translational per-MB) + §7.8.2/§7.8.6 static-sprite warp-and-sample reconstruction + §7.6.8 four-PMV interlaced-B-VOP field MV predictor + §7.6.8 frame-level B-VOP MV decode driver (raster walk + §7.6.9 mode resolution + predictor threading + §7.6.9→§7.3 reconstruct bridge) + §7.4 residual-threaded B-VOP frame loop (§6.2.6 cbpb + §6.3.6 dbquant running-quantiser → motion+residual+reconstruct); lacks interlaced field-pred B-VOP + reference-plane selection + §7.8.3 low-latency sprite piece-update | 🚧 ~5% — scaffold |
| **Theora** | 🚧 ~92% — HD 1920×1088 / 1080p pixel-SHA-exact decode (quarter-pel chroma MC §7.5.1) + intra AND inter frames decode END-TO-END sample-exact from real packets (§6.4 setup-header + §7.9.4 motion-compensated reconstruction incl. half-pixel MV split) + registry `Decoder` trait wired; lacks Ogg carriage | 🚧 ~55% — §6 header-packet serialization (byte-exact ident inverse + all 80 §6.4.4 Huffman tables + §6.4.2 quant params) + `oxideav_core::Encoder` trait registered + intra keyframe stream across all 3 chroma formats (4:2:0/4:2:2/4:4:4) self-roundtrips through the decoder + inter (P-frame) encode (§7.2-§7.5 run-length/CBF/mode/MV writers + INTER_NOMV/INTER_MV/INTER_MV_LAST{,2} mode decision + I/P keyframe-interval emission, full Encoder→Decoder round-trip); lacks golden/four-MV encode + rate control |
| **H.263** | 🚧 ~93% (post-2026-05-18 orphan) — baseline + Annexes D/F/I/J + OBMC + PLUSPTYPE + Annex G PB-frames + Annex K slice-structured + Annex M improved-PB-frames + Annex T modified-quantization (MQ-active picture decode end-to-end on baseline + Annex I AIC paths: §T.2 DQUANT + §T.3 QUANT_C + §T.4 extended-range) + Annex S Alt INTER VLC (§S.2/§S.3) + Annex Q reduced-resolution update (§Q.6 prediction-error upsample + §Q.7 block-boundary filter) + §5.2.2 first-GOB header elision + §5.1.11–§5.1.16 PLUSPTYPE scalability (Annex N/O/P enhancement-layer header) + Annex O scalability macroblock-layer VLCs (Tables O.1–O.4) + Annex O EI + EP enhancement-layer end-to-end reconstruction + Annex N forward-channel RPS (§N.5 store + §N.4.1.4 TRP selection) + Annex P RPR (implicit §P.1 + explicit §P.2 reference resampling/warp → pixels) + §5.1.3 decode_sequence PLUSPTYPE picture dispatch (extended-PTYPE end-to-end + custom-PCF accept) + Annex-K slice PEI/SEPB1 ordering fix + Annex T MQ + Annex S AIV threaded through the slice driver + 6 mode-coverage byte-exact conformance fixtures + reference-encoder-fixture baseline decode conformance; lacks §N.4.2 back-channel BCM + EP-picture lower-layer RPRP | 🚧 ~5% — scaffold |
| **H.261** | ✅ ~99% — I+P + loop filter + BCH error correction + RTP/RTCP/SDP + Annex A + Annex C conformance + fuzz-hardened (6 targets; packetiser MV-desync panic fixed) | ✅ ~98% — ME + rate control + §3.4 forced-update cyclic INTRA refresh + §4.2.3.1 MBA-stuffing emit/pad (§5.2 HRD buffer regulation) + BCH/RTP framing; 45 dB at 64 kbit/s QCIF |
| **MS-MPEG-4** (v1/v2/v3) | 🚧 ~64% — v3 I/P decode + both MV VLC variants (default + alt; v3 joint-MV now decodes against extracted wire codes) + v1/v2 P-frame pixels end-to-end (skip + inter MBs, half-pel MC) + v1 P-frame INTER+Q + INTER4V (4-MV) + v1/v2 intra pixel pipeline (I-frame + intra-in-P MBs) via spec/16 §2 H.263 size+value DC differential + spec/16 §3 table-grounded v1/v2 MB-type decode (region_053140; fixes INTRA+Q misclassification) + v3 joint-MCBPCY intra/inter partition-polarity correction (idx<64=intra per US 6,563,953 Table 1; was inverted — every coded v3 P-MB mis-classified) + picture-level alt-MV-VLC byte-LUT discriminating pin; lacks v3 4-MV MCBPC + real-content bit-exactness | — |
| **H.264** | 🚧 ~84% — §8.5.8 QP_Y >8-bit dequant fix (High10 right-col ~93% byte-exact) + I/P/B + CAVLC/CABAC + all chroma layouts incl. §8.3.4.5 4:4:4 I_NxN chroma recon + §8.7.2 4:4:4 chroma deblock via luma filtering process (intra-only-high444 frame-1 byte-exact) + DPB + 53 SEI types (incl. §H.13.2.6 alternative_depth_info + §G.13.1.7 MVC view_dependency_change) + Annex G MVC subset incl. NAL 20 coded-slice-extension header path + §7.3.2.1.2 SPS-extension (NAL 13 aux/alpha) + §H.7.3.2.1.4 MVCD subset-SPS (profiles 138/135) + §6.4.12.2 MBAFF mb-pair neighbour derivation (Table 6-4) + MBAFF field/frame-adaptive decode (mbaff-interlaced end-to-end) + §8.2.4.2 field reference-picture-list init (parity-alternation interleave) + §8.2.4.3 field RPLM + fuzz-hardened + enforced BitExact conformance tier across 10 staged fixtures; lacks SVC bodies | 🚧 ~83% — I+P (¼-pel) + B + CABAC + Trellis RDOQ-lite; PSNR_Y 44.2 dB |
| **H.265 (HEVC)** | 🚧 ~78% — parameter sets + §9.3 CABAC engine with COMPLETE §9.3.2.2 context-init (Tables 9-5..9-42) + §8.7.2 in-loop deblock complete (edge-flag + CU + picture drivers) + full slice header + residual_coding() driver + §D.2 SEI parse (mastering-display + content-light + recovery-point + decoded-picture-hash) + §8.6.2/§8.6.3/§8.6.4 scaling + inverse transform + §7.3.8.9 mvd_coding + §7.3.8.6 merge_flag binarization + §8.4.2/§8.4.3 intra pred-mode derivation + §8.4.4.2 intra sample prediction (substitution/filtering/planar/DC/angular) + §6.4/§6.5 z-scan/tile-scan neighbour availability + §7.3.8.10 transform_unit() + §7.3.8.8 transform_tree() recursion syntax drivers + §7.3.8 slice-data CTU/CU CABAC syntax walk + §8.4 intra sample reconstruction (§8.6.4 transform-orientation fix + §8.6.1 Qp derivation + picture buffer — tiny-i IDR slice decodes to byte-exact pixels) + §8.7.3 SAO apply (edge + band CTB modification + §7.4.9.3 SaoOffsetVal) + Table 8-12 β′/tC′ + §8.7.2.5 deblocking luma/chroma sample-filter kernels + §8.5.3 inter (P/B) PU reconstruction to pixels (§8.5.3.2 MV/chroma-MV resolution + 8-tap luma/4-tap chroma MC interp + default-weighted bi-pred → bit-exact) + §8.7.2.4 deblocking bS derivation + §8.4.2 neighbour-aware intra MPM (IntraModeField + §6.4.1 z-scan reference availability) + reconstruct_intra_picture multi-CTU recon+SAO driver (tiny-i IDR byte-exact); lacks §8.5.3.2 merge/MVP candidate derivation + DPB + multi-CTU/slice/tile assembly | 🚧 ~5% — scaffold |
| **H.266 (VVC)** | 🚧 ~80% — 4:2:0 IDR intra + full inter toolset (ALF/SAO/HMVP/MMVD/CIIP/BCW/BDOF/GPM/DMVR/affine/PROF/SbTMVP) + §8.7.4.1 MTS (implicit+explicit) + non-square LFNST into intra recon + §8.5.8/§8.7.5.1 inter residual reconstruction (merge/CIIP/subblock-merge non-skip CUs `recSamples = Clip1(pred+res)` over MC luma+chroma) + typed RBSP/parameter-set surface + Annex-D SEI prefix indication (201) + DU info (130) + §D.4.1 picture-timing SEI + LMCS arrays + sample-domain luma map/chroma residual scale (§8.7.5.2/§8.8.2) + §8.7.4.6 inverse adaptive colour transform + §8.7.2 scaling-and-transformation orchestrator (joint Cb-Cr residual) + §8.5.6.6.3 explicit weighted sample prediction + §D.10 CREI SEI + §D.3.1 buffering-period SEI + §8.5.2.1 non-merge AMVP inter reconstruction (candidate derivation → mvd → MC to pixels) + §7.4.12.5 SBT + §7.3.11.4 multi-TB tiling + §7.4.12.11/§8.7.2 joint Cb-Cr inter residual reconstruction + §8.5.5.3/§8.5.6.6.2 affine uni/bi-pred reconstruction to pixels (per-4×4 MV grid + PROF + default-weighted bi-avg) + §8.5.5.5 affine-CPMV parse-to-pixels (eqs 660-667 MVD fold + §8.5.5.7 candidate list → uni/bi recon from parsed CPMVs) + §8.7.4 inverse LFNST (16 lowFreqTransMatrix tables, parse-to-pixels into intra reconstruction)+ §7.3.11.12 transform-skip residual coding (intra luma+chroma, §9.3.4.2.6-.9 TS ctxInc); lacks per-CB affine CPMV store (inherited/constructed CPMVP) + transform-skip inter residual | 🚧 ~93% — forward CABAC + DCT-II + MTT RDO + P/B + sub-pel MC + weighted bi-pred + affine/AMVR/BCW dispatchers |
| **VP6** | 🚧 ~80% — BoolCoder + DC/AC coefficient decode + MV decode/reconstruction + custom scan + per-block reconstruction + §11.4 FilterVarThresh resolve + §11.5 variance edge-clamp + header-driven FilterConfig + §2/§13/§17 block-to-plane raster frame assembly + §9 output-scaling typed surface + §10 macroblock coding-mode traversal + §10 frame-level MB mode-decode pass (availability-aware prob-row select) + §9 BoolCoder frame-header tail + §7.3 BoolEncoder (decoder-matched range encoder) + §10/§11 per-MB MV resolution (Zero/New/Nearest/Near) + §16 inverse DCT (i64-widened descale) + §17.1 per-MB intra decode loop (I-frame decodes END-TO-END to output pixels) + §17.2/.3 integer-MV inter recon + FourMV macroblock resolution + §17.4 sub-pixel MC predictor (§11.4 bilinear/bicubic) + §11.3 loop filter (BoundaryX/Y round-toward-zero fix) + fused inter (P-frame) per-MB decode driver → END-TO-END pixels + §4 golden-frame ReferenceFrames bookkeeping + top-level per-frame Vp6Decoder assembly driver (§9 header→keyframe/inter dispatch) + registry `Decoder` (codec id vp6, tags VP60/VP61/VP62/vp6f) + keyframe→P GOP end-to-end; lacks motion estimation | 🚧 ~28% — P-frame encode (encode_inter_frame all-CODE_INTER_NO_MV + §10 mode_encode; keyframe→P GOP round-trip) + intra (I-frame) encode: §16-dual forward DCT + quantise + §13 token coding (DC/AC trees + §13.3.3.1 zero-run) + §9/§14 header emit → decoder-reconstructible keyframe (flat-exact; ~44 dB at q=48); lacks P-frame encode + rate control |
| **VP8** | ✅ 100% | ✅ 100% |
| **VP9** | 🚧 ~74% — §8.4 backward probability-adaptation transforms + decode_vp9 decodes keyframes END-TO-END, byte-exact on the 13-fixture corpus (§6.4 wiring + intra + §8.8 loop filter) + §6.4.19/20 inter MV residual syntax + §6.5 MV reference geometry (find_best_ref_mvs predictor + §6.5.1 find_mv_refs candidate scan/ModeContext) + §6.4.18 assign_mv per-reference-list MV resolver + §6.4.17 read_ref_frames + §6.4.16 inter_block_mode_info driver + §8.5.2.1/.2/.3 MV selection/clamp/scale + §8.5.2.4 block inter prediction (two-pass 8-tap sub-pixel convolution) + §8.5.2 inter-prediction driver (single + compound) + §8.10 reference frame-buffer state + §6.2 inter uncompressed-header parse (P-frame header byte-exact) + §6.4.4 decode_block inter arm wired → P-frame decodes END-TO-END byte-exact (i→p 64×64 fixture, single-ref 8-tap) + `decode_vp9_sequence` multi-frame driver (§8.10 RefBuffers + §6.5 prev-frame motion field + show_existing_frame sources per-slot §8.9 FrameStore incl. hidden-ARF slots) + §6.1.2 FRAME_CONTEXTS persistence (load/save_probs forward-context threading) → 14/16-fixture P-frame byte-exact; lacks probability adaptation + encoder | 🚧 ~5% — scaffold |
| **AV1** | 🚧 ~40% — §7.14 in-loop deblock bridge (full §7.4 deblock→CDEF→LR trio) + standalone intra decode, 4:2:0/monochrome multi-superblock to 128×128 + registry `Decoder` wired (codec id `av1`; container tags av01/AV01/V_AV1) + §6.10.3 BlockDecoded availability grid (§5.11.3/§5.11.35) + §7.15 CDEF driven from the decoded cdef_idx grid → real post-CDEF frame + §7.17 loop-restoration runs end-to-end (Wiener + self-guided filters applied from decoded §5.11.58 units) + §5.11.33 predict() drives single-ref + AVERAGE/DISTANCE/WEDGE/DIFFWTD compound inter reconstruction across the whole mode-info grid + §7.11.3.1 single-ref inter-intra blend (COMPOUND_INTRA + wedge sub-arms) wired into the §5.11.33 frame-walk inter-intra leaf + single-ref + compound (AVERAGE/DISTANCE/WEDGE/DIFFWTD) MC reconstructed inline into the §5.11.5 walk's CurrFrame buffers, incl. single-ref + compound multi-plane chroma (compound shares the luma-grid §7.11.3.12 DIFFWTD mask) + §5.11.33 someUseIntra intra-neighbour sub-block split + §7.11.4 palette prediction leaf + §7.11.3.1/§7.11.3.5 warped-motion inter reconstruction wired into the §5.11.33 single-ref frame walk + §7.11.2.1 general intra prediction wired into the §5.11 walker → full intra-tile reconstruction (predict_intra → inverse transform → residual merge → §5.11.2 tile assembly) + §7.11.2.4 directional intra edge-filter + 2× upsample pre-pass (luma+chroma, §7.11.2.8 get_filter_type); + §7.11.3.9-10 OBMC reconstruction bridge (above/left/chroma); lacks OBMC frame-walk dispatch + film grain + superres | 🚧 ~34% — intra encode YUV→IVF + §5.11 write side incl. §5.11.47 transform_type + §5.11.57/§5.11.58 read_lr/read_lr_unit loop-restoration unit syntax (use_wiener/use_sgrproj/restoration_type S() + Wiener-tap/sgr-xqd signed-subexp-bool) in decode-walker lockstep; lacks RD picker + inter reconstruction/encode chain |
| **Dirac / VC-2** | ✅ ~97% — VC-2 LD+HQ + Dirac intra/inter + OBMC + 7 wavelets + 10/12-bit + fragmented pictures + asymmetric transforms; bit-exact intra | 🚧 ~97% — HQ+LD + sub-pel 2-ref bipred + rate control + inter sequence-level rate driver (PerPicture/Cbr/Vbv/VbvHysteresis residue-byte) + asymmetric transforms |
| **AMV video** | 🚧 ~80% — typed frame-geometry binding + §4 demuxer 1:1 video:audio interleave cross-check + §4a device-stripped JPEG reconstruction (splices Annex K DQT/DHT + baseline SOF0 4:2:0 → conforming JFIF) + full baseline-JPEG frame decode to RGB (device-hardcoded video tables wired, #127) | 🚧 ~5% — scaffold |
| **ProRes** | ✅ ~96% — RDD 36 all profiles + 8/10/12-bit + alpha + interlaced + §7.5.1 Full/Video output-range clamp + typed header accessors + §6.1.1 qmat-source provenance + IDCT qualification + §6.4 picture version-variant forward-compat + §7.5.3 alpha-array-length proven against reference bitstream + interlaced 4444 16-bit alpha edges; ffmpeg interop 60-68 dB | ✅ ~97% — all 6 profiles + interlaced + alpha + constant-frame-size stuffing + SHA-256 lockstep pins + ffmpeg cross-decode |
| **EVC** (MPEG-5) | 🚧 ~95% — Baseline profile complete + §7.3.4 entry points + §7.4.5 tile subsets + §7.3.8.1 multi-tile slice_data walk + §7.3.8.2 xFirstCtb derivation (errata-#97 reconciled) + §7.4.8.3 Main-profile BTT split-geometry + §7.3.8.3 split-unit recursion geometry + §7.4.9.3 SUCO split-order availability (eqs. 68/69) + §8.4.4.8/.9/.10 EIPD intra prediction kernels (BI/PLN/DC + 33-direction angular) + §8.4.2/.3 EIPD luma/chroma mode derivation + §7.3.8.4 EIPD intra-mode CABAC syntax wiring (mpm/pims/rem via §9.3.3.6 TB primitive) + §7.3.8.5 ATS-intra syntax + Table 30 trType + §8.7.4.3 DST-VII/DCT-VIII full {4,8,16,32} kernels + §8.7.4.3 apply_inverse decode→transform bridge + §8.5.2.3 ADMVP merge-mode MV derivation (spatial A1/B1/B0/A0/B2 + §8.5.2.3.6 HMVP-merge + §8.5.2.3.7 combined-bipred + zero-fill + step-6 selection→MC bridge) + §8.5.2.3.3–.5 TMVP temporal-collocated merge candidate (POC-scaled, SideInfoGrid ColPic bridge)+ §8.5.3 affine toolset (model params + per-subblock MV field + CPMV reconstruct + inherited-CPMV projection + §7.3.8.4 affine CABAC syntax); lacks affine candidate-list assembly + ATS-inter + per-CU TMVP wiring | — |
| **HuffYUV** / FFVHuff | ✅ ~97% — HFYU/FFVH + 6 predictors + interlaced + fast-LUT decode + fuzz | ✅ ~97% — v1.x + v2.x symmetric encode across YUY2/RGB24/RGB32 |
| **Lagarith** | ✅ ~95% — all 11 wire types + modern range coder + legacy adaptive-CDF + typed header surface + fuzz | 🚧 ~85% — all 11 wire types + all-nine-sub-form encodability proven + byte-exact self-roundtrip across exhaustive matrix (incl. YUY2 odd-width + YV12 odd-dim SPECGAP closure) + 1900-iter fuzz; byte-exact-vs-proprietary verification fixture-blocked |
| **Ut Video** | ✅ ~97% — 5 FourCCs × 4 predictors + slice-parallel decode (5.6× at 720p) + opt-in strict-padding conformance decode | ✅ ~96% — slice-parallel encode (3.3×) + fuzz oracle |
| **MagicYUV** | ✅ 100% | ✅ 100% |
| **Cinepak** (CVID) | ✅ ~98% — full CVID intra/inter + Sega FILM demuxer + Saturn/3DO deviants + typed walkers + fuzz; decode 4.4 GiB/s | ✅ ~98% — rolling codebooks + RDO/LBG + rate control; 34.2 dB PSNR |
| **SVQ1/SVQ3** (Sorenson) | 🚧 ~58% — SVQ1 codebook payloads + leaf stage-accumulation reconstruction (spec/04 §4.5) + motion-vector median prediction + full intra-MB reconstruction composition (4×4 + 16×16 luma + chroma DC plane + 3-plane assembly) + SVQ1 inter MV cache + INTER/INTER_4MV neighbour geometry + SVQ3 transform/dequant/intra/interp primitives + chroma DC full-dequant pipeline + SVQ3 4×4 coefficient scan-order arrays (normal + alt) + quantiser-driven scan selection + spec/01 Gap-5 Clip1 predicted+residual writeback (reconstruct_4x4) + intra predictor-selection macroblock loop (5 4×4 modes + 16×16 plane/DC + chroma DC, driven across the MB grid) + spec/01 per-block intra reconstruction composition (place→dequant·M·X·Mᵀ→Clip1, Gaps 1–5) + SVQ3 signed-Golomb MV-difference + inter-MB motion-header decode; lacks CBP me(v) + intra-4×4-mode VLC wire decode (both docs-gapped) | — |
| **Indeo 3** (IV31/IV32) | 🚧 ~72% — headers + VQ codebooks + MV decode + cell decomposition + MC executor to output pixels (§7.2 fix-up + 4-mode cell copy) + §3.2 mode-byte jump-table dispatch + §5.5 4:1:0 chroma box-upsampler + spec/07 §5.5 full-res YUV producer (chroma 4×4 box-upsample over §5.7 assembly → assemble_yuv) + spec/07 §6 frame finalisation (saved frame_flags/frame_number + continuity check + return codes) + spec/06 static-table cell-reconstruction executor (mode-byte stream consumer + plane disposition classifier + VQ_NULL copy-cell pixel drive) + whole-plane/whole-frame VQ_NULL reconstruction executor → real strip pixels via §4.3 upshift (deferred VQ_DATA/INTER left black); lacks codebook-bank LUT values (docs ask) | — |
| **Indeo 2/4/5** | 🚧 ~0% scaffold — pending clean-room workspace; Indeo 4/5 run sandboxed via `oxideav-vfw` | — |

</details>

<details>
<summary><strong>Image</strong> (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **PNG / APNG** | ✅ 100% | ✅ 100% |
| **GIF** | ✅ 100% | ✅ 100% |
| **WebP** (VP8 + VP8L) | ✅ 100% | ✅ 100% |
| **JPEG** (still) | ✅ ~95% — via MJPEG | ✅ ~90% — via MJPEG |
| **TIFF** (6.0) | ✅ ~98% — tiled subsampled/4:4:4 YCbCr decode+encode (§21) + II/MM + BigTIFF + 7 photometrics + all baseline compressions + CCITT fax (incl. T.4-2D/T.6 2-D uncompressed mode) + tiles (incl. sub-byte 1-/4-bit) + multi-page + JPEG-in-TIFF + SampleFormat 1/2/3 (incl. IEEE-float 16/32/64-bit) gray + RGB + §18 ExtraSamples=1 associated (pre-multiplied) alpha + fuzz | ✅ ~95% — chunky + planar + tiled across Gray/RGB/Palette/CIELab/CMYK/YCbCr with predictor + fax modes; decode honours all 8 Orientation values (mirror/rotate/transpose) |
| **BMP** | ✅ ~97% — 1..32-bit + V4/V5 + OS/2 + RLE + ICC profiles + fuzz | ✅ ~97% — top-down + palettes + V4-calibrated-RGB/V5/linked-ICC writers + Rgb565/Pal8 |
| **Netpbm** (PBM/PGM/PPM/PNM/PAM) | ✅ ~95% — all 8 magics at 1/8/16-bit + 6 PAM TUPLTYPEs + fast paths (~45-50 GiB/s) + fuzz | ✅ ~95% — incl. P7 GRAYSCALE_ALPHA 16-bit |
| **ICO / CUR / ANI** | ✅ ~98% — multi-res + BMP/PNG sub-images + hotspots + ANI playback + per-step frame/hotspot accessors + directory-level `select_*_raw` best-fit + strict validation (incl. anih.bfAttributes reserved-bit reject) | ✅ ~94% — ICO/CUR + symmetric ANI/ACON `write_ani_raw` encoder |
| **JPEG 2000** | 🚧 ~80% — END-TO-END decode (headers → tier-2/tier-1 MQ → IDWT → MCT → Annex E reassembly across all 5 §B.12 progression orders incl. RPCL/PCRL/CPRL, §D.3.4 π-membership fix + per-coefficient §D.2.1 Nb → rate-truncated 9-7 now ±1 of reference) + multi-layer (quality-layer) reassembly + §A.6.5 QCC + §A.6.2 COC main-header per-component coding-style overrides + §C.3.6 context-reset coding-style bit + §D.4.2 termination-on-each-coding-pass + §A.6.3/§H.1 main-header RGN implicit-ROI (Maxshift) + 4 fuzz targets; + §D.6 selective arithmetic-coding bypass decode (5-3/9-7/multi-tile pixel-exact) + HTJ2K (ITU-T T.814 / Part-15) block-decoder end-to-end bit-exact vs the reference HTJ2K decoder (§7 MagSgn/MEL/VLC/SigProp/MagRef + Annex-C CxtVLC/U-VLC, single-HT-set across multiple HT code-blocks per sub-band, gray/RGB/RCT/9-7, Annex-C CxtVLC byte-audited 802 entries); + §A.6.6 POC progression-order-change wired into decode (main + tile-part precedence) + §A.8.1 Nsop SOP sequence-number validation + §D.4.2 predictable-termination resilience; lacks Part-2 scaling ROI + HTJ2K MULTIHT/placeholder-pass | 🚧 ~5% — scaffold |
| **JPEG XL** | 🚧 ~94% — §C.7 HfGlobal section (HfPass + histograms + ANS init) on real VarDCT + ISO/IEC 18181-1:2024 lossless Modular bit-exact on all staged fixtures + per-block VarDCT decode walk to spatial samples (square + six rectangular/non-square DCT families) + per-LfGroup three-channel residual-plane assembly + Annex G chroma-from-luma + LLF-seeded block-residual dequant + §6.2 right/bottom crop of the padded VarDCT reconstruction + §J.3.3 VarDCT per-block-sigma EPF (σ<0.3 block-skip) + fused live-entropy per-LfGroup VarDCT reconstruction driver (§C.8.3 multi-pass entropy decode → spatial in one call) + §C.7.2 histogram-backed per-LfGroup VarDCT decode driver (multi-pass §C.8.3 outer loop, bit-exact vs the closure path) + integrated single-LfGroup VarDCT decode runs end-to-end to a non-degenerate RGB frame (public path withholds VarDCT pixels pending per-block HF-scaling validation); lacks frame-level framing | — retired |
| **JPEG XS** | 🚧 ~89% — Part-1 decode + 5/3 DWT + multi-component + high bit depth + 4:2:0 + odd-dimension geometry + Annex C.6.3 cross-precinct vertical prediction (Table C.11) + §C.5.4 Ldat data-subpacket size inference + Annex C.6 bitplane-count subpacket Lcnt[p,s] inference (Tables C.12-C.14) + Part-1 decode conformance gates (Annex C.2 Lprc/Ldat + sub-packet filler cross-check + reserved-field/R[p]/Ss/Cpih=3 rejection, Tables A.9-A.13) + uniform inverse quantizer (Qpih=1) across 4:4:4/4:2:2/4:2:0 composed with multi-slice/multi-precinct/Fs=1/RCT/Star-Tetrix + Annex C.3 Rl=0 raw-mode-consistency decode gate + horizontal-only NL,y=0 decode geometry (joint LL+HL first-packet; luma/RGB/4:2:2/highbd/lossy) + Table A.7 Ng=4/Ss=8 conformance gate + multi-significance-group (Ns>1) round-trip coverage | 🚧 ~97% — Nc 1/3/4 + RCT/Star-Tetrix + NLT + per-precinct rate-budget pickers + Annex H content-adaptive WGT weights (4:4:4 RCT + subsampled 4:2:2/4:2:0 H.4–H.11 incl. CFA Star-Tetrix Cpih=3/Sd=1) + Annex E.3 Fq fractional scaling (Bw=20/Fq=8 high-precision lossy) + Table A.8 (Bw,Fq) conformance fix |
| **AVIF** | 🚧 ~95% — end-to-end HEIF→AV1 decode (grid / alpha / rotation / crop) + §8.11.3 item byte resolution (construction_method 0/1/2 file/idat/item-offset; §8.11.3.3 item_offset follows the 'iloc' iref) across primary / grid-tiles / alpha / metadata) + iovl/iden/tmap/sato/grid derived-image geometry resolution (HEIF §6.3/§6.6.2/§6.6.2.3 overlay-canvas clipping + iden crop-of-original + tmap base-derivation + sato sample-transform + grid tile-placement) + §6.5.4..§6.5.40 item-property surface (incl. tols essential descriptor + §6.5.40 cmin camera-intrinsics) + §8.16.5 prft producer-reference-time + §6.5.36 amve ambient-viewing-environment + gain maps (tmap ISO 21496-1 §6 parse + apply) + profile audits; pixel fidelity tracks oxideav-av1 intra | — |
| **DDS** | ✅ ~99% — header + DXT10 + BC1-7 + BC6H all modes + ASTC LDR decode (Khronos DFS ch.23, all 4×4–12×12 footprints + void-extent + multi-partition + dual-plane, DXGI 133–187) + cubemaps/arrays/volumes + 16-bit/float + packed R11G11B10_FLOAT + R9G9B9E5_SHAREDEXP + R10G10B10A2_UINT + sub-sampled packed R8G8_B8G8/G8R8_G8B8 + 8/16/32-bit plain-integer UINT/SINT HDR uncompressed surfaces + normalised 8/16-bit UNORM/SNORM 1-/2-channel surfaces (normal/height maps) + legacy X8B8G8R8/X1R5G5B5/X4R4G4B4/L16/A4L4 mask layouts + YUV video formats (AYUV/Y410/Y416/YUY2/Y210/Y216/NV12/P010/P016/420_OPAQUE/NV11 → interleaved YUVA) + depth/depth-stencil decode (D16/D32/D24S8/D32S8 + R24G8/R32G8X24 typeless) + encoder dangling-index OOB→InvalidData guard + daily fuzz | ✅ ~96% — uncompressed + BC1-7 + BC6H + mip chains + cubemap/array |
| **OpenEXR** | 🚧 ~95% — scanline + tiled + deep + multi-part across all 4 part types + mip/ripmap + PXR24 & B44/B44A decode+encode across scanline / tiled (ONE_LEVEL/MIPMAP/RIPMAP) / multi-part (edge-aware HALF blocks + pLinear inverse-log) + mixed multi-part files carrying multi-level (MIPMAP/RIPMAP) flat tiled parts + typed attribute inspectors; PIZ/DWAA/DWAB + deep multi-level blocked on docs trace | ✅ ~96% — scanline (+PXR24 + B44/B44A) + tiled + deep + multi-part mixed write (incl. multi-level tiled) |
| **Farbfeld** | ✅ 100% | ✅ 100% |
| **HDR** (Radiance RGBE) | ✅ ~99% — new/old RLE + all axis flags + header metadata + derived colorimetry + scene-referred physical luminance (EXPOSURE/COLORCORR recovery) + fuzz + Criterion suite w/ ranked hotspots + bit-exact RGBE-quad round-trip surface (`from`/`to_rgbe_quads`) + 8×8×4 resolution/orientation/mode property matrix + D₄ geometric reorientation (HdrImage::reorient across the 8-orientation matrix, wire-verified) | ✅ ~98% — RLE modes + XYZE↔RGB + 8 tonemap ops |
| **QOI** | ✅ 100% | ✅ 100% |
| **TGA** | ✅ 100% | ✅ 100% |
| **ICER** (JPL) | 🚧 ~80% — bit-plane scan + spec-exact reversible integer wavelet (all 7 filters A–F+Q, IPN 42-155 §II.A, bit-exact lossless) + §III.B context model + §III.A subband priority model (Fig. 7 weights + cross-subband bit-plane order) + §III.A deadzone mid-bin reconstruction (+0.5–3.8 dB on truncated) + packet-loss-lenient multi-segment decode + YUV 4:4:4 colour encode/decode + fuzz-red root-caused (geometry-preserving budget truncation §V.B + compute-bounded DecodeLimits + §III.A per-coefficient deadzone on mid-plane budget cuts, +1.4–3.0 dB) | ✅ ~85% — quota encoding + R-D byte budget + §III.A image-domain-weighted R-D packet selection + PSNR-target rate control |
| **WBMP** | ✅ 100% | ✅ 100% |
| **PCX** (ZSoft) | ✅ 100% — all bpp/plane layouts + DCX multi-page + over-padded-stride decode robustness + fuzz | ✅ ~94% — 8 write paths + DCX; lacks framework `PixelFormat::Pal8` |
| **ILBM** (Amiga IFF) | ✅ ~94% — BMHD/CMAP/BODY + EHB/HAM + SHAM/PCHG + ANIM op-0/1/5 + colour-range chunks + RGBN/RGB8 24-bit genlock-RLE bodies + DEEP chunky deep-raster (DGBL/DPEL/DLOC + TVDC + RUNLENGTH/ByteRun1 decode+encode → RGBA); lacks DEEP HUFFMAN/JPEG bodies | ✅ ~84% — muxer parity + masking + ANIM op-5 |
| **PICT** (Apple QuickDraw) | ✅ ~99% — v1 + v2 opcode walkers + rasteriser + indexed PixMap + picture comments + CopyBits/PnMode transfer modes + DirectBits packType 0→§A-3 default packing + QuickDraw text rasterisation (built-in clean-room ASCII face + TxRatio h/v anisotropic glyph scaling + lineJustify intercharacter spacing); lacks system-font pixel fidelity (txFace/grayishTextOr) | ✅ ~94% — `PictBuilder` covering every v2 drawing-command family |
| **SVG** | ✅ ~99% — full SVG 1.1 + SVG 2 feature grid (shapes / text / gradients / masks / markers / SMIL / CSS3 selectors + media queries) + all 16 §15 filter primitives rendered + feDropShadow & feComposite (over/arithmetic) + feMerge/feGaussianBlur(edgeModes)/feOffset/feComponentTransfer(§9.7)/feMorphology(§9.17 erode/dilate)/feConvolveMatrix(§9.9)/feDisplacementMap(§9.11)/feTile(§9.20)/feTurbulence(§9.21 Perlin) + feDiffuseLighting(§18)/feSpecularLighting(§19) Sobel-normal lighting pixel evaluation + top-level filter-graph DAG evaluator (in/result chaining + §9.4 subregion clip) | ✅ ~88% — round-trips full shape graph + PreservedExtras + §10.9.2 dominant-baseline |
| **PDF** | ✅ ~99% — §9.4.3/§9.4.4 text-matrix glyph advancement + bytes → Scene via xref/ObjStm + encryption R=2..6 + signatures + text extraction + Tagged-PDF + §14.6 marked-content + 5 stream filters + annotations + §7.10 multi-input Type 0 (Order-1 + Order-3 cubic-spline)/Type 4 + Type 2/3 functions + Type 3 /FontMatrix glyph-advance scaling + §8.6.6.5 DeviceN + Separation tint transforms + §8.7.4.5 all 7 shading types evaluated to geometry/colour (Gouraud/Coons/tensor meshes + axial/radial/function gradients); read −30% wall (content-number fast path, bit-identical) | ✅ ~99% — multi-page writer + encryption + signatures + AcroForm + annotation/embedded-file/timestamp writers |

</details>

<details>
<summary><strong>3D scenes & assets</strong> (click to expand)</summary>

> The typed Scene3D / Mesh / Material PBR / Skin / Animation / Camera / Light / AudioEmitter model lives in `oxideav-mesh3d`, with `Mesh3DDecoder` / `Mesh3DEncoder` traits and a `Mesh3DRegistry` that's parallel to `oxideav-core::CodecRegistry`. Per-format crates register into it. `oxideav-meta::populate_mesh3d_registry(&mut Mesh3DRegistry)` walks every enabled format's `register()`. Lazy bytes flow through `AssetSource` (with a `raw_storage` pass-through hook for archive-backed sources, e.g. ZIP-stored USDZ textures + audio).

| Format | Decode | Encode |
|--------|--------|--------|
| **STL** (ASCII + binary) | ✅ ~99% — both forms + colour attrs + topology + 9-step repair pipeline + validation/lint surface | ✅ ~99% — both formats + attribute pass-through |
| **OBJ** (+ MTL) | ✅ ~98% — full Wavefront grammar + MTL (Phong + PBR) + free-form curves/surfaces with trim-loop re-meshing + ctech/stech cparm resolution-aware tessellation + typed directive accessors + superseded cdc/bzp free-form tessellation + typed obj:superseded accessor + smoothing-group vertex-normal synthesis + fuzz | ✅ ~96% — symmetric + negative-index encoder |
| **glTF 2.0** (+ .glb) | ✅ ~98% — JSON + .glb + full PBR + 12+ KHR extensions (incl. KHR_gaussian_splatting typed splat-field decode + ellipse-kernel attribute + SH colour evaluator + SH-degree conformance) + skins/animations/morph targets + KHR_texture_transform across every textureInfo (incl. material-extension slots) + spec-MUST validators (index-reference + structural-minimum + animation-sampler + image-source + morph-weights passes); Draco/meshopt + splat bitstream pending | ✅ ~93% — symmetric round-trip incl. XMP |
| **USDZ** (+ USDA) | ✅ ~95% — ZIP walker + USDA composition (LIVRPS / variants / references) + `.usdc` Crate parser with resolved-spec join (SPECS↔FIELDSETS↔FIELDS) + §4.6 spec pathIndex bounds-check + §3b int-coded buffer trailing-payload reject + §4.3 value-rep flag decode + value-region resolution (inline scalars / uncompressed+compressed arrays); lacks §4.5 PATHS tail (docs gap) + FIELDS value-rep type codes + UsdSkel | ✅ ~88% — symmetric writer + pass-through + composition arcs |
| **FBX** | 🚧 ~93% — binary + ASCII + object graph + mesh/animation/deformers + Properties70 grammar + class-default resolution + multi-LayerElementNormal layers + FBXHeaderExtension authoring metadata; lacks Constraint/Pose/MarkerSet round-trip | ✅ ~58% — symmetric binary + ASCII writer + opt-in deflate |
| **IFC** (BIM, ISO 16739) | 🚧 Phase 2+3 — STEP/P21 parser + EXPRESS schema typing (typed entity resolution over the core IFC entity set), full parameter grammar + Phase-3 tessellation (IfcTriangulatedFaceSet / IfcPolygonalFaceSet → Scene3D) + IfcLocalPlacement world-positioning (IfcBuildAxes placement chain) + §8.8.3.15 extruded swept solids (IfcExtrudedAreaSolid → prism), 5/5 fixtures; lacks revolved/curved solids | — |
| **Alembic** | 🚧 ~0% — Ogawa wire format docs-gapped per `docs/3d/alembic/GAP-TRACKER.md` | — |

Cross-format integration: `oxideav-cli-convert` exposes a 3D conversion path through `oxideav_meta::populate_mesh3d_registry` — `oxideav convert in.obj out.gltf` (or `--probe` for structural inspection). `crates/oxideav-tests/tests/mesh3d_*.rs` runs the cross-format roundtrip suite. The convert verb carries an ImageMagick-compatible op set (`-resize` / `-thumbnail` / `-extent` / `-monochrome` / `-roll` / `-define` …) plus a 3D→raster renderer (Gouraud + Phong, `-light` / `-camera` / `-projection` / `-fov`, debug render modes, `-aa N`). Black-box oracles cross-validate against Apple `usdzconvert` + Blender + assimp.

</details>

<details>
<summary><strong>Trackers</strong> (decode-only by design) (click to expand)</summary>

| Codec | Decode | Encode |
|-------|--------|--------|
| **MOD / STM / XM** | ✅ ~97% MOD · ~92% STM · ~90% XM — shared Paula/FT2 mixer + full effect sets + Ultimate SoundTracker 15-sample + Startrekker FLT8 layouts + STM E4x/E7x waveform control + XM fine-slide last-non-zero memory (E1/E2/EA/EB/X1/X2) + note-delay LFO/counter-reset consistency + Kxy=note-97 silence + E6x pattern-loop point reset on pattern transition + typed sample-header accessors + fuzz | — |
| **STM** (Scream Tracker v1) | ✅ ~85% — structural parse + shared-mixer playback; XM-parity effects (Gxy/Jxy/Bxy/Cxy/Exy/Hxy + 7xy tremolo + volume-slide variants); hard-pan LRRL | — |
| **XM** (FastTracker 2) | ✅ ~90% — structural parse + full playback; envelopes + fadeout + key-off; vibrato + tone porta + pattern jumps + fine/extra-fine porta + Exy/Kxy subcommands + volume-column slides | — |
| **S3M** | ✅ ~96% — stereo + full ST3 v3.20 effect set + per-channel effect memory + canonical 9-octave ST3 period table + Jxy note-index arpeggio + OPL2/AdLib instrument decode + YM3812 operator core + per-voice latched global volume (Vxx no longer rescales held notes); AdLib audio gated on OPL2 EG-rate docs gap | — |

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
| **`oxideav-videotoolbox`** | macOS / iOS | 🚧 H.264 + HEVC + ProRes + MJPEG + MPEG-2 + VP9 + MPEG-4 Pt 2 + AV1 (M3+) + VVC | 🚧 H.264 + HEVC + ProRes + MJPEG | Encoder knobs map onto VT session properties (bit rate / quality / profile / data-rate limits); PSNR_Y ~36-61 dB per codec. iOS links the frameworks via build.rs + `dlsym(RTLD_DEFAULT)`; macOS keeps the `dlopen` path; device-specific encoder gaps degrade gracefully via `kVTPropertyNotSupportedErr`. |
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
| **`oxideav-http`** | HTTP / HTTPS source driver | ✅ `http://` + `https://` via pure-Rust `ureq` + `rustls` + `webpki-roots`; Range-request seeking; `HttpConfig` policy + RFC 7233 Content-Range/200-fallback/416 handling + RFC 9110 If-Range strong-validator + Content-Length cross-checks + HTTP-date 3 forms (IMF-fixdate/rfc850/asctime) + multipart/byteranges reject + Retry-After surfacing + RFC 7230 §3.2.4 obs-fold normaliser + RFC 9110 §8.4/§12.5.3 content-coding refusal (identity-only negotiation + coded-response rejection) + §12.5.5 Vary content-negotiation stability check + `parse_headers` fuzz |
| **`oxideav-generator`** | Synthetic media source (`generate://...` URIs) + zero-input filters | ✅ audio synth (sine + chirp/FM/DTMF/multitone/ADSR/ringmod + 5-colour noise + `pwm` + `supersaw`/`saws` + `tremolo`/`trem`) + image (xc/gradient/pattern/fractal/plasma/noise/label + Perlin-2001 + Worley/cellular) + video (testsrc/smptebars/fractal_zoom/gradient_animate/zoneplate/`scroll` toroidal motion ground-truth) |
| **`oxideav-rtmp`** | RTMP ingest + push | ✅ Server + client; AMF0/AMF3 parser/builder; Enhanced-RTMP v1 video + v2 audio + ModEx; pluggable key-verification; `rtmp://` PacketSource; symmetric teardown + client `poll_event` + v2 `MultichannelConfig` (24 SMPTE 22.2 positions) + Multitrack body + §E FLV file writer + `FlvReader<R: Write>` + NetConnection capability negotiation + §7.1.6 Aggregate Message routed end-to-end (`send_aggregate` + `next_packet` + `poll_event`) + ModEx TimestampOffsetNano (ns timebase) + typed `MessageStreamKind` accessor + §5 protocol-control invariant validator + §5.3 Acknowledgement received-byte window + Enhanced-RTMP v2 ReconnectRequest (typed client event + tcUrl resolution) + AMF3 §3.12 externalizable-object decode via `register_externalizable` per-class handlers + typed Enhanced-RTMP VideoFrameType.Command (StartSeek/EndSeek) seek-command frames + Enhanced-RTMP v2 audio silence-message + VideoPacketType.MPEG2TSSequenceStart (av01 descriptor) + SequenceEnd typed on both pipelines + AMF0 complete serializable marker set (§2.15 Unsupported 0x0D / §2.17 XML Document 0x0F / §2.18 Typed Object 0x10 + avmplus 0x11 AMF3 bridge) |
| **`oxideav-sysaudio`** | Native audio output | ✅ Runtime-loaded backends (ALSA, PulseAudio, WASAPI, CoreAudio, OSS); CoreAudio + WASAPI real HAL latency; output-device enumeration; per-device routing API on all backends; `StreamRequest::buffer_frames` honoured; `Driver::preferred_format` introspection on WASAPI/CoreAudio/ALSA; functional OSS `/dev/dsp` via dlopen'd libc (S16_LE negotiation). BT-aware; falls back to software estimate if HAL unavailable. |
| **`oxideav-pipeline`** | Pipeline composition (source → transforms → sink) | ✅ JSON transcode-graph executor; pipelined multithreaded runtime + `Executor::with_channel_caps` + `with_max_queue_bytes` byte-ceiling + `Progress::elapsed_micros` + `packets_skipped` + `packets_read` (demuxer-cumulative; wedged-decoder signature) + EOF Progress retry ride-out + `Progress::packets_copied` sink-cumulative (source-vs-sink lag = one subtraction) |
| **`oxideav-scene`** | Time-based scene / composition model | 🚧 data model for PDF pages / RTMP streaming compositor / NLE timelines + per-frame `Sample` + animation-track composition + `RasterRenderer` (bg solid/gradient + Rect/Polygon + `ObjectKind::Vector`) + `ObjectKind::Group` nested + SVG 1.1 path-data (M/L/H/V/C/S/Q/T/Z + relative + A arc) + `ObjectKind::Image(Decoded)` RGBA8 + `Background::DecodedImage(Arc<VideoFrame>)` + audio-cue mixing into `RenderedFrame.audio` + typed PBR metallic-roughness `Material` + `Scene::materials` palette + glTF 2.0 `node` transform graph (TRS/matrix + flat hierarchy + world-matrix fold) |
| **`oxideav-audio-filter`** | Audio effects & conversions (streaming) | ✅ ~50 filters: classic + transient/spatial/restoration family + SlewLimiter + LR4 crossover + `true_peak_detector` + `state_variable` Chamberlin SVF + Criterion benchmark harness (7 scenarios) + `crest_factor_meter` + `stereo_correlation_meter` (Pearson coefficient, sliding-window) + `zero_crossing_rate` observer (per-channel sliding-window meter, `sign(0.0) = +1` defends against `f32::signum -0.0` phantom-crossing) + `dither` (TPDF/RPDF requantizer + error-feedback noise shaping) + complete staged EQ-cookbook biquad catalogue (constant-peak BPF + slope shelves) + parallel/New-York compressor (dry/wet blend) + band-limited rational resampler (ratio-scaled anti-alias prototype, ≥40 dB end-to-end alias rejection) — see crate README for the catalogue |
| **`oxideav-image-filter`** | Single-frame image effects (stateless) | ✅ 133 filter types / 185 factory names — `VoronoiTransform`/`ProximityFill` (exact nearest-feature) + `SignedDistanceField` (exact signed Euclidean DT) + Gabor + Niblack adaptive local-statistics threshold + `CurveInterpolation::NaturalCubic` + `CentripetalCatmullRom` + `ReinhardExtended` + Drago §4 adaptive-log tone-map (Ld_max cd/m² + exposure-independent log-average pre-scaling) — see crate README for the catalogue |
| **`oxideav-pixfmt`** | Pixel-format conversion + palette + dither | ✅ YUV↔RGB matrices (BT.601 / BT.709 / BT.2020 / BT.2100) + chroma subsampling + packed 4:2:2 (YUYV / UYVY) ↔ planar/RGB/RGBA + palette quantisation + Floyd-Steinberg dither + PQ + HLG + BT.1886 transfer functions + Porter-Duff alpha + `Ya8` (luma+alpha) + direct `NV12`/`NV21` ↔ `Rgb24`/`Rgba` + direct planar YUV ↔ planar YUV chroma resample (4:2:0/4:2:2/4:4:4 incl. JPEG full-range) + planar GBR(A) ↔ packed deep-RGB (`Gbrp10/12/14Le`↔`Rgb48Le`, `Gbrap10/12/14Le`↔`Rgba64Le`; bit-reorder + container shift) + BT.2020 NCL Table 4/5 anchor vectors + Criterion alpha bench |

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
| **WebVTT**          | ✅ | ✅ | Header, STYLE ::cue(.class), REGION, inline b/i/u/c/v/lang/ruby/timestamp + cue-settings round-trip + full REGION block + §4.1 NOTE comment-block round-trip + §3.4 cue identifier round-trip + §4.1/§3.3 strict signature + canonical timestamp enforcement + §6.4 HTML character-reference decoder (decimal / hex / 8 named) + §4.2.2 `&` / `<` / `>` escape on write |
| **MicroDVD**        | ✅ | ✅ | frame-based, `{y:b/i/u/s}`, `{c:$BBGGRR}`, `{f:family}` |
| **MPL2**            | ✅ | ✅ | decisecond timing, `/` italic, `\|` break |
| **MPsub**           | ✅ | ✅ | relative-start timing, `FORMAT=TIME`, `TITLE=`/`AUTHOR=` |
| **VPlayer**         | ✅ | ✅ | `HH:MM:SS:text`, end inferred |
| **PJS**             | ✅ | ✅ | frame-based, quoted body |
| **AQTitle**         | ✅ | ✅ | `-->> N` frame markers |
| **JACOsub**         | ✅ | ✅ | `\B/\I/\U`, `#TITLE`/`#TIMERES` headers |
| **RealText**        | ✅ | ✅ | HTML-like `<time>/<b>/<i>/<u>/<font>/<br/>` |
| **SubViewer 1/2**   | ✅ | ✅ | marker-based v1, `[INFORMATION]` header v2 |
| **TTML**            | ✅ | ✅ | W3C Timed Text, `<tt>/<head>/<styling>/<style>/<p>/<span>/<br/>`, tts:* styling + r171 IMSC 1.2: `<layout>` regions + `tts:textAlign` + 22 IR-unmodelled `tts:*` / `itts:*` style extras + 11 `ttp:*` / `ittp:*` parameter attrs + `HH:MM:SS:FF` / `<n>f` / `<n>t` against `ttp:frameRate` / `ttp:tickRate` + TTML2 §8.1.5 inline `tts:*` on `<p>` (modelled-attr wrap + ttml_p_extra canonical order) + §12.2.4 par/seq timeContainer timing + timed-span progressive reveal + TTML2 §10.2 complete styling-attribute vocabulary (44 tts:* round-trip byte-stable across style/region/inline-p) |
| **SAMI**            | ✅ | ✅ | Microsoft, `<SYNC Start=ms>` + `<STYLE>` CSS classes |
| **EBU STL**         | ✅ | ✅ | ISO/IEC 18041 binary GSI+TTI (text mode only; bitmap + colour variants deferred) |

**Advanced text (own crate)** — `oxideav-ass`:

| Format              | Decode | Encode | Notes |
|---------------------|:------:|:------:|-------|
| **ASS / SSA**       | ✅ | ✅ | Script Info (typed header accessors + WrapStyle→\q bridge) + V4+/V4 styles + full override-tag set rendered (borders / shadows / blur / clips / shear / karaoke / alignment) + typed font-metric/rotation tag family + typed \t animated-transform tag + typed event columns + [Fonts]/[Graphics] attachments; re-emit byte-identical |

**Bitmap-native (own crate)** — `oxideav-sub-image`:

| Format              | Decode | Encode | Notes |
|---------------------|:------:|:------:|-------|
| **PGS / HDMV** (`.sup`) | ✅ | ✅ | Blu-ray subtitle stream; PCS/WDS/PDS/ODS + RLE + YCbCr palette → RGBA + RLE codec property+negative sweep + PCS composition_state classified + routed to Packet keyframe flag + independent per-`palette_id` PDS slots within a display set (BD-ROM Part 3 §2.2.1.2.3 "Composition Segments indicate the Palette to be used") with PCS palette_id-driven render selection (fade/colour-change sets) |
| **DVB subtitles**   | ✅ | ✅ | ETSI EN 300 743 segments + §7.2.2 epoch state machine (cross-packet region/CLUT/object retention; normal-case deltas render) + 2/4/8-bit pixel-coded objects + §7.2.4 Y=0 full-transparency + character-coded objects + §7.2.5.1 CLUT-depth map-table application + §7.2.1 Display Definition window clip; encoder: full segment writers + 2/4/8-bit RLE + RGBA display-set encoder (PES-level), roundtrip-pinned |
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

The `oxideav-id3` crate parses ID3v2.2 / v2.3 / v2.4 tags (v2.2: complete §4 frame table with typed v2.2-only walkers + §3.1 compression-bit skip since r283; whole-tag
and per-frame unsync, extended header with **CRC-32 [ISO-3309]
verification and emission** since r153, v2.4 data-length indicator,
encrypted/compressed frames recorded as `Unknown` (v2.2 §4.20 CRM encrypted-meta frame now typed decode/encode/round-trip), **r161 v2.4 §3.4
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

The JSON job graph (executed by `oxideav-pipeline` via `oxideav run`;
the former `oxideav-job` crate was folded into the pipeline) is a
declarative way to describe multi-output transcode pipelines. A job is a JSON object: keys are output
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

- `scripts/update-crates.sh` — clones every missing OxideAV sibling AND fast-forwards already-cloned siblings to upstream tip via a single GraphQL call. Skips siblings whose upstream is already an ancestor of local HEAD and refuses to fast-forward when local commits have diverged, so in-progress work is preserved. Idempotent; safe to re-run.

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
