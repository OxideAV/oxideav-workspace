# oxideav-tests

[![CI](https://github.com/OxideAV/oxideav-workspace/actions/workflows/ci.yml/badge.svg)](https://github.com/OxideAV/oxideav-workspace/actions/workflows/ci.yml) [![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](../../LICENSE)

Cross-crate integration test harness for the `oxideav` workspace. This
crate holds no production code — it exists so that codec, container,
3D-asset, and pipeline tests can depend on many sibling crates at once
without any of those crates taking a dev-dependency on its peers (which
would couple every consumer to its producer's publish cadence).

`publish = false`; it never ships to crates.io.

## What it tests

The suites compare our encoders/decoders against external reference
binaries (invoked as black-box oracles) and against each other. Every
test that needs an external tool skips gracefully when the tool is
absent, so the suite stays green on a bare checkout — and most suites
also carry an oracle-free self-roundtrip leg (our encoder → our
container → our decoder) that runs everywhere.

| Area      | Coverage |
| --------- | -------- |
| Audio     | aac (LC oracles + HE-AAC v1 direct/registry-ASC encode → registry decode), ape (payload-magic registry decode; docs corpus byte-exact via `oxideav_meta::register_all`), flac, g729 (registry 8 kbit/s round trip; Annex B SID/no-transmission packets pinned as not yet routed), gsm, mp1, mp2, mp3, musepack (SV8 framework round trip, SV7 from-PCM chain, §9 seek rejoin, mpc7/mpc8 oracles), opus (encode→decode incl. DTX, via Ogg; registry pre-skip/gain/multistream + reduced output rates), speex (decode oracle + framework encoder roundtrip), vorbis, wma (wave-tag registry resolution + crafted v2 decode) |
| Video     | evc (B-slice / multi-ref option round trips), ffv1, h263 (direct picture API + registry tags/payload-magic + framework GOP round trip), mjpeg, mpeg1, mpeg4 (oracle legs + registry `mb-aq` / `packet-bits` / `rvlc` / `bf` option round trips), theora, vp8 (both directions), vp9 (registry whole-GOP, chained default framing, `Vp9GopConfig` alt-ref/segmentation → registry decode, `Yuv440P` synthetic + docs 4:4:0 fixtures) |
| Container | AMV (both device-profile fixtures: registry probe → demux → `amv_video` / `adpcm_amv` decode; registry encoders → muxer → demux → decode), WAV EXTENSIBLE (multi-channel float → typed `ChannelLayout`), MPEG-TS remux round-trip + §13818-1 conformance validation + completed stream_type map + hostile-PCR typed tally |
| Image     | heif / heic / avif (round 462: the CLI path both directions — see below; round 460: registry resolution audit across every HEIF-family brand vs `mp4` / `mov`; all 39 vendored producer files — Apple ImageIO, libheif, ImageMagick — plus the 13 corpus bundles decode through `register_all`, 26 byte-exact against the black-box video decoder, PNG oracles within the reader's rounding, the image-sequence track frame by frame; a fresh 30-file producer matrix via `sips` / `heif-enc` / `magick` incl. 4032×3024 Apple grids, 10/12-bit, alpha, lossless, thumbnails, AVIF — SKIP without the binaries), jpeg2000 (registry encoder: packed formats, `container=jp2/jph`, lossy/structure options → registry decode), openexr (core 0.1.35 F32 family byte-exact via `register_all`) |
| Core      | `PixelFormat` plane-geometry pins for the 4:4:0 ladder and the F32 family |
| 3D mesh   | cross-format roundtrip, encoder-option roundtrip, extras/skinning coverage, multi-material stress, registry lookup, glTF↔USDZ on the mesh3d 0.0.5 surface, mesh3d 0.0.6 morph surface (target names, in-betweens, sampled `MorphWeights`) across glTF/USDZ/FBX, plus Blender/assimp and USDZ reference oracles |
| Pipeline  | wav roundtrip, codec parity, pixel-format conversion |

The typical codec test follows one shape:

1. Generate a deterministic test signal in-process (no checked-in fixtures).
2. **Encoder direction** — encode with ours, decode with the reference,
   and compare against a reference-only encode/decode of the same input.
3. **Decoder direction** — encode with the reference, decode with ours,
   and compare against the reference's own decode.

Comparisons use the shared metrics in `src/lib.rs` (audio RMS/PSNR,
Y-plane PSNR for video) so each suite asserts a fidelity threshold
rather than byte equality where the formats are lossy.

## Shared helpers

`src/lib.rs` exposes the reusable building blocks: deterministic signal
generators, raw-PCM and YUV420P readers/writers, RMS / PSNR metrics, a
temp-path helper, and a guarded wrapper around the external reference
binary that no-ops when it is not installed. The `image` module adds
the still-image helpers the HEIF suites share: registry decode of any
still (HEIF / AVIF / PNG …) to its first `VideoFrame`, the pipeline's
own pixel-format step, a straight H.273 reference render for oracle
comparisons, FNV-1a plane fingerprints, and packed-plane comparison.

The CLI-level HEIF tests (spawning the built `oxideav` binary) live in
`crates/oxideav-cli/tests/heif_images.rs`, where `CARGO_BIN_EXE_oxideav`
is available.

## HEIF / HEIC / AVIF through the CLI (r462)

The user-facing path — `oxideav probe` / `convert` / `transcode` on
the built binary — is proven both directions in
`crates/oxideav-cli/tests/heif_matrix.rs` (the tables below are that
suite's `--nocapture` output on the round-462 host: macOS 26.6,
Apple ImageIO `sips`, libheif 1.20 `heif-enc` / `heif-convert` /
`heif-info`, ImageMagick 7 `magick`, the black-box video tool 8.0 with
SVT-AV1 4.2 and x265). Legs whose producer or reader is absent print
`SKIP`; the vendored-file leg runs unconditionally on CI.

Cells read **max/mean** absolute difference in 8-bit units over the
RGB channels (alpha reported separately when it differs; `exact` =
every sample identical). "vs library decode" compares the CLI's PNG
with the library decode pushed through the pipeline's own
pixel-format step (`oxideav_pixfmt::convert`) — this is the
sample-exactness contract, and it holds for every file below. "vs
producer render" compares our PNG with the producer's own reader
(`sips`, `heif-convert`, `magick`, the video tool) rendering the same
file: lossless files are exact; lossy ones differ only by each
reader's YCbCr → RGB rounding and chroma upsampling filter (mean ≤ 3.0
is the assertion; the max lands on chroma edges of the 4:2:0 synthetic
checkerboard). Sources are synthesised in-process and written as PNG
through the registry's own encoder; >8-bit inputs are 16-bit PNG.

**Summary.** 52 vendored inputs + 45 fresh producer cases decode
through `oxideav convert` **sample-exact with the library** in every
cell that decodes (no exceptions); 29 CLI-written HEIC / AVIF files
open in `heif-info`, `sips`, `heif-convert`, `magick` and the video
tool, and every reader renders them within its own rounding of our
decode wherever the pipeline carries the colour signalling. The cells
marked as **known gaps** are sibling-crate limitations the CLI path
exposes (each is an ask in the round-462 report), not decode errors —
the decoded planes fit the reference under the right H.273
interpretation (the "planes fit … " tail of the cell):

| gap | where it shows | cells |
|---|---|---|
| full range lost for alpha (`Yuva*`) and >8-bit (`Yuv*P10/12`) layouts — no `J` label / range field, so the pipeline's pixel-format step renders limited range (mean ≈ 7/255 off) | every alpha and 10/12-bit still, from every producer, and our own alpha writes | 8 + 8 + 14 |
| identity-matrix (GBR) 4:4:4 items labelled `YuvJ444P`, rendered through BT.601 | libheif `-L` lossless HEIC / AVIF | 2 + 6 |
| 4:2:0 / 4:2:2 → RGB refused when a dimension is odd (exit 3) | 63×61 alpha AVIF (libheif, ImageMagick), 7×5 AVIF (libheif, video tool) | 2 + 4 |
| image sequences: `convert` feeds the cover *and* the track to the PNG muxer (exit 4); a `run` job with `stream_selector` writes the cover | `.heics`, corpus `image-sequence-3frame`, the video tool's `avis` | 2 + 1 |
| `convert` has no encoder-option channel: `out.avif` is written as HEVC (`heic` brand); `transcode` infers `codec=av1` | `convert photo.png photo.avif` | 1 |
| `rd` / `tiles` refused ("require the ctb option") | `transcode … -o rd=2`, `-o tiles=2x2` | 2 (refused rows) |

Tiny pictures (7×5, 1×1) are reported against the source but not
asserted (chroma subsampling dominates); the readers still agree with
our decode of them exactly.

#### Vendored files → `oxideav convert` → PNG (52 inputs)

| file | layout | vs library decode | vs producer render (max/mean, 8-bit) |
|---|---|---|---|
| henc_10bit_rgb16_96x80.heic | Yuv420P10Le 96x80 | exact | 18/7.46 — full range lost (alpha / >8-bit); planes fit BT.601 full: 1/0.19 |
| henc_12bit_rgb16_96x80.heic | Yuv420P12Le 96x80 | exact | no oracle |
| henc_422_rgb_96x80.heic | YuvJ422P 96x80 | exact | 1/0.00 |
| henc_444_rgb_96x80.heic | YuvJ444P 96x80 | exact | 1/0.00 |
| henc_avif_10bit_rgb16_96x80.avif | Yuv420P10Le 96x80 | exact | no oracle |
| henc_avif_444_rgb_96x80.avif | YuvJ444P 96x80 | exact | no oracle |
| henc_avif_gray_96x80.avif | Gray8 96x80 | exact | no oracle |
| henc_avif_lossless_rgb_96x80.avif | YuvJ444P 96x80 | exact | no oracle |
| henc_avif_rgb_96x80.avif | YuvJ420P 96x80 | exact | 1/0.02 |
| henc_avif_rgba_63x61.avif | — | refused — odd 4:2:0 refused by pixfmt | — |
| henc_gray16_96x80.heic | Gray10Le 96x80 | exact | 1/0.33 |
| henc_gray_96x80.heic | Gray8 96x80 | exact | exact |
| henc_lossless10_444_rgb16_96x80.heic | Yuv444P10Le 96x80 | exact | no oracle |
| henc_lossless_rgb_96x80.heic | YuvJ444P 96x80 | exact | 135/44.65 — identity matrix rendered as BT.601; planes fit identity full: exact |
| henc_lossless_rgba_63x61.heic | Yuva444P 63x61 | exact | 136/42.97 — identity matrix rendered as BT.601; planes fit identity full: exact |
| henc_rgb_63x61.heic | YuvJ444P 63x61 | exact | 1/0.02 |
| henc_rgb_7x5.heic | YuvJ444P 7x5 | exact | exact |
| henc_rgb_96x80.heic | YuvJ420P 96x80 | exact | no oracle |
| henc_rgba_80x64.heic | Yuva420P 80x64 | exact | 17/6.36 — full range lost (alpha / >8-bit); planes fit BT.601 full: 1/0.72 |
| henc_thumb_rgb_96x80.heic | YuvJ420P 96x80 | exact | no oracle |
| magick_12bit_rgb16_96x80.heic | Yuv420P12Le 96x80 | exact | no oracle |
| magick_avif_rgb_96x80.avif | YuvJ420P 96x80 | exact | no oracle |
| magick_avif_rgba_63x61.avif | — | refused — odd 4:2:0 refused by pixfmt | — |
| magick_gray16_96x80.heic | Gray12Le 96x80 | exact | 1/0.25 |
| magick_rgb_63x61.heic | YuvJ444P 63x61 | exact | 1/0.02 |
| magick_rgb_96x80.heic | YuvJ420P 96x80 | exact | no oracle |
| magick_rgba_80x64.heic | Yuva420P 80x64 | exact | 17/6.36 — full range lost (alpha / >8-bit); planes fit BT.601 full: 1/0.72 |
| sips_gray16_96x80.heic | Yuv420P10Le 96x80 | exact | 22/7.15 — full range lost (alpha / >8-bit); planes fit BT.601 full: 1/0.24 |
| sips_gray_96x80.heic | YuvJ420P 96x80 | exact | no oracle |
| sips_grid_1024x768.heic | YuvJ420P 1024x768 | exact | no oracle |
| sips_q100_rgb_96x80.heic | YuvJ444P 96x80 | exact | 1/0.00 |
| sips_rgb16_96x80.heic | Yuv420P10Le 96x80 | exact | no oracle |
| sips_rgb_1x1.heic | YuvJ444P 1x1 | exact | exact |
| sips_rgb_63x61.heic | YuvJ444P 63x61 | exact | 1/0.03 |
| sips_rgb_7x5.heic | YuvJ444P 7x5 | exact | exact |
| sips_rgb_96x80.heic | YuvJ420P 96x80 | exact | 1/0.02 |
| sips_rgba_63x61.heic | Yuva444P 63x61 | exact | 19/6.35 — full range lost (alpha / >8-bit); planes fit BT.601 full: 1/0.70 |
| sips_rgba_80x64.heic | Yuva420P 80x64 | exact | 17/6.38 — full range lost (alpha / >8-bit); planes fit BT.601 full: 1/0.70 |
| sips_seq_rgb_96x80.heics | — | refused — sequence: 2 streams into the PNG muxer | — |
| corpus/image-sequence-3frame | — | refused — sequence: 2 streams into the PNG muxer | — |
| corpus/multi-image-burst-3 | YuvJ420P 96x96 | exact | 1/0.02 |
| corpus/single-image-1x1 | YuvJ444P 1x1 | exact | exact |
| corpus/single-image-with-thumbnail | YuvJ420P 256x256 | exact | 1/0.03 |
| corpus/still-10bit-main10 | Yuv420P10Le 128x128 | exact | 22/8.20 — full range lost (alpha / >8-bit); planes fit BT.601 full: 1/0.24 |
| corpus/still-image-grid-2x2 | YuvJ420P 256x256 | exact | exact |
| corpus/still-image-overlay | YuvJ444P 256x256 | exact | 2/0.01 |
| corpus/still-image-with-alpha | Yuva420P 128x128 | exact | 15/10.00 — full range lost (alpha / >8-bit); planes fit BT.601 full: 1/0.83 |
| corpus/still-image-with-exif | YuvJ420P 256x256 | exact | 1/0.03 |
| corpus/still-image-with-icc | YuvJ420P 256x256 | exact | 1/0.03 |
| corpus/still-image-with-xmp | YuvJ420P 256x256 | exact | 1/0.03 |
| corpus/still-monochrome | Gray8 128x128 | exact | exact |
| corpus/still-yuv444 | YuvJ444P 128x128 | exact | exact |

#### Fresh producer files → `oxideav convert` → PNG

| producer / case | layout | vs library decode | vs source PNG (max/mean) | vs producer's own render |
|---|---|---|---|---|
| sips rgb_1x1 | YuvJ444P 1x1 | exact | 2/1.33 | sips exact |
| sips rgb_7x5 | YuvJ444P 7x5 | exact | 80/19.58 | sips exact |
| sips rgb_63x61 | YuvJ444P 63x61 | exact | 15/2.35 | sips 1/0.39 |
| sips rgb_96x80 | YuvJ420P 96x80 | exact | 10/1.54 | sips 11/1.53 |
| sips q=best rgb_96x80 | YuvJ444P 96x80 | exact | 1/0.33 | sips 1/0.25 |
| sips rgba_63x61 | Yuva444P 63x61 | exact | 28/8.17 α2 | sips 20/7.39 — full range lost (alpha / >8-bit); planes fit BT.601 full: 2/0.89 |
| sips gray_96x80 | YuvJ420P 96x80 | exact | 4/0.61 | sips exact |
| sips 16-bit in rgb16_96x80 | Yuv420P10Le 96x80 | exact | 21/7.31 | sips 21/7.25 — full range lost (alpha / >8-bit); planes fit BT.601 full: 0/0.18 |
| sips grid rgb_1024x768 | YuvJ420P 1024x768 | exact | 8/0.86 | sips 10/0.46 |
| sips grid rgb_4032x3024 | YuvJ420P 4032x3024 | exact | 10/0.72 | sips 10/0.30 |
| heif-enc q60 rgb_1x1 | YuvJ444P 1x1 | exact | 1/0.33 | heif-convert exact |
| heif-enc q60 rgb_7x5 | YuvJ444P 7x5 | exact | 45/16.64 | heif-convert 1/0.02 |
| heif-enc q60 rgb_96x80 | YuvJ420P 96x80 | exact | 9/1.40 | heif-convert 1/0.03 |
| heif-enc lossless rgb_96x80 | YuvJ444P 96x80 | exact | 127/40.47 — identity matrix rendered as BT.601; planes fit identity full: exact | heif-convert 127/40.47 — identity matrix rendered as BT.601; planes fit identity full: exact |
| heif-enc lossless rgba_63x61 | Yuva444P 63x61 | exact | 141/44.48 — identity matrix rendered as BT.601; planes fit identity full: exact | heif-convert 141/44.48 — identity matrix rendered as BT.601; planes fit identity full: exact |
| heif-enc thumb rgb_96x80 | YuvJ420P 96x80 | exact | 9/1.40 | heif-convert 1/0.03 |
| heif-enc 10-bit rgb16_96x80 | Yuv420P10Le 96x80 | exact | 22/7.39 | heif-convert 21/7.32 — full range lost (alpha / >8-bit); planes fit BT.601 full: 1/0.25 |
| heif-enc 12-bit rgb16_96x80 | Yuv420P12Le 96x80 | exact | 24/7.42 | heif-convert 21/7.30 — full range lost (alpha / >8-bit); planes fit BT.601 full: 0/0.06 |
| heif-enc 4:4:4 rgb_96x80 | YuvJ444P 96x80 | exact | 6/1.02 | heif-convert 1/0.00 |
| heif-enc 4:2:2 rgb_96x80 | YuvJ422P 96x80 | exact | 8/1.35 | heif-convert 1/0.00 |
| heif-enc gray gray_96x80 | Gray8 96x80 | exact | 4/0.48 | heif-convert exact |
| heif-enc rgba rgba_63x61 | Yuva444P 63x61 | exact | 23/7.41 α1 | heif-convert 20/7.22 — full range lost (alpha / >8-bit); planes fit BT.601 full: 1/0.70 |
| heif-enc rotate 90 rgb_96x80 | YuvJ420P 80x96 | exact | rotated 90° | heif-convert 1/0.03 |
| heif-enc limited range rgb_96x80 | Yuv420P 96x80 | exact | 9/1.48 | heif-convert 1/0.49 |
| heif-enc single rgb_4032x3024 | YuvJ420P 4032x3024 | exact | 6/0.44 | heif-convert 1/0.03 |
| heif-enc avif rgb_7x5 | — | refused — odd 4:2:0 refused by pixfmt | — | — |
| heif-enc avif rgb_96x80 | YuvJ420P 96x80 | exact | 9/1.58 | heif-convert 1/0.03 |
| heif-enc avif lossless rgb_96x80 | YuvJ444P 96x80 | exact | 127/40.47 — identity matrix rendered as BT.601; planes fit identity full: exact | heif-convert 127/40.47 — identity matrix rendered as BT.601; planes fit identity full: exact |
| heif-enc avif rgba rgba_63x61 | — | refused — odd 4:2:0 refused by pixfmt | — | — |
| heif-enc avif gray gray_96x80 | Gray8 96x80 | exact | 2/0.51 | heif-convert exact |
| heif-enc avif 10-bit rgb16_96x80 | Yuv420P10Le 96x80 | exact | 25/7.47 | heif-convert 21/7.33 — full range lost (alpha / >8-bit); planes fit BT.601 full: 1/0.25 |
| heif-enc avif 4:4:4 rgb_96x80 | YuvJ444P 96x80 | exact | 6/0.99 | heif-convert 1/0.00 |
| heif-enc avif rgb_4032x3024 | YuvJ420P 4032x3024 | exact | 8/0.54 | heif-convert 1/0.03 |
| magick q60 rgb_7x5 | YuvJ444P 7x5 | exact | 45/16.64 | magick 1/0.02 |
| magick q60 rgb_96x80 | YuvJ420P 96x80 | exact | 9/1.40 | magick 1/0.03 |
| magick q60 rgba_63x61 | Yuva444P 63x61 | exact | 23/7.41 α1 | magick 20/7.22 — full range lost (alpha / >8-bit); planes fit BT.601 full: 1/0.70 |
| magick q60 gray_96x80 | YuvJ420P 96x80 | exact | 4/0.48 | magick exact |
| magick 12-bit rgb16_96x80 | Yuv420P12Le 96x80 | exact | 23/7.44 | magick 21/7.29 — full range lost (alpha / >8-bit); planes fit BT.601 full: 0/0.06 |
| magick avif rgb_96x80 | YuvJ420P 96x80 | exact | 9/1.58 | magick 1/0.03 |
| magick avif rgba_63x61 | — | refused — odd 4:2:0 refused by pixfmt | — | — |
| ffmpeg avif crf30 rgb_96x80 | Yuv420P 96x80 | exact | 12/2.25 | ffmpeg 1/0.34 |
| ffmpeg avif crf30 rgb_7x5 | — | refused — odd 4:2:0 refused by pixfmt | — | — |
| ffmpeg avif 10-bit rgb16_96x80 | Yuv420P10Le 96x80 | exact | 11/2.01 | ffmpeg 5/1.14 |
| ffmpeg avif gray gray_96x80 | Yuv420P 96x80 | exact | 7/1.63 | ffmpeg 1/0.13 |
| ffmpeg avis sequence (3 frames) | probe: 2 streams | refused — sequence: 2 streams into the PNG muxer | `run` + stream_selector: cover written | — |

failures:

failures:
    cli_written_files_open_in_every_reader


error: test failed, to rerun pass `-p oxideav-cli --test heif_matrix`
EXIT 101

#### CLI-written HEIC / AVIF → readers (reader render vs our decode, max/mean 8-bit)

| case | layout | brand | our decode vs source (max/mean) | heif-info | sips | heif-convert | magick | ffmpeg |
|---|---|---|---|---|---|---|---|---|
| `transcode` hevc defaults · rgb_96x80 | YuvJ420P 96x80 | heic | 18/2.56 | opens | 15/1.58 | 1/0.03 | 1/0.03 | 1/0.08 (α dropped) |
| `transcode` hevc defaults · rgb_7x5 | YuvJ444P 7x5 | heic | 52/16.97 | opens | exact | 1/0.02 | 1/0.02 | 1/0.28 (reader crops to 6x4) |
| `transcode` hevc defaults · rgb_1x1 | YuvJ444P 1x1 | heic | 1/0.33 | opens | exact | exact | exact | refused (reader: 1-px picture) |
| `transcode` hevc defaults · rgba_63x61 | Yuva444P 63x61 | heic | 31/7.73 α2 | opens | 20/7.36 — full range lost (alpha / >8-bit); planes fit BT.601 full: 2/0.89 | 20/7.23 — full range lost (alpha / >8-bit); planes fit BT.601 full: 1/0.70 | 20/7.23 — full range lost (alpha / >8-bit); planes fit BT.601 full: 1/0.70 | 20/7.18 α204 (reader crops to 62x60) range-lost |
| `transcode` hevc defaults · gray_96x80 | YuvJ420P 96x80 | heic | 13/1.81 | opens | exact | exact | exact | exact (α dropped) |
| `transcode` hevc defaults · rgb16_96x80 | YuvJ420P 96x80 | heic | 18/2.61 | opens | 13/1.57 | 1/0.03 | 1/0.03 | 1/0.08 (α dropped) |
| `transcode` qp=10 · rgb_96x80 | YuvJ420P 96x80 | heic | 9/1.18 | opens | 10/1.51 | 1/0.04 | 1/0.04 | 1/0.08 (α dropped) |
| `transcode` qp=40 · rgb_96x80 | YuvJ420P 96x80 | heic | 31/6.72 | opens | 17/1.58 | 1/0.04 | 1/0.04 | 1/0.08 (α dropped) |
| `transcode` mode=pcm · rgb_96x80 | YuvJ420P 96x80 | heic | 5/1.04 | opens | 8/1.51 | 1/0.04 | 1/0.04 | 1/0.08 (α dropped) |
| `transcode` mode=pcm · rgba_63x61 | Yuva444P 63x61 | heic | 22/7.40 — full range lost (alpha / >8-bit); planes fit BT.601 full: 6/1.60 | opens | 20/7.31 — full range lost (alpha / >8-bit); planes fit BT.601 full: 2/0.88 | 20/7.20 — full range lost (alpha / >8-bit); planes fit BT.601 full: 1/0.70 | 20/7.20 — full range lost (alpha / >8-bit); planes fit BT.601 full: 1/0.70 | 20/7.15 α204 (reader crops to 62x60) range-lost |
| `transcode` grid=64 · rgb_96x80 | YuvJ420P 96x80 | heic | 18/2.51 | opens | 15/1.51 | 1/0.03 | 1/0.03 | 1/0.08 (α dropped) |
| `transcode` grid=512 · rgb_1024x768 | YuvJ420P 1024x768 | heic | 18/2.56 | opens | 17/0.51 | 1/0.04 | 1/0.04 | 1/0.09 (α dropped) |
| `transcode` thumbnail=32 · rgb_96x80 | YuvJ420P 96x80 | heic | 18/2.56 | opens | 15/1.58 | 1/0.03 | 1/0.03 | 1/0.08 (α dropped) |
| `transcode` range=limited · rgb_96x80 | Yuv420P 96x80 | heic | 18/2.72 | opens | 13/1.62 | 1/0.51 | 1/0.51 | 1/0.33 (α dropped) |
| `transcode` rd=2 · rgb_96x80 | refused: the sdh / rdoq / tudepth / sl / wp / wpp / tiles / rd options require the ctb option |
| `transcode` tiles=2x2 · rgb_1024x768 | refused: the sdh / rdoq / tudepth / sl / wp / wpp / tiles / rd options require the ctb option |
| `transcode` codec=av1 · rgb_96x80 | YuvJ444P 96x80 | avif | 19/3.58 | opens | 1/0.25 | 1/0.00 | 1/0.00 | 1/0.00 (α dropped) |
| `transcode` codec=av1 · rgba_63x61 | Yuva444P 63x61 | avif | 35/8.39 α2 | opens | 20/7.43 — full range lost (alpha / >8-bit); planes fit BT.601 full: 2/0.91 | 20/7.25 — full range lost (alpha / >8-bit); planes fit BT.601 full: 1/0.70 | 20/7.25 — full range lost (alpha / >8-bit); planes fit BT.601 full: 1/0.70 | 20/7.25 α202 — full range lost (alpha / >8-bit); planes fit BT.601 full: 1/0.70 α202 (α dropped) |
| `transcode` codec=av1 · gray_96x80 | Gray8 96x80 | avif | 17/3.02 | opens | exact | exact | exact | exact (α dropped) |
| `transcode` codec=av1 · rgb16_96x80 | YuvJ420P 96x80 | avif | 21/4.36 | opens | 10/1.51 | 1/0.04 | 1/0.04 | 1/0.08 (α dropped) |
| `transcode` codec=av1 quality=100 · rgb_96x80 | YuvJ444P 96x80 | avif | 1/0.33 | opens | 1/0.24 | 1/0.00 | 1/0.00 | 1/0.00 (α dropped) |
| `transcode` codec=av1 quality=30 · rgb_96x80 | YuvJ444P 96x80 | avif | 25/5.40 | opens | 1/0.24 | 1/0.00 | 1/0.00 | 1/0.00 (α dropped) |
| `transcode` codec=av1 speed=thorough · rgb_96x80 | YuvJ444P 96x80 | avif | 7/1.05 | opens | 1/0.24 | 1/0.00 | 1/0.00 | 1/0.00 (α dropped) |
| `transcode` codec=av1 mode=pcm · rgb_96x80 | YuvJ444P 96x80 | avif | 1/0.33 | opens | 1/0.24 | 1/0.00 | 1/0.00 | 1/0.00 (α dropped) |
| `transcode` codec=av1 grid=64 thumbnail=32 · rgb_96x80 | YuvJ444P 96x80 | avif | 19/3.32 | opens | 1/0.25 | 1/0.00 | 1/0.00 | 1/0.00 (α dropped) |
| `transcode` .avif infers codec=av1 · rgb_96x80 | YuvJ444P 96x80 | avif | 19/3.58 | opens | 1/0.25 | 1/0.00 | 1/0.00 | 1/0.00 (α dropped) |
| `convert` defaults .heic · rgb_96x80 | YuvJ420P 96x80 | heic | 18/2.56 | opens | 15/1.58 | 1/0.03 | 1/0.03 | 1/0.08 (α dropped) |
| `convert` defaults .heic · rgba_63x61 | Yuva444P 63x61 | heic | 31/7.73 α2 | opens | 20/7.36 — full range lost (alpha / >8-bit); planes fit BT.601 full: 2/0.89 | 20/7.23 — full range lost (alpha / >8-bit); planes fit BT.601 full: 1/0.70 | 20/7.23 — full range lost (alpha / >8-bit); planes fit BT.601 full: 1/0.70 | 20/7.18 α204 (reader crops to 62x60) range-lost |
| `convert` defaults .avif · rgb_96x80 | YuvJ420P 96x80 | heic — `convert` writes .avif as HEVC (heic brand) | 18/2.56 | opens | 15/1.58 | 1/0.03 | 1/0.03 | 1/0.08 (α dropped) |

### HEIF performance baseline (r462)

Release build (`cargo build --release -p oxideav-cli`, rustc 1.98.1),
Apple M4 Max / 64 GiB, macOS 26.6.2, `/usr/bin/time -l`, second of
two runs (the first is within 0.1 s except the cold grid decode at
6.2 s). Inputs: a 4032×3024 `plasma:fractal` PNG (62.9 MB, 24-bit)
turned into an Apple `sips` grid HEIC (6.2 MB, 48 × 512-px tiles), a
libheif single-item HEIC (`-q 60`, 3.7 MB) and a libheif AVIF (`-A -q
60`, 0.7 MB). PNG decode / encode time is included in every row (it is
what the user pays); the third-party column is the same host running
the producer's own tool on the same file, for scale only.

| conversion | `oxideav` wall | user | peak RSS | output | reference tool on the same file |
|---|---|---|---|---|---|
| Apple grid HEIC 4032×3024 → PNG | **5.39 s** | 5.35 s | 465 MiB | 22.5 MB PNG | `sips` 1.76 s / 261 MiB |
| single-item HEIC 4032×3024 → PNG | **5.27 s** | 5.20 s | 605 MiB | 21.8 MB PNG | `heif-convert` 12.07 s / 159 MiB |
| AVIF 4032×3024 → PNG | **4.70 s** | 4.67 s | 369 MiB | 16.3 MB PNG | `heif-convert` 13.10 s / 222 MiB |
| PNG 4032×3024 → HEIC (`convert`, defaults) | **3.39 s** | 3.34 s | 627 MiB | 0.44 MB | `sips` 0.31 s / 249 MiB · `heif-enc -q 60` 2.00 s / 455 MiB · `magick` 1.52 s / 551 MiB |
| PNG 4032×3024 → HEIC (`transcode --codec-video heif`) | **3.40 s** | 3.35 s | 687 MiB | 0.44 MB | as above |
| PNG 4032×3024 → AVIF (`transcode --codec-video heif`, infers `codec=av1`, `speed=fast`) | **80.95 s** | 80.77 s | 915 MiB | 0.31 MB | `heif-enc -A -q 60` 0.42 s / 410 MiB |
| `probe` of the grid HEIC | 0.03 s | 0.03 s | 24 MiB | — | — |

Decode is single-threaded (user ≈ wall); the grid decodes 48 tiles
serially. The AV1 still encoder at its default effort is the outlier
(~190× the reference producer); HEVC encode at defaults produces a
file ~8× smaller than the references' `q 60`, i.e. not the same
operating point — the quality/size trade-off is the optimisation
rounds' first question, this table is the baseline they measure
against.


## Running

```sh
cargo test -p oxideav-tests
```

Tests requiring an external oracle are skipped automatically when the
oracle is not present on the host. The reference binary is looked up
at `/usr/bin/ffmpeg` (the CI runners' location); set `OXIDEAV_FFMPEG`
to point at a different install prefix to run the oracle legs on a
developer machine:

```sh
OXIDEAV_FFMPEG=$(which ffmpeg) cargo test -p oxideav-tests
```

## License

MIT, matching the workspace. Part of the
[oxideav](https://github.com/OxideAV) workspace.
