import test from "node:test";
import assert from "node:assert/strict";
import http from "node:http";
import { schnorr } from "@noble/curves/secp256k1.js";
import { bytesToHex, hexToBytes } from "@noble/hashes/utils.js";
import { eventId } from "../lib.mjs";
import { createServer } from "../server.mjs";
import { guardedGet, guardedLookup } from "../fetcher.mjs";

const PNG = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 1, 2, 3]);
function listen(srv) {
  return new Promise((r) => srv.listen(0, "127.0.0.1", () => r(`http://127.0.0.1:${srv.address().port}`)));
}

test("fetcher refuses loopback literals, names resolving to private addresses, and redirects into them", async () => {
  const upstream = http.createServer((req, res) => res.end("<title>secret</title>"));
  const base = await listen(upstream);
  await assert.rejects(guardedGet(`${base}/`), /blocked/);
  // localhost.  (trailing dot) passes the name filter but resolves to 127.0.0.1: refused by the pinned lookup.
  await assert.rejects(guardedGet("http://localhost./"), /blocked/);
  await new Promise((r) => guardedLookup("localhost.", {}, (e) => { assert.equal(e?.code, "EBLOCKED"); r(); }));
  upstream.close();
});

test("session + preview end to end (upstream injected)", async () => {
  const sk = schnorr.utils.randomSecretKey();
  const pages = {
    "https://news.example/a": { status: 200, headers: { "content-type": "text/html" }, body: Buffer.from(`<title>Hello</title><meta property="og:image" content="https://cdn.example/i.png"><meta property="og:description" content="desc">`) },
    "https://cdn.example/i.png": { status: 200, headers: { "content-type": "image/png" }, body: PNG },
    "https://news.example/svg": { status: 200, headers: { "content-type": "text/html" }, body: Buffer.from(`<title>S</title><meta property="og:image" content="https://cdn.example/x.svg">`) },
    "https://cdn.example/x.svg": { status: 200, headers: { "content-type": "image/svg+xml" }, body: Buffer.from("<svg/>") },
  };
  let calls = 0;
  const srv = createServer(
    { sessionSecret: "k".repeat(40), publicUrl: "https://lp.example", allowedOrigins: ["https://hulabuzz.huladesk.com"], perMinute: 100 },
    { guardedGet: async (u) => { calls++; const p = pages[u]; if (!p) throw new Error("nope"); return { ...p, url: new URL(u) }; } },
  );
  const base = await listen(srv);
  const pubkey = bytesToHex(schnorr.getPublicKey(sk));
  const ev = { pubkey, created_at: Math.floor(Date.now() / 1000), kind: 27235, content: "", tags: [["u", "https://lp.example/session"], ["method", "POST"]] };
  ev.id = eventId(ev);
  ev.sig = bytesToHex(schnorr.sign(hexToBytes(ev.id), sk));

  assert.equal((await fetch(`${base}/preview?url=https://news.example/a`)).status, 401);
  const sres = await fetch(`${base}/session`, { method: "POST", headers: { authorization: `Nostr ${Buffer.from(JSON.stringify(ev)).toString("base64")}`, origin: "https://hulabuzz.huladesk.com" } });
  assert.equal(sres.status, 200);
  assert.equal(sres.headers.get("access-control-allow-origin"), "https://hulabuzz.huladesk.com");
  const { token } = await sres.json();
  const H = { authorization: `Bearer ${token}` };

  const r = await fetch(`${base}/preview?url=${encodeURIComponent("https://news.example/a")}`, { headers: H });
  assert.equal(r.status, 200);
  const m = await r.json();
  assert.equal(m.title, "Hello");
  assert.equal(m.description, "desc");
  assert.equal(m.imageFetchState, "image");
  assert.ok(m.imageDataUrl.startsWith("data:image/png;base64,"));
  const before = calls;
  assert.equal((await fetch(`${base}/preview?url=${encodeURIComponent("https://news.example/a")}`, { headers: H })).status, 200);
  assert.equal(calls, before, "cached");

  const s = await (await fetch(`${base}/preview?url=${encodeURIComponent("https://news.example/svg")}`, { headers: H })).json();
  assert.equal(s.imageFetchState, "rejected");
  assert.equal(s.imageDataUrl, null);

  assert.equal((await fetch(`${base}/preview?url=${encodeURIComponent("http://127.0.0.1:22/")}`, { headers: H })).status, 400);
  assert.equal((await fetch(`${base}/preview?url=${encodeURIComponent("https://missing.example/")}`, { headers: H })).status, 204);
  assert.equal((await fetch(`${base}/session`, { method: "POST" })).status, 401);
  srv.close();
});
