// Offline support: pre-cache the app on install, then serve network-first with a cache fallback.
const CACHE = "vidgif-v1";
const FIXED = [
  "./", "./manifest.json", "./icon-192.png", "./icon-512.png",
  "./worker-loader.js", "./worker.js", "./worker_bg.wasm",
];

self.addEventListener("install", (event) => {
  event.waitUntil((async () => {
    const cache = await caches.open(CACHE);
    // The main bundle has a content hash in its name, so read it out of index.html.
    const html = await (await fetch("./", { cache: "reload" })).text();
    const hashed = [...html.matchAll(/(?:href|from)=?\s*['"]\.?\/?(vidgif-app-[\w-]+(?:_bg\.wasm|\.js))['"]/g)]
      .map((m) => "./" + m[1]);
    await cache.addAll([...new Set([...FIXED, ...hashed])]);
    await self.skipWaiting();
  })());
});

self.addEventListener("activate", (event) => {
  event.waitUntil((async () => {
    for (const key of await caches.keys()) {
      if (key !== CACHE) await caches.delete(key);
    }
    await self.clients.claim();
  })());
});

self.addEventListener("fetch", (event) => {
  const { request } = event;
  if (request.method !== "GET" || new URL(request.url).origin !== location.origin) return;
  event.respondWith((async () => {
    const cache = await caches.open(CACHE);
    try {
      const response = await fetch(request);
      if (response.ok) cache.put(request, response.clone());
      return response;
    } catch (error) {
      const cached = await cache.match(request, { ignoreSearch: true });
      if (cached) return cached;
      throw error;
    }
  })());
});
