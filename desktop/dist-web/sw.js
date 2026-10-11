// Buzz web app-shell service worker.
// Caches ONLY the static shell (hashed /assets/*, fonts/icons/css, and the navigation document).
// Never touches: cross-origin requests, non-GET, the relay API paths (/query /events /count /media /upload /api /info /health ...),
// or WebSocket upgrades (the SW cannot intercept those anyway).
const VERSION = "buzz-web-v2";
const SHELL = ["/", "/manifest.webmanifest", "/buzz.svg", "/boot.css", "/pwa/icon-192.png", "/pwa/icon-512.png"];
const API = /^\/(query|events|count|media|upload|api|info|health|_liveness|_readiness|workflows|hooks|huddle|operator|invite|\.well-known)(\/|$)/;

self.addEventListener("install", (e) => {
  e.waitUntil(caches.open(VERSION).then((c) => c.addAll(SHELL)).then(() => self.skipWaiting()));
});
self.addEventListener("activate", (e) => {
  e.waitUntil(caches.keys().then((ks) => Promise.all(ks.filter((k) => k !== VERSION).map((k) => caches.delete(k)))).then(() => self.clients.claim()));
});
self.addEventListener("fetch", (e) => {
  const req = e.request;
  const url = new URL(req.url);
  if (req.method !== "GET" || url.origin !== self.location.origin || API.test(url.pathname)) return;
  if (req.mode === "navigate") {
    // network-first so a new deploy is picked up; fall back to the cached shell offline
    e.respondWith(fetch(req).then((r) => { if (r.ok) { const cp = r.clone(); caches.open(VERSION).then((c) => c.put("/", cp)); } return r; }).catch(() => caches.match("/")));
    return;
  }
  if (url.pathname.startsWith("/assets/") || /\.(woff2?|png|svg|css|webmanifest|ico)$/.test(url.pathname)) {
    e.respondWith(caches.match(req).then((hit) => hit || fetch(req).then((r) => { if (r.ok) { const cp = r.clone(); caches.open(VERSION).then((c) => c.put(req, cp)); } return r; })));
  }
});
