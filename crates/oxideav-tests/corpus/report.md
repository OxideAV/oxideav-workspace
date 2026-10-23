# oxideplay / oxideav vs the playback corpus

Player: `oxideplay`  
CLI: `oxideav`  
Corpus: `fixtures` (245 fixtures from `corpus/manifest.csv`, 7 missing on this host)  
Timeout: 30 s per process; 245 fixtures run in 443 s; host macos aarch64

## Headline

**204 / 238 generated fixtures fully play in `oxideplay` (85.7%).** 8 emit only a first frame, 5 play one track, 21 fail at open / decode, 0 hang until killed; 7 fixtures could not be generated on this host.  
`oxideav transcode` (decode to y4m / wav): **162 / 238 PASS (68.1%)**.

Method: `oxideplay --dry-run`, then `oxideplay --vo hash --ao hash` (the default stream mapping, headless); a run that is not PASS is re-run per track (`--vo hash --ao none`, `--vo none --ao hash`) to attribute the failure. `oxideav transcode` decodes to `.y4m` / `.wav`. Reference counts: `ffprobe -count_frames` (video) and summed `nb_samples` (audio).

- **PASS** — every advertised A/V track produced all of its frames / samples and the process reached end of stream.
- **FIRST_FRAME** — the video decoder produced 1–10 frames (typically the IDR only).
- **PARTIAL** — one track decoded fully, the other did not (or the default mapping failed while the isolated legs played).
- **FAIL** — open / probe / decode error with 0 frames and 0 samples.
- **TIMEOUT** — process killed; no usable output.
- **MISSING** — the fixture could not be generated (this ffmpeg build lacks the encoder).

## Overview

| Verdict | oxideplay | Rate | oxideav transcode | Rate |
|---------|------:|-----:|------:|-----:|
| PASS | 204 | 83.3% | 162 | 66.1% |
| FIRST_FRAME | 8 | 3.3% | 8 | 3.3% |
| PARTIAL | 5 | 2.0% | 47 | 19.2% |
| FAIL | 21 | 8.6% | 21 | 8.6% |
| TIMEOUT | 0 | 0.0% | 0 | 0.0% |
| MISSING | 7 | 2.9% | 7 | 2.9% |
| **Total** | **245** | | **245** | |

## By container

| Ext | Pass | First-frame | Partial | Fail | Timeout | Missing | Total | CLI pass |
|-----|-----:|------------:|--------:|-----:|--------:|--------:|------:|---------:|
| .264 | 1 | 0 | 0 | 0 | 0 | 0 | 1 | 1 |
| .ac3 | 0 | 0 | 0 | 2 | 0 | 0 | 2 | 0 |
| .avi | 9 | 0 | 0 | 5 | 0 | 0 | 14 | 0 |
| .eac3 | 0 | 0 | 0 | 1 | 0 | 0 | 1 | 0 |
| .flac | 2 | 0 | 0 | 0 | 0 | 0 | 2 | 2 |
| .m2ts | 1 | 0 | 2 | 0 | 0 | 0 | 3 | 0 |
| .m4a | 7 | 0 | 0 | 0 | 0 | 0 | 7 | 7 |
| .mkv | 76 | 0 | 1 | 0 | 0 | 0 | 77 | 76 |
| .mov | 0 | 0 | 1 | 0 | 0 | 0 | 1 | 0 |
| .mp3 | 3 | 0 | 0 | 0 | 0 | 0 | 3 | 3 |
| .mp4 | 65 | 0 | 0 | 0 | 0 | 0 | 65 | 65 |
| .mpg | 0 | 8 | 0 | 0 | 0 | 0 | 8 | 0 |
| .ogg | 5 | 0 | 0 | 0 | 0 | 1 | 6 | 5 |
| .ogv | 0 | 0 | 0 | 0 | 0 | 3 | 3 | 0 |
| .ts | 1 | 0 | 1 | 0 | 0 | 0 | 2 | 0 |
| .wav | 3 | 0 | 0 | 0 | 0 | 0 | 3 | 3 |
| .webm | 31 | 0 | 0 | 0 | 0 | 3 | 34 | 0 |
| .wmv | 0 | 0 | 0 | 4 | 0 | 0 | 4 | 0 |
| .yuv | 0 | 0 | 0 | 9 | 0 | 0 | 9 | 0 |

## By section

| Section | Pass | First-frame | Partial | Fail | Timeout | Missing | Total | Pass rate | CLI pass |
|---------|-----:|------------:|--------:|-----:|--------:|--------:|------:|----------:|---------:|
| H.264 / AVC | 27 | 0 | 0 | 0 | 0 | 0 | 27 | 100.0% | 27 |
| HEVC / H.265 | 14 | 0 | 0 | 0 | 0 | 0 | 14 | 100.0% | 14 |
| VP8 / VP9 | 12 | 0 | 0 | 0 | 0 | 0 | 12 | 100.0% | 0 |
| AV1 | 7 | 0 | 0 | 0 | 0 | 3 | 10 | 70.0% | 0 |
| MPEG-1 / MPEG-2 / MPEG-4 | 4 | 5 | 0 | 0 | 0 | 0 | 9 | 44.4% | 0 |
| H.263 FAMILY | 3 | 0 | 0 | 0 | 0 | 0 | 3 | 100.0% | 0 |
| LOSSLESS / INTRA CODECS | 15 | 0 | 0 | 0 | 0 | 0 | 15 | 100.0% | 15 |
| RAW / UNCOMPRESSED PIXEL FORMATS | 0 | 0 | 0 | 14 | 0 | 0 | 14 | 0.0% | 0 |
| AUDIO CODECS / CHANNEL LAYOUTS | 19 | 0 | 0 | 3 | 0 | 1 | 23 | 82.6% | 19 |
| CONTAINER VARIATIONS | 9 | 2 | 2 | 0 | 0 | 0 | 13 | 69.2% | 4 |
| COLOR SPACE / RANGE TESTS | 10 | 0 | 0 | 0 | 0 | 0 | 10 | 100.0% | 10 |
| ALPHA / RGB / GBR TESTS | 10 | 0 | 0 | 0 | 0 | 0 | 10 | 100.0% | 10 |
| AUDIO + VIDEO COMBINATIONS | 15 | 0 | 0 | 0 | 0 | 0 | 15 | 100.0% | 11 |
| STREAM / TRACK EDGE CASES | 9 | 0 | 0 | 0 | 0 | 0 | 9 | 100.0% | 9 |
| FRAME / RESOLUTION VARIATIONS | 10 | 0 | 0 | 0 | 0 | 0 | 10 | 100.0% | 10 |
| INTERLACING / FIELD TESTS | 2 | 1 | 0 | 0 | 0 | 0 | 3 | 66.7% | 2 |
| SPECIAL COLOR FORMAT CONVERSIONS | 13 | 0 | 0 | 0 | 0 | 0 | 13 | 100.0% | 13 |
| EXTREME COMPRESSION TESTS | 9 | 0 | 0 | 0 | 0 | 0 | 9 | 100.0% | 5 |
| USEFUL METADATA / PROBE TESTS | 11 | 0 | 0 | 0 | 0 | 0 | 11 | 100.0% | 11 |
| WMV / VC-1 / THEORA / AVCHD / SVC (WebRTC) | 5 | 0 | 3 | 4 | 0 | 3 | 15 | 33.3% | 2 |

