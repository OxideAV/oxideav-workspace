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
ImageIO, libheif and ImageMagick — grids, alpha, 10/12-bit,
thumbnails, image sequences — decode byte-exact against those readers.

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

# HEIC → PNG (alpha, grayscale, 10-bit and Apple grids included);
# the PNG is sample-exact with the library decode after the
# pipeline's pixel-format step
oxideav convert photo.heic photo.png
oxideav convert photo.heic -resize 1024x768 photo.jpg

# Same through an explicit JSON job
oxideav run --inline '{"photo.png": {"video": [{"convert": "rgba",
    "input": {"from": "photo.heic"}, "codec": "png"}]}}'

# Image sequences (.heics): stream 0 is the cover still, stream 1 the
# `pict` track of h265 packets
oxideav probe burst.heics

# The encoder side of the `heif` codec (options: codec=hevc|av1,
# mode=intra|pcm, qp, grid, thumbnail — see `crates/oxideav-heif/README.md`)
oxideav info heif
```

Writing HEIC from the CLI (`oxideav convert photo.png photo.heic`) is
not available yet: the `heif` muxer accepts `h265` / `av1` packets
(it writes an HEVC / AV1 image *sequence*) but not the `heif` codec's
whole-file still packets, and the encoder takes planar YCbCr input
only. `crates/oxideav-cli/tests/heif_images.rs` pins today's
diagnostic and validates the file with every third-party reader on
the host as soon as the muxer passes those packets through.

Note that the decoded planes of full-range files (every producer
above writes `colr` nclx full-range BT.601) are labelled `Yuv420P` /
`Yuv444P`, so the pipeline's pixel-format step renders them
narrow-range; a `YuvJ*` label from the demuxer would let it pick the
full-range matrix.

## License

MIT — see [LICENSE](https://github.com/OxideAV/oxideav-workspace/blob/master/LICENSE).
