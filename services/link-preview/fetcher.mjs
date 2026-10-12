// SSRF-guarded HTTP GET: every hop (≤ 3 redirects) is re-checked, DNS answers are validated and the socket is pinned to the
// validated address, bodies are capped and the whole fetch has one deadline.
import dns from "node:dns";
import http from "node:http";
import https from "node:https";
import { checkTargetUrl, isBlockedAddress } from "./lib.mjs";

const UA = "Mozilla/5.0 (compatible; BuzzLinkPreview/0.1; +https://hulabuzz.huladesk.com)";

export function guardedLookup(hostname, opts, cb) {
  dns.lookup(hostname, { all: true, verbatim: true }, (err, addrs) => {
    if (err) return cb(err);
    if (!addrs.length || addrs.some((a) => isBlockedAddress(a.address))) {
      const e = new Error(`blocked address for ${hostname}`);
      e.code = "EBLOCKED";
      return cb(e);
    }
    if (opts && opts.all) return cb(null, addrs);
    cb(null, addrs[0].address, addrs[0].family);
  });
}

function once(url, { maxBytes, accept, signal, lookup }) {
  return new Promise((resolve, reject) => {
    const mod = url.protocol === "https:" ? https : http;
    const req = mod.request(
      url,
      { method: "GET", lookup, signal, headers: { "user-agent": UA, accept, "accept-encoding": "identity" }, agent: false },
      (res) => {
        const status = res.statusCode || 0;
        if (status >= 300 && status < 400 && res.headers.location) {
          res.resume();
          return resolve({ redirect: res.headers.location, status });
        }
        const len = Number(res.headers["content-length"] || 0);
        if (len > maxBytes) {
          res.destroy();
          return resolve({ status, tooLarge: true, headers: res.headers, body: Buffer.alloc(0) });
        }
        const chunks = [];
        let n = 0;
        res.on("data", (c) => {
          n += c.length;
          if (n > maxBytes) {
            chunks.push(c.subarray(0, c.length - (n - maxBytes)));
            res.destroy();
            resolve({ status, truncated: true, headers: res.headers, body: Buffer.concat(chunks) });
            return;
          }
          chunks.push(c);
        });
        res.on("end", () => resolve({ status, headers: res.headers, body: Buffer.concat(chunks) }));
        res.on("error", reject);
      },
    );
    req.on("error", reject);
    req.end();
  });
}

/** GET `raw` with the guards above. Resolves {status, headers, body, url} or throws. */
export async function guardedGet(raw, { maxBytes = 512 * 1024, timeoutMs = 5000, accept = "text/html,*/*;q=0.5", lookup = guardedLookup } = {}) {
  const ac = new AbortController();
  const timer = setTimeout(() => ac.abort(new Error("timeout")), timeoutMs);
  try {
    let url = checkTargetUrl(raw);
    for (let hop = 0; hop <= 3; hop++) {
      if (!url) throw Object.assign(new Error("blocked url"), { code: "EBLOCKED" });
      const r = await once(url, { maxBytes, accept, signal: ac.signal, lookup });
      if (!r.redirect) return { ...r, url };
      url = checkTargetUrl(new URL(r.redirect, url).toString());
    }
    throw new Error("too many redirects");
  } finally {
    clearTimeout(timer);
  }
}
