import test from "node:test";
import assert from "node:assert/strict";
import { schnorr } from "@noble/curves/secp256k1.js";
import { bytesToHex, hexToBytes } from "@noble/hashes/utils.js";
import { checkSession, checkTargetUrl, eventId, isBlockedAddress, Lru, mintSession, parseMetadata, RateLimit, sniffImage, verifyNip98 } from "../lib.mjs";

test("blocks non-public addresses", () => {
  for (const ip of ["127.0.0.1", "10.1.2.3", "172.20.0.1", "192.168.1.1", "169.254.169.254", "100.64.0.1", "0.0.0.0", "224.0.0.1", "::1", "::", "fe80::1", "fd00::1", "::ffff:127.0.0.1", "::ffff:10.0.0.1", "64:ff9b::a00:1", "2002:a00:1::"])
    assert.equal(isBlockedAddress(ip), true, ip);
  for (const ip of ["8.8.8.8", "1.1.1.1", "2606:4700:4700::1111", "::ffff:8.8.8.8"]) assert.equal(isBlockedAddress(ip), false, ip);
  assert.equal(isBlockedAddress("nonsense"), true);
});

test("target url policy", () => {
  assert.ok(checkTargetUrl("https://example.com/a#x"));
  assert.equal(checkTargetUrl("https://example.com/a#x").hash, "");
  for (const u of ["ftp://example.com", "http://localhost/", "http://127.0.0.1/", "http://[::1]/", "https://a:b@example.com", "https://example.com:8443/", "http://foo.internal/", "http://10.0.0.1/", "javascript:alert(1)", "x".repeat(3000)])
    assert.equal(checkTargetUrl(u), null, u);
});

function signed(url, method, sk, created_at = Math.floor(Date.now() / 1000)) {
  const pubkey = bytesToHex(schnorr.getPublicKey(sk));
  const ev = { pubkey, created_at, kind: 27235, content: "", tags: [["u", url], ["method", method]] };
  ev.id = eventId(ev);
  ev.sig = bytesToHex(schnorr.sign(hexToBytes(ev.id), sk));
  return { header: `Nostr ${Buffer.from(JSON.stringify(ev)).toString("base64")}`, pubkey };
}

test("nip98 verification", () => {
  const sk = schnorr.utils.randomSecretKey();
  const { header, pubkey } = signed("https://lp.example/session", "POST", sk);
  assert.equal(verifyNip98(header, "POST", "https://lp.example/session"), pubkey);
  assert.throws(() => verifyNip98(header, "GET", "https://lp.example/session"), /mismatch/);
  assert.throws(() => verifyNip98(header, "POST", "https://other/session"), /mismatch/);
  const old = signed("https://lp.example/session", "POST", sk, Math.floor(Date.now() / 1000) - 600).header;
  assert.throws(() => verifyNip98(old, "POST", "https://lp.example/session"), /stale/);
  const ev = JSON.parse(Buffer.from(header.slice(6), "base64").toString());
  ev.tags.push(["x", "y"]);
  assert.throws(() => verifyNip98(`Nostr ${Buffer.from(JSON.stringify(ev)).toString("base64")}`, "POST", "https://lp.example/session"), /id/);
  assert.throws(() => verifyNip98("Bearer x", "POST", "u"), /missing/);
});

test("session tokens", () => {
  const s = "s".repeat(40);
  const pk = "a".repeat(64);
  const { token } = mintSession(s, pk, 60, 1000);
  assert.equal(checkSession(s, token, 1001), pk);
  assert.equal(checkSession(s, token, 2000), null);
  assert.equal(checkSession("t".repeat(40), token, 1001), null);
  assert.equal(checkSession(s, token.replace(/.$/, "A"), 1001) === pk && token.endsWith("A"), false);
});

test("metadata parsing", () => {
  const html = `<html><head><title>Fallback &amp; title</title><meta property="og:title" content="OG &quot;T&quot;"><meta name="description" content="d1"><meta property="og:image" content="/img.png"><meta property='og:site_name' content='Site'></head>`;
  const m = parseMetadata(html, "https://example.com/p");
  assert.deepEqual(m, { title: 'OG "T"', siteName: "Site", description: "d1", image: "https://example.com/img.png" });
  assert.equal(parseMetadata("<p>no title</p>", "https://e.com"), null);
  assert.equal(parseMetadata(`<title>t</title><meta property="og:image" content="http://127.0.0.1/x.png">`, "https://e.com").image, null);
  assert.equal(parseMetadata(`<title>${"x".repeat(500)}</title>`, "https://e.com").title.length, 180);
});

test("image sniffing rejects svg", () => {
  assert.equal(sniffImage(Buffer.from([0x89, 0x50, 0x4e, 0x47, 0, 0, 0, 0])), "image/png");
  assert.equal(sniffImage(Buffer.from([0xff, 0xd8, 0xff, 0])), "image/jpeg");
  assert.equal(sniffImage(Buffer.from("<svg xmlns=...>")), null);
});

test("lru + rate limit", () => {
  const l = new Lru(2, 1000);
  l.set("a", 1, 0); l.set("b", 2, 0); l.set("c", 3, 0);
  assert.equal(l.get("a", 1), undefined);
  assert.equal(l.get("c", 1), 3);
  assert.equal(l.get("c", 5000), undefined);
  const r = new RateLimit(2);
  assert.ok(r.take("k", 0) && r.take("k", 1));
  assert.equal(r.take("k", 2), false);
  assert.ok(r.take("k", 60_001));
});