## Track-level codec scoreboard

Video (fixtures advertising the codec → any frame → all frames):

| Codec | Files | Any frame | All frames |
|-------|------:|----------:|-----------:|
| av1 | 12 | 12 | 12 |
| ffv1 | 27 | 27 | 27 |
| h263 | 4 | 4 | 4 |
| h264 | 88 | 87 | 87 |
| h265 | 32 | 32 | 32 |
| mpeg2video | 1 | 1 | 1 |
| mpeg4video | 5 | 5 | 5 |
| rawvideo | 5 | 0 | 0 |
| vp8 | 3 | 3 | 3 |
| vp9 | 16 | 16 | 16 |
| wbmp | 8 | 8 | 0 |
| wmv2 | 5 | 0 | 0 |

Audio:

| Codec | Files | Any samples | All samples |
|-------|------:|------------:|------------:|
| aac | 111 | 111 | 111 |
| ac3 | 11 | 8 | 6 |
| eac3 | 2 | 1 | 1 |
| flac | 30 | 30 | 30 |
| mp2 | 7 | 1 | 0 |
| mp3 | 9 | 8 | 8 |
| opus | 37 | 37 | 37 |
| pcm_f32le | 1 | 1 | 1 |
| pcm_mulaw | 3 | 3 | 3 |
| pcm_s16le | 1 | 1 | 1 |
| pcm_s24le | 1 | 1 | 1 |
| vorbis | 3 | 3 | 3 |
| wmav2 | 4 | 0 | 0 |

## Failures / partials / timeouts (oxideplay)

| Id | Ext | Title | Verdict | Video | Audio | Time | Notes |
|----|-----|-------|---------|-------|-------|-----:|-------|
| 056 | .webm | AV1 12-bit | MISSING | — | — | 0.0s | fixture not generated (see generate.log) |
| 057 | .webm | AV1 YUV422 | MISSING | — | — | 0.0s | fixture not generated (see generate.log) |
| 058 | .webm | AV1 YUV444 | MISSING | — | — | 0.0s | fixture not generated (see generate.log) |
| 064 | .mpg | MPEG-1 Video + MP2 | FIRST_FRAME | wbmp 1x7457 frames=1/50 | mp2 2ch samples=None/96768 | 0.5s | audio: invalid data: no playable audio or video stream (streams: Video:wbmp) |
| 065 | .mpg | MPEG-1 Video + MP3 | FIRST_FRAME | wbmp 1x7457 frames=1/50 | mp3 2ch samples=None/97920 | 0.5s | audio: invalid data: no playable audio or video stream (streams: Video:wbmp) |
| 066 | .mpg | MPEG-2 Video + MP2 | FIRST_FRAME | wbmp 1x7457 frames=1/50 | mp2 2ch samples=None/96768 | 0.5s | audio: invalid data: no playable audio or video stream (streams: Video:wbmp) |
| 067 | .mpg | MPEG-2 Video + AC3 | FIRST_FRAME | wbmp 1x7492 frames=1/50 | ac3 2ch samples=None/96768 | 0.5s | audio: invalid data: no playable audio or video stream (streams: Video:wbmp) |
| 068 | .mpg | MPEG-2 YUV422 | FIRST_FRAME | wbmp 1x7457 frames=1/50 | mp2 2ch samples=None/96768 | 0.6s | audio: invalid data: no playable audio or video stream (streams: Video:wbmp) |
| 091 | .yuv | Raw YUV420P | FAIL | — | — | 0.7s | invalid data: no playable audio or video stream (streams: Subtitle:ebu_stl) |
| 092 | .yuv | Raw YUV422P | FAIL | — | — | 0.9s | invalid data: no playable audio or video stream (streams: Subtitle:ebu_stl) |
| 093 | .yuv | Raw YUV444P | FAIL | — | — | 1.2s | invalid data: no playable audio or video stream (streams: Subtitle:ebu_stl) |
| 094 | .yuv | Raw YUV420P10 | FAIL | — | — | 0.2s | format not found: no registered demuxer recognises this input |
| 095 | .yuv | Raw YUV422P10 | FAIL | — | — | 0.2s | format not found: no registered demuxer recognises this input |
| 096 | .yuv | Raw YUV444P10 | FAIL | — | — | 0.2s | format not found: no registered demuxer recognises this input |
| 097 | .yuv | Raw YUV420P12 | FAIL | — | — | 0.2s | format not found: no registered demuxer recognises this input |
| 098 | .yuv | Raw YUV422P12 | FAIL | — | — | 0.2s | format not found: no registered demuxer recognises this input |
| 099 | .yuv | Raw YUV444P12 | FAIL | — | — | 0.2s | format not found: no registered demuxer recognises this input |
| 100 | .avi | Raw RGB24 | FAIL | rawvideo 320x180 frames=None/50 | — | 0.2s | codec not found: rgb24 |
| 101 | .avi | Raw BGR24 | FAIL | rawvideo 320x180 frames=None/50 | — | 0.2s | codec not found: rgb24 |
| 102 | .avi | Raw RGBA | FAIL | rawvideo 320x180 frames=None/50 | — | 0.2s | codec not found: avi:RGBA |
| 103 | .avi | Raw BGRA | FAIL | rawvideo 320x180 frames=None/50 | — | 0.2s | codec not found: rgb24 |
| 104 | .avi | Raw GBRP | FAIL | rawvideo 320x180 frames=None/50 | — | 0.2s | codec not found: rgb24 |
| 118 | .ogg | Vorbis mono | MISSING | — | — | 0.0s | fixture not generated (see generate.log) |
| 120 | .ac3 | AC-3 stereo | FAIL | — | ac3 2ch samples=None/96768 | 0.2s | invalid data: no playable audio or video stream (streams: Subtitle:ebu_stl) |
| 121 | .ac3 | AC-3 5.1 | FAIL | — | ac3 6ch samples=None/96768 | 0.2s | invalid data: no playable audio or video stream (streams: Subtitle:ebu_stl) |
| 122 | .eac3 | E-AC-3 5.1 | FAIL | — | eac3 6ch samples=None/96768 | 0.2s | format not found: no registered demuxer recognises this input |
| 129 | .mov | H.264 MOV | PARTIAL | h264 320x180 frames=0/50 | aac 2ch samples=97280/96000 | 4.1s | [data+data track]  |
| 132 | .mpg | H.264 MPEG-PS | FIRST_FRAME | wbmp 1x7457 frames=1/50 | mp2 2ch samples=None/96768 | 0.4s | audio: invalid data: no playable audio or video stream (streams: Video:wbmp) |
| 133 | .ts | MPEG-2 TS | PARTIAL | mpeg2video 0x0 frames=50/50 | mp2 0ch samples=48384/96768 | 1.0s |  |
| 139 | .mpg | MPEG-2 MPG | FIRST_FRAME | wbmp 1x7457 frames=1/50 | mp2 2ch samples=None/96768 | 0.5s | audio: invalid data: no playable audio or video stream (streams: Video:wbmp) |
| 197 | .mpg | MPEG-2 interlaced | FIRST_FRAME | wbmp 1x7457 frames=1/50 | mp2 2ch samples=None/96768 | 0.5s | audio: invalid data: no playable audio or video stream (streams: Video:wbmp) |
| 231 | .wmv | WMV2 (Windows Media Video 8) in ASF | FAIL | wmv2 320x180 frames=None/50 | wmav2 2ch samples=None/94208 | 0.5s | format not found: no registered demuxer recognises this input |
| 232 | .wmv | WMV3 / VC-1 Simple Main in WMV container (wmv2; vc1 encoder not in upstream ffmpeg) | FAIL | wmv2 320x180 frames=None/50 | wmav2 2ch samples=None/94208 | 0.5s | format not found: no registered demuxer recognises this input |
| 233 | .mkv | VC-1 Advanced Profile in MKV (wmv2 stand-in) | PARTIAL | wmv2 320x180 frames=0/50 | ac3 2ch samples=96768/96000 | 0.6s | no decoder for Video codec `mkv:BI/WMV2` — skipping Video \| video: codec not found: mkv:BI/WMV2 |
| 234 | .wmv | VC-1 Main Profile in ASF/WMV (wmv2 stand-in) | FAIL | wmv2 320x180 frames=None/50 | wmav2 2ch samples=None/94208 | 0.6s | format not found: no registered demuxer recognises this input |
| 235 | .wmv | VC-1 Advanced + WMA stereo (wmv2 stand-in; wmav2 max 2 channels) | FAIL | wmv2 320x180 frames=None/50 | wmav2 2ch samples=None/94208 | 0.6s | format not found: no registered demuxer recognises this input |
| 236 | .ogv | Theora + Vorbis (OGV) | MISSING | — | — | 0.0s | fixture not generated (see generate.log) |
| 237 | .ogv | Theora low bitrate | MISSING | — | — | 0.0s | fixture not generated (see generate.log) |
| 238 | .ogv | Theora YUV422 | MISSING | — | — | 0.0s | fixture not generated (see generate.log) |
| 239 | .m2ts | AVCHD H.264 High + AC-3 in M2TS (1080p24) | PARTIAL | h264 0x0 frames=48/48 | ac3 0ch samples=32256/96768 | 3.4s |  |
| 241 | .m2ts | AVCHD 1080i interlaced + AC-3 | PARTIAL | h264 0x0 frames=60/60 | ac3 0ch samples=32256/96768 | 4.9s |  |

