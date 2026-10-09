#!/usr/bin/env bash
# Build dist/vidgif-macos-universal.dmg (unsigned, ad-hoc signed). Run from repo root on macOS.
set -euo pipefail
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo build --locked --release -p vidgif-app --target aarch64-apple-darwin
cargo build --locked --release -p vidgif-app --target x86_64-apple-darwin
rm -rf dist && mkdir -p dist
APP=dist/vidgif.app; mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
lipo -create -output "$APP/Contents/MacOS/vidgif" \
  target/aarch64-apple-darwin/release/vidgif-app target/x86_64-apple-darwin/release/vidgif-app
bash scripts/fetch-ffmpeg.sh macos-arm64 dist/tmp-ff
bash scripts/fetch-ffmpeg.sh macos-x64 dist/tmp-ff
lipo -create -output "$APP/Contents/MacOS/ffmpeg" dist/tmp-ff/ffmpeg-macos-arm64 dist/tmp-ff/ffmpeg-macos-x64
rm -rf dist/tmp-ff
cp README.md LICENSE-* THIRD_PARTY.md assets/icon.icns "$APP/Contents/Resources/"
cat > "$APP/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>vidgif</string>
  <key>CFBundleDisplayName</key><string>vidgif</string>
  <key>CFBundleIdentifier</key><string>dev.vidgif.app</string>
  <key>CFBundleExecutable</key><string>vidgif</string>
  <key>CFBundleIconFile</key><string>icon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
</dict></plist>
PLIST
# Ad-hoc signature (no developer account) so Apple Silicon will launch it after the Gatekeeper prompt.
codesign --force --deep -s - "$APP"
hdiutil create -volname vidgif -srcfolder "$APP" -ov -format UDZO dist/vidgif-macos-universal.dmg
