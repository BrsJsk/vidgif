# Third-party software

Release builds of vidgif bundle a separate `ffmpeg` executable, which vidgif runs as a
subprocess to decode video. vidgif's own code is MIT OR Apache-2.0; ffmpeg is not covered by that license.

| Platform | ffmpeg build | License |
|---|---|---|
| Windows, Linux | [BtbN/FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds) (`*-lgpl` variant) | LGPL-2.1+ |
| macOS | [martin-riedl.de](https://ffmpeg.martin-riedl.de) static builds | GPL-2.0+ (includes GPL components) |

FFmpeg source code: <https://ffmpeg.org/download.html>. Each build's exact source and
build configuration is published by its provider at the links above.
You can replace the bundled binary with your own by setting `VIDGIF_FFMPEG=/path/to/ffmpeg`.
