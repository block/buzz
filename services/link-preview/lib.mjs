// Pure helpers for the link preview helper (unit-tested in test/).
import { createHmac, timingSafeEqual } from "node:crypto";
import { isIP } from "node:net";
import { schnorr } from "@noble/curves/secp256k1.js";
import { sha256 } from "@noble/hashes/sha2.js";
import { bytesToHex, hexToBytes } from "@noble/hashes/utils.js";

// ── Address policy ──────────────────────────────────────────────────────────
const V4_BLOCKED = [
  ["0.0.0.0", 8], ["10.0.0.0", 8], ["100.64.0.0", 10], ["127.0.0.0", 8], ["169.254.0.0", 16], ["172.16.0.0", 12],
  ["192.0.0.0", 24], ["192.0.2.0", 24], ["192.168.0.0", 16], ["198.18.0.0", 15], ["198.51.100.0", 24], ["203.0.113.0", 24],
  ["224.0.0.0", 4], ["240.0.0.0", 4],
];
const v4num = (ip) => ip.split(".").reduce((n, o) => (n << 8) + Number(o), 0) >>> 0;
function v4Blocked(ip) {
  const n = v4num(ip);
  return V4_BLOCKED.some(([base, bits]) => (n >>> (32 - bits)) === (v4num(base) >>> (32 - bits)));
}
function v6Words(ip) {
  let [head, tail] = ip.toLowerCase().split("%")[0].split("::");
  const parse = (s) => (s ? s.split(":") : []);
  let h = parse(head), t = tail === undefined ? [] : parse(tail);
  const last = (tail === undefined ? h : t).at(-1);
  let v4tail = null;
  if (last && last.includes(".")) {
    v4tail = last;
    (tail === undefined ? h : t).pop();
  }
  const fill = 8 - h.length - t.length - (v4tail ? 2 : 0);
  const words = [...h, ...(tail === undefined ? [] : Array(fill).fill("0")), ...t].map((w) => parseInt(w || "0", 16));
  if (v4tail) {
    const n = v4num(v4tail);
    words.push(n >>> 16, n & 0xffff);
  }
  return words;
}
/** True for loopback, private, link-local, CGNAT, multicast, documentation and other non-public addresses. */
export function isBlockedAddress(ip) {
  const fam = isIP(ip);
  if (fam === 4) return v4Blocked(ip);
  if (fam !== 6) return true;
  const w = v6Words(ip);
  if (w.length !== 8 || w.some((x) => Number.isNaN(x))) return true;
  if (w.every((x) => x === 0)) return true; // ::
  if (w.slice(0, 7).every((x) => x === 0) && w[7] === 1) return true; // ::1
  if (w.slice(0, 5).every((x) => x === 0) && (w[5] === 0xffff || w[5] === 0)) {
    return v4Blocked(`${w[6] >>> 8}.${w[6] & 255}.${w[7] >>> 8}.${w[7] & 255}`); // v4-mapped / compat
  }
  if (w[0] === 0x64 && w[1] === 0xff9b) return true; // NAT64
  if ((w[0] & 0xfe00) === 0xfc00) return true; // ULA
  if ((w[0] & 0xffc0) === 0xfe80) return true; // link-local
  if ((w[0] & 0xff00) === 0xff00) return true; // multicast
  if (w[0] === 0x2001 && w[1] === 0x0db8) return true; // documentation
  if (w[0] === 0x2002) return true; // 6to4 (can embed private v4)
  return false;
}

/** Only http(s) on default ports, no credentials, ≤ 2 KB. Returns a URL or null. */
export function checkTargetUrl(raw) {
  if (typeof raw !== "string" || raw.length > 2048) return null;
  let u;
  try {
    u = new URL(raw);
  } catch {
    return null;
  }
  if (u.protocol !== "http:" && u.protocol !== "https:") return null;
  if (u.username || u.password) return null;
  if (u.port && !((u.protocol === "http:" && u.port === "80") || (u.protocol === "https:" && u.port === "443"))) return null;
  const host = u.hostname.replace(/^\[|\]$/g, "");
  if (!host || host === "localhost" || host.endsWith(".localhost") || host.endsWith(".local") || host.endsWith(".internal")) return null;
  if (isIP(host) && isBlockedAddress(host)) return null;
  u.hash = "";
  return u;
}

// ── NIP-98 + sessions ───────────────────────────────────────────────────────
export function eventId(ev) {
  return bytesToHex(sha256(new TextEncoder().encode(JSON.stringify([0, ev.pubkey, ev.created_at, ev.kind, ev.tags, ev.content]))));
}
const tagv = (ev, n) => ev.tags?.find((t) => t[0] === n)?.[1];
/** Verifies a `Nostr <base64 event>` header for (method, url). Returns the signer pubkey or throws. */
export function verifyNip98(header, method, url, nowS = Math.floor(Date.now() / 1000)) {
  if (typeof header !== "string" || !header.startsWith("Nostr ")) throw new Error("missing nip98");
  let ev;
  try {
    ev = JSON.parse(Buffer.from(header.slice(6), "base64").toString("utf8"));
  } catch {
    throw new Error("bad nip98");
  }
  if (ev?.kind !== 27235 || typeof ev.pubkey !== "string" || !/^[0-9a-f]{64}$/.test(ev.pubkey)) throw new Error("bad nip98");
  if (!Number.isInteger(ev.created_at) || Math.abs(nowS - ev.created_at) > 60) throw new Error("stale nip98");
  if (tagv(ev, "u") !== url || (tagv(ev, "method") || "").toUpperCase() !== method) throw new Error("nip98 target mismatch");
  if (eventId(ev) !== ev.id) throw new Error("bad nip98 id");
  if (!schnorr.verify(hexToBytes(ev.sig), hexToBytes(ev.id), hexToBytes(ev.pubkey))) throw new Error("bad nip98 sig");
  return ev.pubkey;
}
const b64u = (b) => Buffer.from(b).toString("base64url");
export function mintSession(secret, pubkey, ttlS, nowS = Math.floor(Date.now() / 1000)) {
  const exp = nowS + ttlS;
  const body = `${pubkey}.${exp}`;
  return { token: `${body}.${b64u(createHmac("sha256", secret).update(body).digest())}`, exp };
}
/** Returns the pubkey for a valid unexpired bearer token, else null. */
export function checkSession(secret, token, nowS = Math.floor(Date.now() / 1000)) {
  const m = /^([0-9a-f]{64})\.(\d{1,12})\.([A-Za-z0-9_-]{43})$/.exec(token || "");
  if (!m || Number(m[2]) < nowS) return null;
  const want = createHmac("sha256", secret).update(`${m[1]}.${m[2]}`).digest();
  const got = Buffer.from(m[3], "base64url");
  return got.length === want.length && timingSafeEqual(got, want) ? m[1] : null;
}

