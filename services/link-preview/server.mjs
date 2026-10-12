// Buzz link preview helper. NOT deployed; see README.md for the host plan.
//   POST /session            NIP-98 signed → { token, exp } bearer (one signature per SESSION_TTL_S, default 12 h)
//   GET  /preview?url=...    Bearer → LinkPreviewMetadata JSON (same shape as the desktop command), 204 when none
//   GET  /healthz
import http from "node:http";
import { checkSession, checkTargetUrl, Lru, mintSession, parseMetadata, RateLimit, sniffImage, verifyNip98 } from "./lib.mjs";
import { guardedGet as defaultGet } from "./fetcher.mjs";

export function createServer(cfg, { guardedGet = defaultGet } = {}) {
  const secret = cfg.sessionSecret;
  if (!secret || secret.length < 32) throw new Error("LINK_PREVIEW_SESSION_SECRET must be at least 32 characters");
  const publicUrl = cfg.publicUrl.replace(/\/+$/, "");
  const origins = new Set(cfg.allowedOrigins);
  const allowPubkeys = cfg.allowedPubkeys?.length ? new Set(cfg.allowedPubkeys) : null;
  const cache = new Lru(cfg.cacheEntries ?? 2000, (cfg.cacheTtlS ?? 86400) * 1000);
  const perUser = new RateLimit(cfg.perMinute ?? 60);
  const sessions = new RateLimit(10);
  const inflight = new Map();

  async function build(target) {
    const page = await guardedGet(target.toString(), { maxBytes: 512 * 1024 });
    const type = String(page.headers["content-type"] || "");
    if (page.status < 200 || page.status >= 300 || !/text\/html|application\/xhtml/i.test(type)) return null;
    const meta = parseMetadata(page.body.toString("utf8"), page.url);
    if (!meta) return null;
    let imageDataUrl = null;
    let imageFetchState = "none";
    if (meta.image) {
      try {
        const img = await guardedGet(meta.image, { maxBytes: 1024 * 1024, accept: "image/png,image/jpeg,image/webp,image/gif" });
        const mime = img.status === 200 && !img.truncated && !img.tooLarge ? sniffImage(img.body) : null;
        if (mime) {
          imageDataUrl = `data:${mime};base64,${img.body.toString("base64")}`;
          imageFetchState = "image";
        } else imageFetchState = "rejected";
      } catch {
        imageFetchState = "transient_failure";
      }
    }
    return {
      title: meta.title,
      siteName: meta.siteName,
      description: meta.description,
      imageDataUrl,
      imageDomain: meta.image ? new URL(meta.image).hostname : null,
      imageFetchState,
      imageRetryAfterMs: null,
      faviconDataUrl: null,
    };
  }

  function cors(req, res) {
    const o = req.headers.origin;
    if (o && origins.has(o)) {
      res.setHeader("Access-Control-Allow-Origin", o);
      res.setHeader("Vary", "Origin");
      res.setHeader("Access-Control-Allow-Headers", "Authorization");
      res.setHeader("Access-Control-Allow-Methods", "GET, POST");
      res.setHeader("Access-Control-Max-Age", "600");
    }
  }
  const send = (res, code, body) => {
    res.statusCode = code;
    res.setHeader("Cache-Control", "no-store");
    res.setHeader("X-Content-Type-Options", "nosniff");
    if (body === undefined) return res.end();
    res.setHeader("Content-Type", "application/json");
    res.end(JSON.stringify(body));
  };

  return http.createServer(async (req, res) => {
    try {
      cors(req, res);
      const u = new URL(req.url, "http://x");
      if (req.method === "OPTIONS") return send(res, 204);
      if (req.method === "GET" && u.pathname === "/healthz") return send(res, 200, { ok: true });
      if (req.method === "POST" && u.pathname === "/session") {
        let pk;
        try {
          pk = verifyNip98(req.headers.authorization, "POST", `${publicUrl}/session`);
        } catch (e) {
          return send(res, 401, { error: e.message });
        }
        if (allowPubkeys && !allowPubkeys.has(pk)) return send(res, 403, { error: "not allowed" });
        if (!sessions.take(pk)) return send(res, 429, { error: "slow down" });
        return send(res, 200, mintSession(secret, pk, cfg.sessionTtlS ?? 12 * 3600));
      }
      if (req.method === "GET" && u.pathname === "/preview") {
        const auth = req.headers.authorization || "";
        const pk = auth.startsWith("Bearer ") ? checkSession(secret, auth.slice(7)) : null;
        if (!pk || (allowPubkeys && !allowPubkeys.has(pk))) return send(res, 401, { error: "session required" });
        const target = checkTargetUrl(u.searchParams.get("url"));
        if (!target) return send(res, 400, { error: "url not allowed" });
        const key = target.toString();
        const hit = cache.get(key);
        if (hit !== undefined) return hit ? send(res, 200, hit) : send(res, 204);
        if (!perUser.take(pk)) return send(res, 429, { error: "slow down" });
        let p = inflight.get(key);
        if (!p) {
          p = build(target).finally(() => inflight.delete(key));
          inflight.set(key, p);
        }
        let v;
        try {
          v = await p;
        } catch {
          cache.set(key, null, Date.now(), 5 * 60_000); // brief negative cache for failures
          return send(res, 204);
        }
        cache.set(key, v);
        return v ? send(res, 200, v) : send(res, 204);
      }
      return send(res, 404, { error: "not found" });
    } catch {
      return send(res, 500, { error: "internal" });
    }
  });
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const list = (v) => (v || "").split(",").map((s) => s.trim()).filter(Boolean);
  const cfg = {
    sessionSecret: process.env.LINK_PREVIEW_SESSION_SECRET,
    publicUrl: process.env.LINK_PREVIEW_PUBLIC_URL || "",
    allowedOrigins: list(process.env.LINK_PREVIEW_ALLOWED_ORIGINS || "https://hulabuzz.huladesk.com"),
    allowedPubkeys: list(process.env.LINK_PREVIEW_ALLOWED_PUBKEYS).map((s) => s.toLowerCase()),
    perMinute: Number(process.env.LINK_PREVIEW_PER_MINUTE || 60),
  };
  if (!cfg.publicUrl) throw new Error("LINK_PREVIEW_PUBLIC_URL is required (the exact https origin+path clients sign for)");
  const port = Number(process.env.PORT || 8787);
  createServer(cfg).listen(port, process.env.HOST || "0.0.0.0", () => console.log(`link-preview listening on ${port}`));
}
