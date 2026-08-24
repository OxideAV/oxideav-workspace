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
| Audio     | aac, ape (payload-magic registry decode), flac, gsm, mp1, mp2, mp3, musepack (SV8 framework round trip, SV7 from-PCM chain, §9 seek rejoin, mpc7/mpc8 oracles), opus (encode→decode incl. DTX, via Ogg; registry pre-skip/gain/multistream + reduced output rates), speex (decode oracle + framework encoder roundtrip), vorbis, wma (wave-tag registry resolution + crafted v2 decode) |
| Video     | ffv1, h263 (direct picture API + registry tags/payload-magic + framework GOP round trip), mjpeg, mpeg1, mpeg4, theora, vp8 (both directions), vp9 (registry whole-GOP, chained default framing) |
| Container | WAV EXTENSIBLE (multi-channel float → typed `ChannelLayout`), MPEG-TS remux round-trip + §13818-1 conformance validation + completed stream_type map + hostile-PCR typed tally |
| 3D mesh   | cross-format roundtrip, encoder-option roundtrip, extras/skinning coverage, multi-material stress, registry lookup, glTF↔USDZ on the mesh3d 0.0.5 surface, plus Blender/assimp and USDZ reference oracles |
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
binary that no-ops when it is not installed.

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
