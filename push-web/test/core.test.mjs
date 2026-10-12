import assert from "node:assert/strict";
import test from "node:test";
import { createHash } from "node:crypto";
import { mkdtempSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { finalizeEvent, generateSecretKey, getPublicKey } from "nostr-tools/pure";
import { verifyNip98 } from "../src/nip98.mjs";
import { Store } from "../src/store.mjs";
import { buildPayload, recipientsFor } from "../src/classify.mjs";

const sk = generateSecretKey();
const pk = getPublicKey(sk);
function header(method, url, body) {
  const ev = finalizeEvent({ kind: 27235, created_at: Math.floor(Date.now() / 1000), content: "",
    tags: [["u", url], ["method", method], ["payload", createHash("sha256").update(body).digest("hex")]] }, sk);
  return `Nostr ${Buffer.from(JSON.stringify(ev)).toString("base64")}`;
}

test("nip98: accepts valid, rejects tamper and replay", () => {
  const body = Buffer.from('{"a":1}');
  const url = "https://push.example/subscriptions";
  const h = header("PUT", url, body);
  assert.equal(verifyNip98(h, { method: "PUT", url, body }), pk);
  assert.throws(() => verifyNip98(h, { method: "PUT", url, body }), /replayed/);
  assert.throws(() => verifyNip98(header("PUT", url, body), { method: "PUT", url, body: Buffer.from("x") }), /payload/);
  assert.throws(() => verifyNip98(header("PUT", "https://evil/subscriptions", body), { method: "PUT", url, body }), /url/);
});

test("store: 0600 file, rejects non-push endpoints", () => {
  const dir = mkdtempSync(join(tmpdir(), "wp-"));
  const s = new Store(dir);
  assert.throws(() => s.upsert(pk, { endpoint: "https://169.254.169.254/x", keys: { p256dh: "a", auth: "b" } }), /unsupported/);
  s.upsert(pk, { endpoint: "https://web.push.apple.com/abc", keys: { p256dh: "a", auth: "b" } }, { dm: false });
  assert.equal(s.user(pk).prefs.dm, false);
  assert.equal(s.user(pk).prefs.mention, true);
  assert.equal(statSync(join(dir, "subscriptions.json")).mode & 0o777, 0o600);
  s.removeEndpoint("https://web.push.apple.com/abc");
  assert.equal(s.user(pk), undefined);
});

const ev = (o) => ({ id: "e1", pubkey: "author", kind: 9, created_at: 1, content: "secret text nsec1xyz", tags: [], ...o });
test("classify: mention / dm / thread_reply / approvals, never self", () => {
  const registered = new Set(["me", "author"]);
  const dm = new Map([["dmchan", new Set(["me", "author"])]]);
  assert.deepEqual(recipientsFor(ev({ tags: [["h", "c"], ["p", "me"]] }), { registered, dmChannelMembers: dm }), [{ recipient: "me", category: "mention" }]);
  assert.deepEqual(recipientsFor(ev({ tags: [["h", "dmchan"]] }), { registered, dmChannelMembers: dm }), [{ recipient: "me", category: "dm" }]);
  assert.deepEqual(recipientsFor(ev({ tags: [["h", "c"], ["e", "root", "", "root"]] }), { registered, dmChannelMembers: dm, rootAuthor: "me" }), [{ recipient: "me", category: "thread_reply" }]);
  assert.deepEqual(recipientsFor(ev({ kind: 46010, tags: [["p", "me"]] }), { registered, dmChannelMembers: dm }), [{ recipient: "me", category: "needs_action" }]);
  assert.deepEqual(recipientsFor(ev({ tags: [["p", "author"]] }), { registered, dmChannelMembers: dm }), []);
});

test("payload never contains message content", () => {
  const p = JSON.stringify(buildPayload(ev({ tags: [["h", "c"]] }), "dm", { senderName: "Ann", channelName: "" }));
  assert.ok(!p.includes("secret text") && !p.includes("nsec1"));
  assert.match(p, /New message from Ann/);
});
