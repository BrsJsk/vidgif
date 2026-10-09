#!/usr/bin/env bash
# Builds assets/icon.png, icon.icns and icon.ico from assets/full-bleed-icon.png.
# Needs ffmpeg and python3; the .icns step needs macOS (sips, iconutil). Run from the repo root.
set -euo pipefail
src=assets/full-bleed-icon.png
tmp="$(mktemp -d)"

# Rounded corners (radius ~22% like the macOS icon shape), at 1024px.
r=229
round="format=rgba,geq=r='r(X,Y)':g='g(X,Y)':b='b(X,Y)':a='if(gt(abs(W/2-X),W/2-$r)*gt(abs(H/2-Y),H/2-$r),if(lte(hypot($r-(W/2-abs(W/2-X)),$r-(H/2-abs(H/2-Y))),$r),255,0),255)'"
ffmpeg -loglevel error -y -i "$src" -vf "$round" "$tmp/rounded.png"

# Window / Linux icon.
ffmpeg -loglevel error -y -i "$tmp/rounded.png" -vf scale=512:512:flags=lanczos assets/icon.png

# Web app icons.
ffmpeg -loglevel error -y -i "$tmp/rounded.png" -vf scale=192:192:flags=lanczos crates/vidgif-app/web/icon-192.png
cp assets/icon.png crates/vidgif-app/web/icon-512.png

# Windows .ico: PNG-compressed entries.
for s in 16 32 48 64 128 256; do
  ffmpeg -loglevel error -y -i "$tmp/rounded.png" -vf scale=$s:$s:flags=lanczos "$tmp/ico$s.png"
done
python3 - "$tmp" assets/icon.ico <<'PY'
import struct, sys
tmp, out = sys.argv[1:]
sizes = [16, 32, 48, 64, 128, 256]
images = [open(f"{tmp}/ico{s}.png", "rb").read() for s in sizes]
data = struct.pack("<HHH", 0, 1, len(sizes))
offset = 6 + 16 * len(sizes)
for s, img in zip(sizes, images):
    data += struct.pack("<BBBBHHII", s % 256, s % 256, 0, 0, 1, 32, len(img), offset)
    offset += len(img)
open(out, "wb").write(data + b"".join(images))
PY

# macOS .icns: icons sit inside a margin on a 1024 canvas.
if command -v iconutil >/dev/null; then
  ffmpeg -loglevel error -y -i "$tmp/rounded.png" \
    -vf "scale=824:824:flags=lanczos,pad=1024:1024:100:100:color=black@0" "$tmp/mac.png"
  set_dir="$tmp/icon.iconset"; mkdir "$set_dir"
  for s in 16 32 128 256 512; do
    sips -z $s $s "$tmp/mac.png" --out "$set_dir/icon_${s}x${s}.png" >/dev/null
    sips -z $((s * 2)) $((s * 2)) "$tmp/mac.png" --out "$set_dir/icon_${s}x${s}@2x.png" >/dev/null
  done
  iconutil -c icns "$set_dir" -o assets/icon.icns
fi
rm -rf "$tmp"
