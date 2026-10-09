#!/usr/bin/env bash
# Build dist/vidgif-linux-x86_64.tar.gz and dist/vidgif-linux-x86_64.AppImage (run from repo root).
set -euo pipefail
cargo build --locked --release -p vidgif-app
rm -rf dist AppDir && mkdir -p dist AppDir/usr/bin
bash scripts/fetch-ffmpeg.sh linux AppDir/usr/bin
cp target/release/vidgif-app AppDir/usr/bin/vidgif
cp assets/icon.png AppDir/vidgif.png
cat > AppDir/vidgif.desktop <<'DESK'
[Desktop Entry]
Type=Application
Name=vidgif
Comment=Convert videos to GIFs
Exec=vidgif
Icon=vidgif
Categories=AudioVideo;Video;
DESK
printf '#!/bin/sh\nexec "$(dirname "$(readlink -f "$0")")/usr/bin/vidgif" "$@"\n' > AppDir/AppRun
chmod +x AppDir/AppRun
# tarball
mkdir -p vidgif-linux-x86_64 && cp AppDir/usr/bin/* vidgif-linux-x86_64/ && cp README.md LICENSE-* THIRD_PARTY.md vidgif-linux-x86_64/
tar -czf dist/vidgif-linux-x86_64.tar.gz vidgif-linux-x86_64
# AppImage
curl -fsSL -o appimagetool https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage
chmod +x appimagetool
ARCH=x86_64 ./appimagetool --appimage-extract-and-run AppDir dist/vidgif-linux-x86_64.AppImage
