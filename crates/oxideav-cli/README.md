# oxideav-cli

Command-line frontend for oxideav

Part of the [oxideav](https://github.com/OxideAV/oxideav-workspace) framework — a
pure-Rust media transcoding and streaming stack. Every codec, container, and
filter is implemented from the spec — no C codec libraries linked or wrapped,
no `*-sys` crates. The optional hardware-acceleration crates
(`oxideav-videotoolbox` / `-audiotoolbox` / `-vaapi` / `-vdpau` / `-nvidia` /
`-vulkan-video`) bridge to OS-provided HW engines via `libloading` (runtime-
loaded, no compile-time link). Pass `--no-hwaccel` to opt out.

## Usage

```toml
[dependencies]
oxideav-cli = "0.0.1"
```

## License

MIT — see [LICENSE](https://github.com/OxideAV/oxideav-workspace/blob/master/LICENSE).