## CLI failures (`oxideav transcode` to y4m / wav)

| Id | Ext | Title | CLI verdict | Frames | Samples | Error |
|----|-----|-------|-------------|-------:|--------:|-------|
| 042 | .webm | VP8 + Vorbis | PARTIAL | —/50 | 96256/96000 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 043 | .webm | VP8 + Opus | PARTIAL | —/50 | 96648/96000 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 044 | .webm | VP8 grayscale | PARTIAL | —/50 | 96648/96000 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 045 | .webm | VP9 8-bit | PARTIAL | —/50 | 96648/96000 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 046 | .webm | VP9 10-bit | PARTIAL | —/50 | 96648/96000 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 047 | .webm | VP9 12-bit | PARTIAL | —/50 | 96648/96000 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 048 | .webm | VP9 YUV422 | PARTIAL | —/50 | 96648/96000 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 049 | .webm | VP9 YUV444 | PARTIAL | —/50 | 96648/96000 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 050 | .webm | VP9 10-bit YUV444 | PARTIAL | —/50 | 96648/96000 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 051 | .webm | VP9 grayscale | PARTIAL | —/50 | 96648/96000 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 052 | .webm | VP9 BT.709 | PARTIAL | —/50 | 96648/96000 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 053 | .webm | VP9 BT.2020 | PARTIAL | —/50 | 96648/96000 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 054 | .webm | AV1 8-bit libaom | PARTIAL | —/50 | 96648/96000 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 055 | .webm | AV1 10-bit | PARTIAL | —/50 | 96648/96000 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 059 | .webm | AV1 grayscale | PARTIAL | —/50 | 96648/96000 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 060 | .webm | AV1 BT.709 | PARTIAL | —/50 | 96648/96000 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 061 | .webm | AV1 BT.2020 10-bit | PARTIAL | —/50 | 96648/96000 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 062 | .webm | AV1 PQ | PARTIAL | —/50 | 96648/96000 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 063 | .webm | AV1 HLG | PARTIAL | —/50 | 96648/96000 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 064 | .mpg | MPEG-1 Video + MP2 | FIRST_FRAME | 1/50 | —/96768 | wav: exit 3: oxideav: invalid data: WAV stream must be audio |
| 065 | .mpg | MPEG-1 Video + MP3 | FIRST_FRAME | 1/50 | —/97920 | wav: exit 3: oxideav: invalid data: WAV stream must be audio |
| 066 | .mpg | MPEG-2 Video + MP2 | FIRST_FRAME | 1/50 | —/96768 | wav: exit 3: oxideav: invalid data: WAV stream must be audio |
| 067 | .mpg | MPEG-2 Video + AC3 | FIRST_FRAME | 1/50 | —/96768 | wav: exit 3: oxideav: invalid data: WAV stream must be audio |
| 068 | .mpg | MPEG-2 YUV422 | FIRST_FRAME | 1/50 | —/96768 | wav: exit 3: oxideav: invalid data: WAV stream must be audio |
| 069 | .avi | MPEG-4 Part 2 | PARTIAL | —/50 | 97280/97280 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 070 | .avi | MPEG-4 Part 2 grayscale | PARTIAL | —/50 | 97920/97920 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 071 | .avi | MPEG-4 Part 2 YUV422 | PARTIAL | —/50 | 97920/97920 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 072 | .avi | MPEG-4 Part 2 QPEL | PARTIAL | —/50 | 97280/97280 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 073 | .avi | H.263 | PARTIAL | —/50 | 16000/16000 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 074 | .avi | H.263+ | PARTIAL | —/50 | 97920/97920 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 075 | .avi | H.263 low bitrate | PARTIAL | —/50 | 16000/16000 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 091 | .yuv | Raw YUV420P | FAIL | —/— | —/— | y4m: exit 3: oxideav: invalid data: y4m stream must be video \| wav: exit 3: oxideav: invalid data: WAV stream must be audio |
| 092 | .yuv | Raw YUV422P | FAIL | —/— | —/— | y4m: exit 3: oxideav: invalid data: y4m stream must be video \| wav: exit 3: oxideav: invalid data: WAV stream must be audio |
| 093 | .yuv | Raw YUV444P | FAIL | —/— | —/— | y4m: exit 3: oxideav: invalid data: y4m stream must be video \| wav: exit 3: oxideav: invalid data: WAV stream must be audio |
| 094 | .yuv | Raw YUV420P10 | FAIL | —/— | —/— | y4m: exit 5: oxideav: format not found: no registered demuxer recognises this input \| wav: exit 5: oxideav: format not found: no registered demuxer recognises this input |
| 095 | .yuv | Raw YUV422P10 | FAIL | —/— | —/— | y4m: exit 5: oxideav: format not found: no registered demuxer recognises this input \| wav: exit 5: oxideav: format not found: no registered demuxer recognises this input |
| 096 | .yuv | Raw YUV444P10 | FAIL | —/— | —/— | y4m: exit 5: oxideav: format not found: no registered demuxer recognises this input \| wav: exit 5: oxideav: format not found: no registered demuxer recognises this input |
| 097 | .yuv | Raw YUV420P12 | FAIL | —/— | —/— | y4m: exit 5: oxideav: format not found: no registered demuxer recognises this input \| wav: exit 5: oxideav: format not found: no registered demuxer recognises this input |
| 098 | .yuv | Raw YUV422P12 | FAIL | —/— | —/— | y4m: exit 5: oxideav: format not found: no registered demuxer recognises this input \| wav: exit 5: oxideav: format not found: no registered demuxer recognises this input |
| 099 | .yuv | Raw YUV444P12 | FAIL | —/— | —/— | y4m: exit 5: oxideav: format not found: no registered demuxer recognises this input \| wav: exit 5: oxideav: format not found: no registered demuxer recognises this input |
| 100 | .avi | Raw RGB24 | FAIL | —/50 | —/— | y4m: exit 6: oxideav: codec not found: rgb24 |
| 101 | .avi | Raw BGR24 | FAIL | —/50 | —/— | y4m: exit 6: oxideav: codec not found: rgb24 |
| 102 | .avi | Raw RGBA | FAIL | —/50 | —/— | y4m: exit 6: oxideav: codec not found: avi:RGBA |
| 103 | .avi | Raw BGRA | FAIL | —/50 | —/— | y4m: exit 6: oxideav: codec not found: rgb24 |
| 104 | .avi | Raw GBRP | FAIL | —/50 | —/— | y4m: exit 6: oxideav: codec not found: rgb24 |
| 120 | .ac3 | AC-3 stereo | FAIL | —/— | —/96768 | wav: exit 3: oxideav: invalid data: WAV stream must be audio |
| 121 | .ac3 | AC-3 5.1 | FAIL | —/— | —/96768 | wav: exit 3: oxideav: invalid data: WAV stream must be audio |
| 122 | .eac3 | E-AC-3 5.1 | FAIL | —/— | —/96768 | wav: exit 5: oxideav: format not found: no registered demuxer recognises this input |
| 129 | .mov | H.264 MOV | PARTIAL | —/50 | 97280/96000 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 131 | .ts | H.264 MPEG-TS | PARTIAL | —/50 | 97280/97280 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 132 | .mpg | H.264 MPEG-PS | FIRST_FRAME | 1/50 | —/96768 | wav: exit 3: oxideav: invalid data: WAV stream must be audio |
| 133 | .ts | MPEG-2 TS | PARTIAL | —/50 | 48384/96768 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 136 | .webm | VP9 WebM | PARTIAL | —/50 | 96648/96000 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 137 | .webm | AV1 WebM | PARTIAL | —/50 | 96648/96000 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 138 | .avi | MPEG-4 AVI | PARTIAL | —/50 | 97920/97920 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 139 | .mpg | MPEG-2 MPG | FIRST_FRAME | 1/50 | —/96768 | wav: exit 3: oxideav: invalid data: WAV stream must be audio |
| 140 | .avi | H.263 AVI | PARTIAL | —/50 | 16000/16000 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 172 | .webm | VP9 + Opus mono | PARTIAL | —/50 | 96648/96000 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 173 | .webm | VP9 + Opus 5.1 | PARTIAL | —/50 | 96648/96000 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 174 | .webm | AV1 + Opus mono | PARTIAL | —/50 | 96648/96000 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 175 | .webm | AV1 + Opus 5.1 | PARTIAL | —/50 | 96648/96000 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 197 | .mpg | MPEG-2 interlaced | FIRST_FRAME | 1/50 | —/96768 | wav: exit 3: oxideav: invalid data: WAV stream must be audio |
| 214 | .webm | VP9 20 kbps | PARTIAL | —/50 | 96648/96000 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 215 | .webm | AV1 20 kbps | PARTIAL | —/50 | 96648/96000 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 218 | .webm | VP9 100 kbps | PARTIAL | —/50 | 96648/96000 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 219 | .webm | AV1 100 kbps | PARTIAL | —/50 | 96648/96000 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 231 | .wmv | WMV2 (Windows Media Video 8) in ASF | FAIL | —/50 | —/94208 | y4m: exit 5: oxideav: format not found: no registered demuxer recognises this input \| wav: exit 5: oxideav: format not found: no registered demuxer recognises this input |
| 232 | .wmv | WMV3 / VC-1 Simple Main in WMV container (wmv2; vc1 encoder not in upstream ffmpeg) | FAIL | —/50 | —/94208 | y4m: exit 5: oxideav: format not found: no registered demuxer recognises this input \| wav: exit 5: oxideav: format not found: no registered demuxer recognises this input |
| 233 | .mkv | VC-1 Advanced Profile in MKV (wmv2 stand-in) | PARTIAL | —/50 | 96768/96000 | y4m: exit 6: oxideav: codec not found: mkv:BI/WMV2 |
| 234 | .wmv | VC-1 Main Profile in ASF/WMV (wmv2 stand-in) | FAIL | —/50 | —/94208 | y4m: exit 5: oxideav: format not found: no registered demuxer recognises this input \| wav: exit 5: oxideav: format not found: no registered demuxer recognises this input |
| 235 | .wmv | VC-1 Advanced + WMA stereo (wmv2 stand-in; wmav2 max 2 channels) | FAIL | —/50 | —/94208 | y4m: exit 5: oxideav: format not found: no registered demuxer recognises this input \| wav: exit 5: oxideav: format not found: no registered demuxer recognises this input |
| 239 | .m2ts | AVCHD H.264 High + AC-3 in M2TS (1080p24) | PARTIAL | —/48 | 32256/96768 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 240 | .m2ts | AVCHD 1440x1080 anamorphic + AC-3 5.1 | PARTIAL | —/60 | 96768/96768 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 241 | .m2ts | AVCHD 1080i interlaced + AC-3 | PARTIAL | —/60 | 32256/96768 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 242 | .webm | VP9 temporal SVC (auto-alt-ref + lag; temporal-layers removed in current libvpx-vp9) | PARTIAL | —/50 | 96648/96000 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |
| 243 | .webm | VP9 temporal SVC + spatial simulcast-style (2 resolutions) | PARTIAL | —/50 | 96648/96000 | y4m: exit 3: oxideav: invalid data: rawvideo encoder: pixel_format is required |

