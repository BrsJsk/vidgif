# vidgif

Convert a video to a GIF, entirely locally (no server, no API). Rust + egui.

Runs as a desktop app (Windows, macOS, Linux) and in the browser. Nothing is uploaded anywhere.

## Run (desktop)

Needs `ffmpeg`: release builds bundle it next to the executable; when building from source, install it
(e.g. `brew install ffmpeg`) or set `VIDGIF_FFMPEG=/path/to/ffmpeg`. See `THIRD_PARTY.md`.

```sh
cargo run --release -p vidgif-app
```

Features: trim, crop (drag + numeric + presets), speed, fps, width, quality presets,
loop toggle, size estimate, result preview, save.

## Run (web)

```sh
rustup target add wasm32-unknown-unknown
cargo install --locked trunk
cd crates/vidgif-app
trunk serve --release
```

The browser decodes the video itself (so it handles whatever the browser can play, typically MP4/H.264
and WebM) and the GIF is encoded in a Web Worker. Output width is capped at 1280 px.
`trunk build --release` writes the static site to `crates/vidgif-app/dist`; it is also an installable
PWA that works offline after the first visit.

The `Web` workflow (`.github/workflows/web.yml`) builds the site on every push to `main` and on PRs, and
attaches it to the run as the `vidgif-web` artifact. To publish, download and unzip that artifact and upload the
folder in Cloudflare Pages (Workers & Pages → Create → Pages → Upload assets).

## Layout

- `crates/vidgif-core` — UI-free logic: settings, timing, GIF encoder (NeuQuant + frame differencing), size estimate.
- `crates/vidgif-app` — egui UI (`app.rs`, `crop.rs`, `gif_preview.rs`) and `platform/`, the part that differs per target:
  `platform/native` decodes with ffmpeg on threads; `platform/web` decodes with a `<video>` element and encodes in
  the Web Worker built from `src/bin/worker.rs`. `web/` holds the PWA files (manifest, service worker, `_headers`).
- `crates/vidgif-app/examples/convert_cli.rs` — headless smoke test.
- `assets/` — app icon. Regenerate `icon.png`, `icon.ico` and `icon.icns` from `full-bleed-icon.png` with `scripts/make-icons.sh`.

## License

MIT OR Apache-2.0 (see `LICENSE-MIT`, `LICENSE-APACHE`). Bundled ffmpeg has its own license: `THIRD_PARTY.md`.

## Releases

Push a tag like `v0.1.0`: GitHub Actions builds Windows (.zip), macOS universal (.dmg, unsigned), Linux (.tar.gz + AppImage) and attaches them to a GitHub Release. `ci.yml` runs fmt/clippy/build on every push and PR.
