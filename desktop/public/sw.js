// Buzz web app-shell service worker.
// Caches ONLY the static shell (hashed /assets/*, fonts/icons/css, and the navigation document).
// Never touches: cross-origin requests, non-GET, the relay API paths (/query /events /count /media /upload /api /info /health ...),
// or WebSocket upgrades (the SW cannot intercept those anyway).
const VERSION = "buzz-web-v3";
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

// ---- Web Push (payloads come from the Hula Buzz push sender; they never carry message secrets) ----
self.addEventListener("push", (e) => {
  let data = {};
  try {
    data = e.data ? e.data.json() : {};
  } catch {
    data = { title: "Hula Buzz", body: e.data ? e.data.text() : "" };
  }
  const title = typeof data.title === "string" && data.title ? data.title : "Hula Buzz";
  e.waitUntil(
    self.registration.showNotification(title, {
      body: typeof data.body === "string" ? data.body : "",
      tag: typeof data.tag === "string" ? data.tag : undefined,
      icon: "/pwa/icon-192.png",
      badge: "/pwa/icon-192.png",
      data: { target: data.target || null },
    }),
  );
});
self.addEventListener("notificationclick", (e) => {
  e.notification.close();
  const target = (e.notification.data && e.notification.data.target) || null;
  e.waitUntil(
    self.clients.matchAll({ type: "window", includeUncontrolled: true }).then((list) => {
      const client = list.find((c) => new URL(c.url).origin === self.location.origin);
      if (client) {
        if (target) client.postMessage({ type: "buzz-push-open", target });
        return client.focus();
      }
      const url = target ? `/?push=${encodeURIComponent(JSON.stringify(target))}` : "/";
      return self.clients.openWindow(url);
    }),
  );
});
