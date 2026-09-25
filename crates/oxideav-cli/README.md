# oxideav-cli

[![CI](https://github.com/OxideAV/oxideav-workspace/actions/workflows/ci.yml/badge.svg)](https://github.com/OxideAV/oxideav-workspace/actions/workflows/ci.yml) [![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](../../LICENSE)

Command-line frontend for oxideav

Part of the [oxideav](https://github.com/OxideAV/oxideav-workspace) framework — a
pure-Rust media transcoding and streaming stack. Every codec, container, and
filter is implemented from the spec — no C codec libraries linked or wrapped,
no `*-sys` crates. The optional hardware-acceleration crates
(`oxideav-videotoolbox` / `-audiotoolbox` / `-vaapi` / `-vdpau` / `-nvidia` /
`-vulkan-video`) bridge to OS-provided HW engines via `libloading` (runtime-
loaded, no compile-time link). Pass `--no-hwaccel` to opt out.

## Usage

`oxideav` is a binary (prebuilt tarballs on GitHub Releases; or
`cargo build -p oxideav-cli` in the workspace). Every codec, container
and filter the workspace links is registered at start-up through
`oxideav-meta`, so the subcommands below work on any format the
framework knows.

| Subcommand | What it does |
|---|---|
| `list` | compiled-in containers (D = demux, M = mux) and codec backends |
| `probe <input>` | container, metadata and per-stream layout |
| `info <codec>` | every backend of a codec: sides, flags, limits, option schemas |
| `remux <in> <out>` | stream copy into another container |
| `transcode <in> <out> [--codec-video X] [--codec-audio Y]` | decode → re-encode per stream |
| `convert <in> [-ops …] <out>` | ImageMagick-style image / media conversion (`oxideav-cli-convert`) |
| `run` / `validate` / `dry-run` | JSON-described pipeline jobs |
| `bench <codec>` | encode / decode throughput per backend |

### Images: HEIF / HEIC / AVIF

`.heic` / `.heif` / `.heics` / `.avif` files resolve to the `heif`
container (ISO/IEC 23008-12 via `oxideav-heif`; HEVC and AV1 items
decode through `oxideav-h265` / `oxideav-av1`). Files from Apple
ImageIO, libheif, ImageMagick and the AVIF muxer of the black-box
video tool — grids, alpha, 10/12-bit, 4:0:0–4:4:4, thumbnails,
rotation, image sequences — decode sample-exact with the library and
within each reader's own rounding of that reader's render (matrix in
[`crates/oxideav-tests/README.md`](../oxideav-tests/README.md#heif--heic--avif-through-the-cli-r462)).

```sh
# Layout: brands, primary item, the still stream + any image-sequence track
oxideav probe photo.heic
#   Format: heif
#   Metadata:
#       major_brand       : heic
#       compatible_brands : mif1,MiPr,miaf,MiHB,heic
#       primary_item_id   : 5
#       primary_item_type : grid
#     Stream #0 [Video]  codec=heif  ...  video 4032x3024  [Yuv420P]

# HEIC / AVIF → PNG (alpha, grayscale, >8-bit and Apple grids included);
# the PNG is sample-exact with the library decode after the pipeline's
# pixel-format step. >8-bit sources come out as 16-bit PNG.
oxideav convert photo.heic photo.png
oxideav convert photo.heic -resize 1024x768 photo.jpg

# Same through an explicit JSON job
oxideav run --inline '{"photo.png": {"video": [{"convert": "rgba",
    "input": {"from": "photo.heic"}, "codec": "png"}]}}'

# Image sequences (.heics / animated .avif): stream 0 is the cover still,
# stream 1 the `pict` track. `convert` refuses them today (both streams
# reach the single-image PNG muxer); pick the cover with a `run` job
oxideav probe burst.heics
oxideav run --inline '{"cover.png": {"video": [{"from": "burst.heics",
    "codec": "png", "stream_selector": {"kind": "video", "index": 0}}]}}'

# PNG → HEIC at the encoder defaults (HEVC intra qp 26). For AVIF use
# `transcode`: it infers `codec=av1` from the `.avif` extension, whereas
# `convert` has no encoder-option channel and would write HEVC (`heic`
# brand) under an .avif name
oxideav convert photo.png photo.heic
oxideav transcode photo.png photo.avif --codec-video heif

# Encoder options (`oxideav info heif` lists them): repeat
# --codec-option / -o KEY=VALUE
oxideav transcode photo.png photo.heic --codec-video heif \
    -o qp=20 -o grid=512 -o thumbnail=320
oxideav transcode photo.png photo.avif --codec-video heif \
    -o codec=av1 -o quality=80 -o speed=balanced
```

#### `heif` encoder options

| option | values | default | meaning |
|---|---|---|---|
| `codec` | `hevc` / `h265` / `av1` | `hevc` (`transcode` infers `av1` for an `.avif` target) | coded item codec: HEVC → `heic` brand, AV1 → `avif` brand |
| `mode` | `intra` / `pcm` | `intra` | HEVC: CABAC intra at `qp`, or PCM (lossless). AV1: `pcm` = lossless |
| `qp` | `0..=51` | `26` | HEVC intra quantiser (lower = better) |
| `quality` | `0..=100` | `60` | AV1 quality for `mode=intra` (`100` = lossless) |
| `speed` | `fast` / `balanced` / `thorough` | `fast` | AV1 search effort |
| `rd` | `0..=2` | encoder's still default | HEVC intra mode-decision effort — refused today ("requires the ctb option", which the `heif` encoder does not expose) |
| `tiles` | `CxR`, e.g. `2x2` | none | HEVC tile layout (columns × rows, 1..=8 each, more than one tile) — refused today, same cause as `rd` |
| `grid` | pixels (`0` = single item) | `0` | tile the picture into a `grid` item of this tile size (MIAF 64-px floor; Apple uses 512) |
| `thumbnail` | pixels (`0` = none) | `0` | add a `thmb` item whose largest side is this many pixels |
| `range` | `full` / `limited` | `full` | sample range signalled in `nclx` (and the HEVC VUI) |

Alpha, grayscale, 8/10/12/16-bit RGB and 4:2:0 / 4:4:4 planar inputs
are all accepted; RGB sources are coded as 4:2:0 YCbCr (AV1: 4:4:4),
so even `mode=pcm` / `quality=100` keep the chroma-subsampling
residue (~1/255 mean on the test signal) — there is no identity /
4:4:4 HEVC option yet. The item's `colr` is `nclx` full-range BT.601
(sRGB) unless `range=limited`. `convert` forwards no encoder options
today (only `-quality`, which the `heif` encoder does not read) — use
`transcode … --codec-option` or a `run` job (`"codec_params"`) for
anything beyond the defaults.

Decoded planes of full-range 8-bit files are labelled `YuvJ420P` /
`YuvJ444P`, so the pipeline's pixel-format step picks the full-range
matrix. Three decode-side gaps are visible through the CLI today and
are marked as such in the matrix: alpha (`Yuva*`) and >8-bit
(`Yuv*P10/12`) layouts have no full-range label, so those PNGs come
out limited-range (mean ≈ 7/255 off; the planes themselves are
exact); identity-matrix (GBR) items — libheif `-L` lossless files —
are labelled `YuvJ444P` and rendered through BT.601; and 4:2:0
pictures with an odd dimension are refused by the pixel-format step
(exit 3).

### Exit codes

Every command exits `0` on success, prints one `oxideav: <error>` line
on stderr otherwise, and never leaves a partial output behind:
`transcode` / `remux` write a same-directory part file
(`<name>.<pid>.oxideav-part`) that is renamed into place only on
success (a pre-existing output survives a failed run untouched);
`convert` / `run` unlink any output they created before failing.

| code | meaning |
|---|---|
| `0` | success |
| `1` | any other failure (`Error::Other`, an unexpected end of stream) |
| `2` | command-line usage error |
| `3` | invalid or truncated input data; a refused option value (`Error::InvalidData`) |
| `4` | unsupported feature or refused option (`Error::Unsupported`) |
| `5` | no container matched — unknown output extension or unprobeable input (`Error::FormatNotFound`) |
| `6` | no such codec / no usable backend (`Error::CodecNotFound`) |
| `7` | a decoder limit or pool cap fired (`Error::ResourceExhausted`) |
| `8` | input not found |
| `9` | permission denied on an input or output |
| `10` | any other I/O failure |

Whether a refused encoder option is `3` or `4` follows the codec
crate's own classification (an out-of-range enum is invalid data; a
layout the encoder cannot honour is unsupported).
`crates/oxideav-cli/tests/heif_failures.rs` pins the table on the
built binary; `tests/heif_hostile.rs` drives the heif fuzz corpus plus
truncated / bit-flipped producer files through `probe` and `convert`
and requires a typed exit (never a panic, abort or hang) for each.

## License

MIT — see [LICENSE](https://github.com/OxideAV/oxideav-workspace/blob/master/LICENSE).
