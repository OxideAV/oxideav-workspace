# oxideplay

Reference media player for the [oxideav](https://github.com/OxideAV/oxideav-workspace)
framework. Uses the library's pure-Rust demuxers + decoders and hands decoded
frames to SDL2 for audio output and (optionally) video display.

## Build requirement: SDL2

Unlike the rest of the workspace, this binary crate links against SDL2 via
`rust-sdl2` + `sdl2-sys` (C). Install SDL2 first:

- **Gentoo**: `sudo emerge media-libs/libsdl2`
- **Debian / Ubuntu**: `sudo apt install libsdl2-dev`
- **Fedora**: `sudo dnf install SDL2-devel`
- **macOS (Homebrew)**: `brew install sdl2`

## Usage

```sh
cargo run -p oxideplay -- path/to/file.flac
cargo run -p oxideplay -- --dry-run path/to/file.mp4   # probe & exit
cargo run -p oxideplay -- --no-video path/to/file.mp4  # audio only
```

Keybinds (same in window and TUI):

| Key | Action |
| --- | --- |
| `q`, `Esc` | Quit |
| `space` | Pause / resume |
| `←` / `→` | Seek ±5 s |
| `shift+←` / `shift+→` | Seek ±30 s |
| `↑` / `↓` | Volume ±5 % |

When stdout is a TTY, a one-line status bar is shown. When it's piped,
a simple progress message is emitted to stderr every ~1 s instead.

## On-screen overlay (winit + wgpu)

When the `winit` feature is on (default), the wgpu video output draws an
[egui](https://docs.rs/egui)-based control overlay on top of the video:
play / pause toggle, draggable seek bar with hover preview, MM:SS time
display, volume slider + mute button, ±10 s skip, and a stats panel
(toggled with the `i` button) showing resolution, codec, and duration.

The overlay auto-hides ~3 s after the cursor stops moving during active
playback, fades back in on cursor motion, and stays visible while paused
(VLC-style). Click the seek bar to jump anywhere in the timeline, drag
the volume slider to set absolute volume, click the speaker to toggle
mute. The keyboard shortcuts above continue to work alongside the
mouse UI.

## System "Now Playing" integration (`media-controls`)

Optional, off by default. When built with `--features media-controls`,
oxideplay publishes the current track's title / artist / album /
artwork / duration / position to the OS's media widget and accepts
the system's media-key + lock-screen + Touch-Bar commands back into
the engine.

| Platform | Status |
| --- | --- |
| macOS | Implemented. Uses `MPNowPlayingInfoCenter` + `MPRemoteCommandCenter` from `MediaPlayer.framework`, all loaded at runtime via `libloading` so the binary's `LC_LOAD_DYLIB` list is unchanged. Verify with `otool -L target/release/oxideplay \| grep MediaPlayer` (no output expected). |
| Linux (MPRIS over D-Bus) | TODO — follow-up will runtime-load `libdbus-1.so.3`. |
| Windows (SMTC via `combase.dll`) | TODO. |

Build + run on macOS:

```sh
cargo build -p oxideplay --features media-controls --release
./target/release/oxideplay path/to/cyber.mod
# Open Control Center / Touch Bar — "cyber" appears as the title.
```

Out-of-the-box this surfaces:

- MOD / S3M / IT / XM (oxideav-mod) — title only (the format carries no
  artist/album/cover).
- MP3 (oxideav-id3) — title, artist, album, genre, cover art.
- FLAC (oxideav-flac) — Vorbis comments + `METADATA_BLOCK_PICTURE`.
- MP4 / M4A (oxideav-mp4) — `©nam` / `©ART` / `©alb` + `covr` atoms.
- Ogg/Vorbis — Vorbis comments.

Headphone play/pause, Touch Bar buttons, lock-screen scrub, and the
Now Playing widget all route into the player as `TogglePause` /
`SeekAbsolute` engine events.
