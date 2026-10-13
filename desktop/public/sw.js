// Buzz web app-shell service worker.
// Caches ONLY the static shell (hashed /assets/*, fonts/icons/css, and the navigation document).
// Never touches: cross-origin requests, non-GET, the relay API paths (/query /events /count /media /upload /api /info /health ...),
// or WebSocket upgrades (the SW cannot intercept those anyway).
const VERSION = "buzz-web-v8";
const PREFETCH_CACHE = "buzz-push-prefetch-v1"; // kept across shell versions; entries are short-lived
const SHELL = ["/", "/manifest.webmanifest", "/buzz.svg", "/boot.css", "/pwa/icon-192.png", "/pwa/icon-512.png"];
const API = /^\/(query|events|count|media|upload|api|info|health|_liveness|_readiness|workflows|hooks|huddle|operator|invite|\.well-known)(\/|$)/;

self.addEventListener("install", (e) => {
  e.waitUntil(caches.open(VERSION).then((c) => c.addAll(SHELL)).then(() => self.skipWaiting()));
});
self.addEventListener("activate", (e) => {
  e.waitUntil(caches.keys().then((ks) => Promise.all(ks.filter((k) => k !== VERSION && k !== PREFETCH_CACHE).map((k) => caches.delete(k)))).then(() => self.clients.claim()));
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

// ---- Web Push (payloads come from the Hula Buzz push gateway; they never carry message content) ----
// Tiny IndexedDB key/value store shared with the page (platform/web/push/pending.ts): "pending" = the conversation the last
// push was about (so the app reconnects and opens there first), "prefetch-enabled" = page-set flag for background prefetch.
function idb() {
  return new Promise((resolve, reject) => {
    const r = indexedDB.open("buzz-push", 1);
    r.onupgradeneeded = () => r.result.createObjectStore("kv");
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error);
  });
}
function kv(mode, fn) {
  return idb().then((db) => new Promise((resolve, reject) => {
    const tx = db.transaction("kv", mode);
    const req = fn(tx.objectStore("kv"));
    tx.oncomplete = () => resolve(req && req.result);
    tx.onerror = () => reject(tx.error);
  }));
}
const kvGet = (k) => kv("readonly", (s) => s.get(k)).catch(() => undefined);
const kvSet = (k, v) => kv("readwrite", (s) => s.put(v, k)).catch(() => undefined);

// Background prefetch (off until the relay issues scoped read tokens): fetch the new events for exactly the pushed
// conversation with the short-lived token the push carried, and cache them for instant display. Same-relay HTTPS only.
async function prefetch(data) {
  const p = data && data.prefetch;
  if (!p || typeof p.url !== "string" || typeof p.token !== "string") return;
  if ((await kvGet("prefetch-enabled")) !== true) return;
  let url;
  try {
    url = new URL(p.url);
  } catch {
    return;
  }
  const allowed = await kvGet("prefetch-origin");
  if (url.protocol !== "https:" || url.origin !== allowed) return;
  const res = await fetch(url.href, { headers: { Authorization: `Bearer ${p.token}` }, credentials: "omit", cache: "no-store" }).catch(() => null);
  if (!res || !res.ok) return;
  const key = `/__push_prefetch/${encodeURIComponent((data.target && (data.target.threadRootId || data.target.channelId)) || "x")}`;
  const cache = await caches.open(PREFETCH_CACHE);
  await cache.put(key, new Response(await res.text(), { headers: { "Content-Type": "application/json", "X-Fetched-At": String(Date.now()) } }));
}

// Home Screen badge: the page writes the real unread total to IDB "badge" whenever it runs; each push while closed adds one.
async function bumpBadge() {
  const n = ((await kvGet("badge")) || 0) + 1;
  await kvSet("badge", n);
  if (self.navigator && self.navigator.setAppBadge) await self.navigator.setAppBadge(n).catch(() => undefined);
}

// Event ids this SW showed a push notification for; the page never fires a local notification for them.
async function rememberNotified(id) {
  const list = (await kvGet("notified")) || [];
  if (!list.includes(id)) list.push(id);
  await kvSet("notified", list.slice(-200));
}

self.addEventListener("push", (e) => {
  let data = {};
  try {
    data = e.data ? e.data.json() : {};
  } catch {
    data = { title: "Hula Buzz", body: e.data ? e.data.text() : "" };
  }
  const title = typeof data.title === "string" && data.title ? data.title : "Hula Buzz";
  const target = data.target && typeof data.target === "object" ? data.target : null;
  e.waitUntil(
    Promise.all([
      self.registration.showNotification(title, {
        body: typeof data.body === "string" ? data.body : "",
        // One notification per conversation: a repeat push for the same thread/DM replaces (and re-alerts) instead of stacking.
        tag: target && (target.threadRootId || target.channelId) ? `buzz-c-${target.threadRootId || target.channelId}` : typeof data.tag === "string" ? data.tag : undefined,
        renotify: !!(target && (target.threadRootId || target.channelId)),
        icon: "/pwa/icon-192.png",
        badge: "/pwa/icon-192.png",
        data: { target },
      }),
      target ? kvSet("pending", { target, at: Date.now() }) : null,
      prefetch(data).catch(() => undefined),
      bumpBadge().catch(() => undefined),
      target && target.eventId ? rememberNotified(target.eventId).catch(() => undefined) : null,
      // Always show the notification above (iOS userVisibleOnly: a push without one risks revocation). An open page
      // hears about it here and its in-app OS notification for the same event stands down (platform/web/push/seen.ts).
      self.clients.matchAll({ type: "window" }).then((list) => list.forEach((c) => c.postMessage({ type: "buzz-push-arrived", target }))),
    ]),
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
