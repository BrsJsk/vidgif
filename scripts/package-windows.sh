#!/usr/bin/env bash
# Build dist/vidgif-windows-x86_64.zip (run from repo root in Git Bash on Windows).
set -euo pipefail
cargo build --locked --release -p vidgif-app
rm -rf dist vidgif-windows-x86_64 && mkdir -p dist vidgif-windows-x86_64
cp target/release/vidgif-app.exe vidgif-windows-x86_64/vidgif.exe
bash scripts/fetch-ffmpeg.sh windows vidgif-windows-x86_64
cp README.md LICENSE-* THIRD_PARTY.md vidgif-windows-x86_64/
7z a -tzip dist/vidgif-windows-x86_64.zip vidgif-windows-x86_64 >/dev/null