// ── HTML metadata ───────────────────────────────────────────────────────────
const ENT = { amp: "&", lt: "<", gt: ">", quot: '"', apos: "'", nbsp: " " };
export function decodeEntities(s) {
  return s.replace(/&(#x[0-9a-f]+|#\d+|[a-z]+);/gi, (m, e) => {
    if (e[0] === "#") {
      const n = e[1] === "x" || e[1] === "X" ? parseInt(e.slice(2), 16) : parseInt(e.slice(1), 10);
      return n > 0 && n < 0x110000 ? String.fromCodePoint(n) : m;
    }
    return ENT[e.toLowerCase()] ?? m;
  });
}
const clean = (s, max) => {
  if (typeof s !== "string") return null;
  const t = decodeEntities(s).replace(/[\u0000-\u001f\u007f]/g, " ").replace(/\s+/g, " ").trim();
  return t ? [...t].slice(0, max).join("") : null;
};
function attrs(tag) {
  const out = {};
  for (const m of tag.matchAll(/([a-zA-Z:_-]+)\s*=\s*("([^"]*)"|'([^']*)'|([^\s>]+))/g)) out[m[1].toLowerCase()] = m[3] ?? m[4] ?? m[5] ?? "";
  return out;
}
/** og:/twitter:/<title> metadata from an HTML head. Image URL is resolved against `base` and re-checked. */
export function parseMetadata(html, base) {
  const head = html.slice(0, 512 * 1024);
  const meta = {};
  for (const m of head.matchAll(/<meta\b[^>]*>/gi)) {
    const a = attrs(m[0]);
    const k = (a.property || a.name || "").toLowerCase();
    if (k && a.content !== undefined && !(k in meta)) meta[k] = a.content;
  }
  const titleTag = /<title[^>]*>([\s\S]{0,2000}?)<\/title>/i.exec(head)?.[1];
  const title = clean(meta["og:title"] ?? meta["twitter:title"] ?? titleTag, 180);
  if (!title) return null;
  let image = meta["og:image:secure_url"] ?? meta["og:image"] ?? meta["twitter:image"] ?? null;
  if (image) {
    try {
      image = checkTargetUrl(new URL(decodeEntities(image), base).toString())?.toString() ?? null;
    } catch {
      image = null;
    }
  }
  return {
    title,
    siteName: clean(meta["og:site_name"], 180),
    description: clean(meta["og:description"] ?? meta["twitter:description"] ?? meta.description, 280),
    image,
  };
}

/** Raster image types only (no SVG), identified by magic bytes. */
export function sniffImage(buf) {
  if (buf.length >= 8 && buf[0] === 0x89 && buf[1] === 0x50 && buf[2] === 0x4e && buf[3] === 0x47) return "image/png";
  if (buf.length >= 3 && buf[0] === 0xff && buf[1] === 0xd8 && buf[2] === 0xff) return "image/jpeg";
  if (buf.length >= 6 && buf.subarray(0, 6).toString("latin1").startsWith("GIF8")) return "image/gif";
  if (buf.length >= 12 && buf.subarray(0, 4).toString("latin1") === "RIFF" && buf.subarray(8, 12).toString("latin1") === "WEBP") return "image/webp";
  return null;
}

// ── Small LRU / rate limit ──────────────────────────────────────────────────
export class Lru {
  constructor(max, ttlMs) {
    this.max = max;
    this.ttlMs = ttlMs;
    this.m = new Map();
  }
  get(k, now = Date.now()) {
    const v = this.m.get(k);
    if (!v) return undefined;
    if (v.exp < now) {
      this.m.delete(k);
      return undefined;
    }
    this.m.delete(k);
    this.m.set(k, v);
    return v.value;
  }
  set(k, value, now = Date.now(), ttlMs = this.ttlMs) {
    this.m.delete(k);
    this.m.set(k, { value, exp: now + ttlMs });
    while (this.m.size > this.max) this.m.delete(this.m.keys().next().value);
  }
}
export class RateLimit {
  constructor(perMin) {
    this.perMin = perMin;
    this.b = new Map();
  }
  take(key, now = Date.now()) {
    const w = Math.floor(now / 60000);
    const v = this.b.get(key);
    if (!v || v.w !== w) {
      this.b.set(key, { w, n: 1 });
      if (this.b.size > 10000) this.b.clear();
      return true;
    }
    return ++v.n <= this.perMin;
  }
}
