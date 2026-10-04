# AAC container fixtures

Small AAC files used by `tests/aac_containers.rs` (CLI decode of AAC
from MP4 / Matroska / QuickTime / ADTS). Generated with FFmpeg 8.1.2 used
as a black-box tool:

```sh
SRC='aevalsrc=0.4*sin(2*PI*(200+600*t)*t)+0.2*sin(2*PI*1234.5*t):s=44100:d=0.5'
ffmpeg -f lavfi -i "$SRC" -c:a aac -b:a 64k lc_mono.m4a
ffmpeg -f lavfi -i "$SRC" -c:a aac -b:a 64k lc_mono.mkv
ffmpeg -f lavfi -i "$SRC" -c:a aac -b:a 64k lc_mono.mov
ffmpeg -f lavfi -i "$SRC" -c:a aac -b:a 64k lc_mono.aac
```

The test regenerates the same chirp + tone in Rust and checks the
decode against it (lag search + SNR).

`he_implicit_core_rate.mkv` is a stream copy of an HE-AAC v1 ADTS file
(libfdk_aac `aac_he`, 32 kb/s, 440 Hz stereo sine at 44.1 kHz, 0.7 s) into
Matroska:

```sh
ffmpeg -i he-aac-v1-stereo-44100-32kbps.aac -c copy he_implicit_core_rate.mkv
```

FFmpeg writes a 2-byte (implicit-SBR) AudioSpecificConfig and
`SamplingFrequency = OutputSamplingFrequency = 22050`, so the container
only declares the AAC core rate. The decode must then come out at that
declared rate, not at 44.1 kHz labelled as 22.05 kHz.
