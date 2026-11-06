#!/bin/bash
# Rebuilds the spliced fixtures of tests/stream_params_events.rs with
# the black-box ffmpeg binary. Each fixture is independently encoded
# segments of different output shape, joined and stream-copied (never
# re-encoded) into one file.
#
#   bash generate.sh [out-dir]   (default: this directory)
set -euo pipefail
out=$(cd "${1:-$(dirname "$0")}" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cd "$work"
F() { ffmpeg -y -hide_banner -loglevel error "$@"; }

# H.264 Annex B, 6 + 6 pictures, B-frames on: 64x48 then 96x64.
for s in 64x48 96x64; do
  F -f lavfi -i testsrc2=size=$s:rate=10 -frames:v 6 -c:v libx264 -preset veryfast \
    -profile:v main -bf 2 -pix_fmt yuv420p -x264-params keyint=6:scenecut=0 -f h264 $s.h264
done
cat 64x48.h264 96x64.h264 > "$out/h264_resize_64x48_96x64.h264"

# MP3 stereo, 11 frames at 44.1 kHz then 8 frames at 32 kHz, in Matroska.
for r in 44100 32000; do
  F -f lavfi -i "sine=frequency=440:sample_rate=$r" -t 0.25 -ac 2 -c:a libmp3lame -b:a 64k \
    -write_xing 0 -id3v2_version 0 $r.mp3
done
printf "file '%s'\nfile '%s'\n" "$work/44100.mp3" "$work/32000.mp3" > mp3.txt
F -f concat -safe 0 -i mp3.txt -c copy "$out/mp3_rate_44100_32000.mka"

# VP8 / VP9, 5 + 5 frames: 64x48 then 96x64, in WebM.
for c in vp8:libvpx vp9:libvpx-vp9; do
  n=${c%%:*}; e=${c##*:}
  for s in 64x48 96x64; do
    F -f lavfi -i testsrc2=size=$s:rate=10 -frames:v 5 -c:v $e -g 5 -auto-alt-ref 0 \
      -lag-in-frames 0 $n-$s.webm
  done
  printf "file '%s'\nfile '%s'\n" "$work/$n-64x48.webm" "$work/$n-96x64.webm" > $n.txt
  F -f concat -safe 0 -i $n.txt -c copy "$out/${n}_resize_64x48_96x64.webm"
done

# Motion-JPEG, 2 + 2 + 2 pictures: 64x48, 48x32, 64x48, in AVI.
for s in 64x48 48x32; do
  F -f lavfi -i testsrc2=size=$s:rate=10 -frames:v 2 -c:v mjpeg -pix_fmt yuvj420p -f mjpeg $s.mjpeg
done
cat 64x48.mjpeg 48x32.mjpeg 64x48.mjpeg > mixed.mjpeg
F -framerate 10 -f mjpeg -i mixed.mjpeg -c copy -r 10 "$out/mjpeg_mixed_64x48_48x32_64x48.avi"
