#!/usr/bin/env bash
# Download a static ffmpeg for a platform into <outdir>.
# usage: fetch-ffmpeg.sh <linux|windows|macos-arm64|macos-x64> <outdir>
set -euo pipefail
os="$1"; out="$2"; mkdir -p "$out"; tmp="$(mktemp -d)"
case "$os" in
  linux)
    curl -fsSL -o "$tmp/f.tar.xz" https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-linux64-lgpl.tar.xz
    tar -xf "$tmp/f.tar.xz" -C "$tmp"; cp "$tmp"/ffmpeg-*/bin/ffmpeg "$out/ffmpeg" ;;
  windows)
    curl -fsSL -o "$tmp/f.zip" https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-lgpl.zip
    # Git Bash has no unzip, and the native 7z needs Windows-style paths.
    7z x -y -o"$(cygpath -w "$tmp")" "$(cygpath -w "$tmp/f.zip")" >/dev/null
    cp "$tmp"/ffmpeg-*/bin/ffmpeg.exe "$out/ffmpeg.exe" ;;
  macos-arm64|macos-x64)
    arch=arm64; [ "$os" = macos-x64 ] && arch=amd64
    curl -fsSL -o "$tmp/f.zip" "https://ffmpeg.martin-riedl.de/redirect/latest/macos/$arch/release/ffmpeg.zip"
    unzip -q "$tmp/f.zip" -d "$tmp"; cp "$tmp/ffmpeg" "$out/ffmpeg-$os" ;;
  *) echo "unknown os $os" >&2; exit 1 ;;
esac
chmod +x "$out"/ffmpeg* 2>/dev/null || true
rm -rf "$tmp"
