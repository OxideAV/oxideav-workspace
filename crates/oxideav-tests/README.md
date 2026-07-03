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
absent, so the suite stays green on a bare checkout.

| Area     | Coverage |
| -------- | -------- |
| Audio    | aac, flac, gsm, mp1, mp2, mp3, opus, speex, vorbis |
| Video    | ffv1, h263, mjpeg, mpeg1, mpeg4, theora, vp8 |
| 3D mesh  | cross-format roundtrip, encoder-option roundtrip, extras/skinning coverage, multi-material stress, registry lookup, plus Blender/assimp and USDZ reference oracles |
| Pipeline | wav roundtrip, codec parity, pixel-format conversion |

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
binary that no-ops when it is not installed.

## Running

```sh
cargo test -p oxideav-tests
```

Tests requiring an external oracle are skipped automatically when the
oracle is not present on the host.

## License

MIT, matching the workspace. Part of the
[oxideav](https://github.com/OxideAV) workspace.