## All fixtures

| Id | Ext | Title | Verdict | Video frames | Audio samples | CLI | Time |
|----|-----|-------|---------|-------------:|--------------:|-----|-----:|
| 001 | .mp4 | H.264 Baseline 8-bit YUV420 | PASS | 50 | 97280 | PASS | 1.4s |
| 002 | .mp4 | H.264 Main 8-bit YUV420 | PASS | 50 | 97280 | PASS | 2.5s |
| 003 | .mp4 | H.264 High 8-bit YUV420 | PASS | 50 | 97280 | PASS | 2.5s |
| 004 | .mp4 | H.264 High 8-bit YUV422 | PASS | 50 | 97280 | PASS | 2.6s |
| 005 | .mp4 | H.264 High 8-bit YUV444 | PASS | 50 | 97280 | PASS | 2.7s |
| 006 | .mkv | H.264 High10 10-bit YUV420 | PASS | 50 | 97280 | PASS | 2.7s |
| 007 | .mkv | H.264 High422 10-bit | PASS | 50 | 97280 | PASS | 2.8s |
| 008 | .mkv | H.264 High444 10-bit | PASS | 50 | 97280 | PASS | 2.9s |
| 009 | .mkv | H.264 High444 RGB-ish GBR | PASS | 50 | 97280 | PASS | 2.8s |
| 010 | .mkv | H.264 grayscale | PASS | 50 | 97280 | PASS | 1.5s |
| 011 | .mp4 | H.264 CRF 40 | PASS | 50 | 97280 | PASS | 2.6s |
| 012 | .mp4 | H.264 CRF 45 | PASS | 50 | 97280 | PASS | 2.6s |
| 013 | .mp4 | H.264 ultrafast low bitrate | PASS | 50 | 97280 | PASS | 2.6s |
| 014 | .mp4 | H.264 no B-frames | PASS | 50 | 97280 | PASS | 2.7s |
| 015 | .mp4 | H.264 many B-frames | PASS | 50 | 97280 | PASS | 2.6s |
| 016 | .mp4 | H.264 CABAC | PASS | 50 | 97280 | PASS | 2.6s |
| 017 | .mp4 | H.264 CAVLC | PASS | 50 | 97280 | PASS | 2.8s |
| 018 | .mp4 | H.264 BT.709 | PASS | 50 | 97280 | PASS | 2.8s |
| 019 | .mp4 | H.264 BT.601 | PASS | 50 | 97280 | PASS | 2.9s |
| 020 | .mkv | H.264 BT.2020 | PASS | 50 | 97280 | PASS | 2.9s |
| 021 | .mp4 | H.264 limited range | PASS | 50 | 97280 | PASS | 2.9s |
| 022 | .mp4 | H.264 full range | PASS | 50 | 97280 | PASS | 2.8s |
| 023 | .mp4 | H.264 23.976 fps | PASS | 48 | 97280 | PASS | 2.8s |
| 024 | .mp4 | H.264 24 fps | PASS | 48 | 97280 | PASS | 3.0s |
| 025 | .mp4 | H.264 25 fps | PASS | 50 | 97280 | PASS | 3.1s |
| 026 | .mp4 | H.264 30 fps | PASS | 60 | 97280 | PASS | 3.2s |
| 027 | .mp4 | H.264 60 fps | PASS | 120 | 97280 | PASS | 3.6s |
| 028 | .mp4 | HEVC Main 8-bit | PASS | 50 | 97280 | PASS | 3.5s |
| 029 | .mkv | HEVC Main10 | PASS | 50 | 97280 | PASS | 3.1s |
| 030 | .mkv | HEVC 10-bit YUV422 | PASS | 50 | 97280 | PASS | 3.1s |
| 031 | .mkv | HEVC 10-bit YUV444 | PASS | 50 | 97280 | PASS | 3.2s |
| 032 | .mkv | HEVC 12-bit YUV420 | PASS | 50 | 97280 | PASS | 3.0s |
| 033 | .mkv | HEVC 12-bit YUV422 | PASS | 50 | 97280 | PASS | 3.1s |
| 034 | .mkv | HEVC 12-bit YUV444 | PASS | 50 | 97280 | PASS | 3.2s |
| 035 | .mkv | HEVC grayscale | PASS | 50 | 97280 | PASS | 3.0s |
| 036 | .mp4 | HEVC BT.709 | PASS | 50 | 97280 | PASS | 2.9s |
| 037 | .mkv | HEVC BT.2020 10-bit | PASS | 50 | 97280 | PASS | 3.2s |
| 038 | .mkv | HEVC HDR PQ metadata | PASS | 50 | 97280 | PASS | 3.3s |
| 039 | .mkv | HEVC HLG metadata | PASS | 50 | 97280 | PASS | 3.4s |
| 040 | .mp4 | HEVC CRF 40 | PASS | 50 | 97280 | PASS | 3.2s |
| 041 | .mp4 | HEVC CRF 45 | PASS | 50 | 97280 | PASS | 3.0s |
| 042 | .webm | VP8 + Vorbis | PASS | 50 | 96256 | PARTIAL | 0.4s |
| 043 | .webm | VP8 + Opus | PASS | 50 | 96648 | PARTIAL | 1.5s |
| 044 | .webm | VP8 grayscale | PASS | 50 | 96648 | PARTIAL | 1.5s |
| 045 | .webm | VP9 8-bit | PASS | 50 | 96648 | PARTIAL | 1.5s |
| 046 | .webm | VP9 10-bit | PASS | 50 | 96648 | PARTIAL | 1.5s |
| 047 | .webm | VP9 12-bit | PASS | 50 | 96648 | PARTIAL | 1.6s |
| 048 | .webm | VP9 YUV422 | PASS | 50 | 96648 | PARTIAL | 1.6s |
| 049 | .webm | VP9 YUV444 | PASS | 50 | 96648 | PARTIAL | 1.6s |
| 050 | .webm | VP9 10-bit YUV444 | PASS | 50 | 96648 | PARTIAL | 1.6s |
| 051 | .webm | VP9 grayscale | PASS | 50 | 96648 | PARTIAL | 1.5s |
| 052 | .webm | VP9 BT.709 | PASS | 50 | 96648 | PARTIAL | 1.6s |
| 053 | .webm | VP9 BT.2020 | PASS | 50 | 96648 | PARTIAL | 1.5s |
| 054 | .webm | AV1 8-bit libaom | PASS | 50 | 96648 | PARTIAL | 1.5s |
| 055 | .webm | AV1 10-bit | PASS | 50 | 96648 | PARTIAL | 1.5s |
| 056 | .webm | AV1 12-bit | MISSING |  |  | MISSING | 0.0s |
| 057 | .webm | AV1 YUV422 | MISSING |  |  | MISSING | 0.0s |
| 058 | .webm | AV1 YUV444 | MISSING |  |  | MISSING | 0.0s |
| 059 | .webm | AV1 grayscale | PASS | 50 | 96648 | PARTIAL | 1.6s |
| 060 | .webm | AV1 BT.709 | PASS | 50 | 96648 | PARTIAL | 1.7s |
| 061 | .webm | AV1 BT.2020 10-bit | PASS | 50 | 96648 | PARTIAL | 1.6s |
| 062 | .webm | AV1 PQ | PASS | 50 | 96648 | PARTIAL | 1.6s |
| 063 | .webm | AV1 HLG | PASS | 50 | 96648 | PARTIAL | 1.6s |
| 064 | .mpg | MPEG-1 Video + MP2 | FIRST_FRAME | 1 |  | FIRST_FRAME | 0.5s |
| 065 | .mpg | MPEG-1 Video + MP3 | FIRST_FRAME | 1 |  | FIRST_FRAME | 0.5s |
| 066 | .mpg | MPEG-2 Video + MP2 | FIRST_FRAME | 1 |  | FIRST_FRAME | 0.5s |
| 067 | .mpg | MPEG-2 Video + AC3 | FIRST_FRAME | 1 |  | FIRST_FRAME | 0.5s |
| 068 | .mpg | MPEG-2 YUV422 | FIRST_FRAME | 1 |  | FIRST_FRAME | 0.6s |
| 069 | .avi | MPEG-4 Part 2 | PASS | 50 | 97280 | PARTIAL | 3.2s |
| 070 | .avi | MPEG-4 Part 2 grayscale | PASS | 50 | 97920 | PARTIAL | 0.4s |
| 071 | .avi | MPEG-4 Part 2 YUV422 | PASS | 50 | 97920 | PARTIAL | 0.5s |
| 072 | .avi | MPEG-4 Part 2 QPEL | PASS | 50 | 97280 | PARTIAL | 2.8s |
| 073 | .avi | H.263 | PASS | 50 | 16000 | PARTIAL | 0.6s |
| 074 | .avi | H.263+ | PASS | 50 | 97920 | PARTIAL | 0.5s |
| 075 | .avi | H.263 low bitrate | PASS | 50 | 16000 | PARTIAL | 0.5s |
| 076 | .mkv | FFV1 YUV420 8-bit | PASS | 50 | 96000 | PASS | 0.6s |
| 077 | .mkv | FFV1 YUV422 8-bit | PASS | 50 | 96000 | PASS | 0.7s |
| 078 | .mkv | FFV1 YUV444 8-bit | PASS | 50 | 96000 | PASS | 0.8s |
| 079 | .mkv | FFV1 YUV420 10-bit | PASS | 50 | 96000 | PASS | 0.8s |
| 080 | .mkv | FFV1 YUV422 10-bit | PASS | 50 | 96000 | PASS | 0.9s |
| 081 | .mkv | FFV1 YUV444 10-bit | PASS | 50 | 96000 | PASS | 1.1s |
| 082 | .mkv | FFV1 YUV420 12-bit | PASS | 50 | 96000 | PASS | 0.8s |
| 083 | .mkv | FFV1 YUV422 12-bit | PASS | 50 | 96000 | PASS | 0.9s |
| 084 | .mkv | FFV1 YUV444 12-bit | PASS | 50 | 96000 | PASS | 1.1s |
| 085 | .mkv | FFV1 RGB24 | PASS | 50 | 96000 | PASS | 0.7s |
| 086 | .mkv | FFV1 BGR24 | PASS | 50 | 96000 | PASS | 0.8s |
| 087 | .mkv | FFV1 GBR planar | PASS | 50 | 96000 | PASS | 0.8s |
| 088 | .mkv | FFV1 GBR planar 10-bit | PASS | 50 | 96000 | PASS | 1.1s |
| 089 | .mkv | FFV1 GBR planar 12-bit | PASS | 50 | 96000 | PASS | 1.2s |
| 090 | .mkv | FFV1 RGB48 | PASS | 50 | 96000 | PASS | 1.1s |
| 091 | .yuv | Raw YUV420P | FAIL |  |  | FAIL | 0.7s |
| 092 | .yuv | Raw YUV422P | FAIL |  |  | FAIL | 0.9s |
| 093 | .yuv | Raw YUV444P | FAIL |  |  | FAIL | 1.2s |
| 094 | .yuv | Raw YUV420P10 | FAIL |  |  | FAIL | 0.2s |
| 095 | .yuv | Raw YUV422P10 | FAIL |  |  | FAIL | 0.2s |
| 096 | .yuv | Raw YUV444P10 | FAIL |  |  | FAIL | 0.2s |
| 097 | .yuv | Raw YUV420P12 | FAIL |  |  | FAIL | 0.2s |
| 098 | .yuv | Raw YUV422P12 | FAIL |  |  | FAIL | 0.2s |
| 099 | .yuv | Raw YUV444P12 | FAIL |  |  | FAIL | 0.2s |
| 100 | .avi | Raw RGB24 | FAIL |  |  | FAIL | 0.2s |
| 101 | .avi | Raw BGR24 | FAIL |  |  | FAIL | 0.2s |
| 102 | .avi | Raw RGBA | FAIL |  |  | FAIL | 0.2s |
| 103 | .avi | Raw BGRA | FAIL |  |  | FAIL | 0.2s |
| 104 | .avi | Raw GBRP | FAIL |  |  | FAIL | 0.2s |
| 105 | .m4a | AAC mono 16k | PASS | 0 | 97280 | PASS | 1.5s |
| 106 | .m4a | AAC stereo 24k | PASS | 0 | 97280 | PASS | 3.6s |
| 107 | .m4a | AAC 5.1 64k | PASS | 0 | 97280 | PASS | 8.8s |
| 108 | .m4a | AAC 7.1 96k | PASS | 0 | 97280 | PASS | 11.3s |
| 109 | .m4a | AAC low sample rate | PASS | 0 | 46080 | PASS | 0.8s |
| 110 | .m4a | AAC 32 kHz | PASS | 0 | 65536 | PASS | 1.9s |
| 111 | .mp3 | MP3 mono | PASS | 0 | 97920 | PASS | 0.3s |
| 112 | .mp3 | MP3 stereo | PASS | 0 | 97920 | PASS | 0.3s |
| 113 | .mp3 | MP3 high bitrate stereo (libmp3lame does not encode 5.1) | PASS | 0 | 97920 | PASS | 0.3s |
| 114 | .ogg | Opus mono | PASS | 0 | 96648 | PASS | 0.8s |
| 115 | .ogg | Opus stereo | PASS | 0 | 96648 | PASS | 1.6s |
| 116 | .ogg | Opus 5.1 | PASS | 0 | 96648 | PASS | 4.1s |
| 117 | .ogg | Opus 7.1 | PASS | 0 | 96648 | PASS | 5.1s |
| 118 | .ogg | Vorbis mono | MISSING |  |  | MISSING | 0.0s |
| 119 | .ogg | Vorbis stereo | PASS | 0 | 96256 | PASS | 0.3s |
| 120 | .ac3 | AC-3 stereo | FAIL |  |  | FAIL | 0.2s |
| 121 | .ac3 | AC-3 5.1 | FAIL |  |  | FAIL | 0.2s |
| 122 | .eac3 | E-AC-3 5.1 | FAIL |  |  | FAIL | 0.2s |
| 123 | .flac | FLAC stereo | PASS | 0 | 96000 | PASS | 0.4s |
| 124 | .flac | FLAC 5.1 | PASS | 0 | 96000 | PASS | 0.4s |
| 125 | .wav | PCM signed 16-bit mono | PASS | 0 | 32000 | PASS | 0.2s |
| 126 | .wav | PCM signed 24-bit stereo | PASS | 0 | 96000 | PASS | 0.3s |
| 127 | .wav | PCM float 32-bit | PASS | 0 | 96000 | PASS | 0.3s |
| 128 | .mp4 | H.264 MP4 | PASS | 50 | 97280 | PASS | 2.8s |
| 129 | .mov | H.264 MOV | PARTIAL | 0 | 97280 | PARTIAL | 4.1s |
| 130 | .mkv | H.264 MKV | PASS | 50 | 97280 | PASS | 2.9s |
| 131 | .ts | H.264 MPEG-TS | PASS | 50 | 97280 | PARTIAL | 2.7s |
| 132 | .mpg | H.264 MPEG-PS | FIRST_FRAME | 1 |  | FIRST_FRAME | 0.4s |
| 133 | .ts | MPEG-2 TS | PARTIAL | 50 | 48384 | PARTIAL | 1.0s |
| 134 | .mp4 | HEVC MP4 | PASS | 50 | 97280 | PASS | 2.9s |
| 135 | .mkv | HEVC MKV | PASS | 50 | 97280 | PASS | 2.9s |
| 136 | .webm | VP9 WebM | PASS | 50 | 96648 | PARTIAL | 1.5s |
| 137 | .webm | AV1 WebM | PASS | 50 | 96648 | PARTIAL | 1.5s |
| 138 | .avi | MPEG-4 AVI | PASS | 50 | 97920 | PARTIAL | 0.5s |
| 139 | .mpg | MPEG-2 MPG | FIRST_FRAME | 1 |  | FIRST_FRAME | 0.5s |
| 140 | .avi | H.263 AVI | PASS | 50 | 16000 | PARTIAL | 0.6s |
| 141 | .mp4 | YUV420 BT.709 | PASS | 50 | 97280 | PASS | 3.0s |
| 142 | .mp4 | YUV420 BT.601 | PASS | 50 | 97280 | PASS | 2.9s |
| 143 | .mkv | BT.2020 non-constant luminance | PASS | 50 | 97280 | PASS | 3.0s |
| 144 | .mkv | BT.2020 10-bit via zscale (bt2020c matrix unsupported in scale on current FFmpeg) | PASS | 50 | 97280 | PASS | 3.0s |
| 145 | .mp4 | SMPTE 170M | PASS | 50 | 97280 | PASS | 2.9s |
| 146 | .mp4 | BT.470 BG | PASS | 50 | 97280 | PASS | 3.0s |
| 147 | .mp4 | Full-range YUV | PASS | 50 | 97280 | PASS | 2.9s |
| 148 | .mp4 | Limited-range YUV | PASS | 50 | 97280 | PASS | 2.9s |
| 149 | .mkv | PQ transfer | PASS | 50 | 97280 | PASS | 3.0s |
| 150 | .mkv | HLG transfer | PASS | 50 | 97280 | PASS | 2.9s |
| 151 | .mkv | FFV1 RGBA | PASS | 50 | 96000 | PASS | 1.0s |
| 152 | .mkv | FFV1 BGRA | PASS | 50 | 96000 | PASS | 1.0s |
| 153 | .mkv | FFV1 RGB24 | PASS | 50 | 96000 | PASS | 0.8s |
| 154 | .mkv | FFV1 BGR24 | PASS | 50 | 96000 | PASS | 0.8s |
| 155 | .mkv | FFV1 GBRP | PASS | 50 | 96000 | PASS | 0.8s |
| 156 | .mkv | FFV1 GBRP10 | PASS | 50 | 96000 | PASS | 1.1s |
| 157 | .mkv | FFV1 GBRP12 | PASS | 50 | 96000 | PASS | 1.2s |
| 158 | .mkv | RGB JPEG-style full range | PASS | 50 | 96000 | PASS | 0.8s |
| 159 | .mkv | Grayscale H.264 | PASS | 50 | 97280 | PASS | 2.6s |
| 160 | .mkv | Grayscale HEVC | PASS | 50 | 97280 | PASS | 2.6s |
| 161 | .mp4 | H.264 + AAC mono | PASS | 50 | 97280 | PASS | 1.5s |
| 162 | .mp4 | H.264 + AAC stereo | PASS | 50 | 97280 | PASS | 2.6s |
| 163 | .mp4 | H.264 + AAC 5.1 | PASS | 50 | 97280 | PASS | 6.7s |
| 164 | .mkv | H.264 + MP3 | PASS | 50 | 97920 | PASS | 0.5s |
| 165 | .mkv | H.264 + Opus | PASS | 50 | 96648 | PASS | 1.6s |
| 166 | .mkv | H.264 + Vorbis | PASS | 50 | 96256 | PASS | 0.5s |
| 167 | .mkv | H.264 + AC3 5.1 | PASS | 50 | 96768 | PASS | 0.5s |
| 168 | .mkv | H.264 + EAC3 5.1 | PASS | 50 | 96768 | PASS | 0.5s |
| 169 | .mkv | H.264 + FLAC stereo | PASS | 50 | 96000 | PASS | 0.6s |
| 170 | .mkv | HEVC + Opus | PASS | 50 | 96648 | PASS | 1.6s |
| 171 | .mkv | HEVC + AC3 | PASS | 50 | 96768 | PASS | 0.5s |
| 172 | .webm | VP9 + Opus mono | PASS | 50 | 96648 | PARTIAL | 0.9s |
| 173 | .webm | VP9 + Opus 5.1 | PASS | 50 | 96648 | PARTIAL | 3.9s |
| 174 | .webm | AV1 + Opus mono | PASS | 50 | 96648 | PARTIAL | 0.9s |
| 175 | .webm | AV1 + Opus 5.1 | PASS | 50 | 96648 | PARTIAL | 3.8s |
| 176 | .mp4 | Video only | PASS | 50 |  | PASS | 0.3s |
| 177 | .m4a | Audio only | PASS | 0 | 97280 | PASS | 2.2s |
| 178 | .mp4 | Video with no audio | PASS | 50 |  | PASS | 0.4s |
| 179 | .mp4 | Multiple audio tracks | PASS | 50 | 97280 | PASS | 2.4s |
| 180 | .mkv | Multiple video streams | PASS | 50 |  | PASS | 0.4s |
| 181 | .mkv | Explicit mono channel layout | PASS | 50 | 97280 | PASS | 1.5s |
| 182 | .mkv | Explicit stereo channel layout | PASS | 50 | 97280 | PASS | 2.5s |
| 183 | .mkv | Explicit 5.1 layout | PASS | 50 | 96768 | PASS | 0.5s |
| 184 | .mkv | AC-3 5.1 (7.1 downmix; ac3 encoder max is 5.1) | PASS | 50 | 96768 | PASS | 0.5s |
| 185 | .mp4 | 192x108 | PASS | 50 | 97280 | PASS | 2.3s |
| 186 | .mp4 | 320x180 | PASS | 50 | 97280 | PASS | 2.4s |
| 187 | .mp4 | 640x360 | PASS | 50 | 97280 | PASS | 2.9s |
| 188 | .mp4 | 640x480 | PASS | 50 | 97280 | PASS | 3.1s |
| 189 | .mp4 | 720x480 | PASS | 50 | 97280 | PASS | 3.2s |
| 190 | .mp4 | 1280x720 | PASS | 50 | 97280 | PASS | 4.6s |
| 191 | .mp4 | 1920x1080 low bitrate | PASS | 50 | 97280 | PASS | 7.2s |
| 192 | .mkv | Odd dimensions | PASS | 50 | 97280 | PASS | 2.6s |
| 193 | .mp4 | Square | PASS | 50 | 97280 | PASS | 2.5s |
| 194 | .mp4 | Portrait | PASS | 50 | 97280 | PASS | 2.5s |
| 195 | .mp4 | Interlaced H.264 | PASS | 50 | 97280 | PASS | 2.4s |
| 196 | .mp4 | Interlaced bottom field | PASS | 50 | 97280 | PASS | 2.4s |
| 197 | .mpg | MPEG-2 interlaced | FIRST_FRAME | 1 |  | FIRST_FRAME | 0.5s |
| 198 | .mp4 | YUV420P | PASS | 50 | 97280 | PASS | 2.5s |
| 199 | .mkv | YUV422P | PASS | 50 | 97280 | PASS | 2.5s |
| 200 | .mkv | YUV444P | PASS | 50 | 97280 | PASS | 2.5s |
| 201 | .mkv | YUV420P10 | PASS | 50 | 97280 | PASS | 2.6s |
| 202 | .mkv | YUV422P10 | PASS | 50 | 97280 | PASS | 2.8s |
| 203 | .mkv | YUV444P10 | PASS | 50 | 97280 | PASS | 2.8s |
| 204 | .mkv | YUV420P12 | PASS | 50 | 97280 | PASS | 2.6s |
| 205 | .mkv | YUV422P12 | PASS | 50 | 97280 | PASS | 2.7s |
| 206 | .mkv | YUV444P12 | PASS | 50 | 97280 | PASS | 2.8s |
| 207 | .mkv | RGB24 | PASS | 50 | 96000 | PASS | 0.7s |
| 208 | .mkv | BGR24 | PASS | 50 | 96000 | PASS | 0.7s |
| 209 | .mkv | RGBA | PASS | 50 | 96000 | PASS | 1.0s |
| 210 | .mkv | GBR planar | PASS | 50 | 96000 | PASS | 0.7s |
| 211 | .mp4 | H.264 20 kbps | PASS | 50 | 97280 | PASS | 1.4s |
| 212 | .mp4 | H.264 50 kbps | PASS | 50 | 97280 | PASS | 1.4s |
| 213 | .mkv | HEVC 20 kbps | PASS | 50 | 97280 | PASS | 1.5s |
| 214 | .webm | VP9 20 kbps | PASS | 50 | 96648 | PARTIAL | 0.8s |
| 215 | .webm | AV1 20 kbps | PASS | 50 | 96648 | PARTIAL | 0.8s |
| 216 | .mp4 | H.264 100 kbps | PASS | 50 | 97280 | PASS | 2.5s |
| 217 | .mkv | HEVC 100 kbps | PASS | 50 | 97280 | PASS | 2.7s |
| 218 | .webm | VP9 100 kbps | PASS | 50 | 96648 | PARTIAL | 1.5s |
| 219 | .webm | AV1 100 kbps | PASS | 50 | 96648 | PARTIAL | 1.5s |
| 220 | .mp4 | Add rotation metadata | PASS | 50 | 97280 | PASS | 2.7s |
| 221 | .mp4 | Set SAR 1:1 | PASS | 50 | 97280 | PASS | 2.5s |
| 222 | .mp4 | Set SAR 4:3 | PASS | 50 | 97280 | PASS | 2.5s |
| 223 | .mp4 | Set SAR 16:9 | PASS | 50 | 97280 | PASS | 2.6s |
| 224 | .mp4 | Set DAR 4:3 | PASS | 50 | 97280 | PASS | 2.6s |
| 225 | .mp4 | Set DAR 16:9 | PASS | 50 | 97280 | PASS | 2.7s |
| 226 | .mp4 | Zero audio | PASS | 50 | 97280 | PASS | 2.7s |
| 227 | .mp4 | Very low sample rate audio | PASS | 50 | 17408 | PASS | 0.6s |
| 228 | .mp4 | 16 kHz audio | PASS | 50 | 33792 | PASS | 0.8s |
| 229 | .mp4 | 22.05 kHz audio | PASS | 50 | 46080 | PASS | 0.9s |
| 230 | .mp4 | 32 kHz audio | PASS | 50 | 65536 | PASS | 2.0s |
| 231 | .wmv | WMV2 (Windows Media Video 8) in ASF | FAIL |  |  | FAIL | 0.5s |
| 232 | .wmv | WMV3 / VC-1 Simple Main in WMV container (wmv2; vc1 encoder not in upstream ffmpeg) | FAIL |  |  | FAIL | 0.5s |
| 233 | .mkv | VC-1 Advanced Profile in MKV (wmv2 stand-in) | PARTIAL | 0 | 96768 | PARTIAL | 0.6s |
| 234 | .wmv | VC-1 Main Profile in ASF/WMV (wmv2 stand-in) | FAIL |  |  | FAIL | 0.6s |
| 235 | .wmv | VC-1 Advanced + WMA stereo (wmv2 stand-in; wmav2 max 2 channels) | FAIL |  |  | FAIL | 0.6s |
| 236 | .ogv | Theora + Vorbis (OGV) | MISSING |  |  | MISSING | 0.0s |
| 237 | .ogv | Theora low bitrate | MISSING |  |  | MISSING | 0.0s |
| 238 | .ogv | Theora YUV422 | MISSING |  |  | MISSING | 0.0s |
| 239 | .m2ts | AVCHD H.264 High + AC-3 in M2TS (1080p24) | PARTIAL | 48 | 32256 | PARTIAL | 3.4s |
| 240 | .m2ts | AVCHD 1440x1080 anamorphic + AC-3 5.1 | PASS | 60 | 96768 | PARTIAL | 1.8s |
| 241 | .m2ts | AVCHD 1080i interlaced + AC-3 | PARTIAL | 60 | 32256 | PARTIAL | 4.9s |
| 242 | .webm | VP9 temporal SVC (auto-alt-ref + lag; temporal-layers removed in current libvpx-vp9) | PASS | 50 | 96648 | PARTIAL | 1.5s |
| 243 | .webm | VP9 temporal SVC + spatial simulcast-style (2 resolutions) | PASS | 50 | 96648 | PARTIAL | 1.6s |
| 244 | .264 | H.264 WebRTC baseline (Annex B, short GOP, repeat SPS/PPS) | PASS | 50 |  | PASS | 0.4s |
| 245 | .mkv | H.264 + Opus in MKV (WebRTC-friendly A/V; WebM does not support H.264) | PASS | 50 | 96648 | PASS | 1.6s |
