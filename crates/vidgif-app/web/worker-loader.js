// Classic-worker entry point: trunk emits the encoder as a no-modules wasm-bindgen script.
importScripts("./worker.js");
wasm_bindgen({ module_or_path: "./worker_bg.wasm" });
