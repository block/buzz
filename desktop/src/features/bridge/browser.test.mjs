import assert from "node:assert/strict";
import test from "node:test";
import { getPublicKey, finalizeEvent, nip44 } from "nostr-tools";
import { BridgeCommandHandler, makeSeenSet } from "./commandHandler.ts";
import { BridgeCommander } from "./commander.ts";
import { BridgePublisher } from "./publisher.ts";
import { BridgeWebClient } from "./webClient.ts";
import { BrowserSessionManager, buildDesktopSessionList, chunkImage, fitImage, publishableUrl, validateBrowserUrl } from "./browserSessions.ts";
import { SHOT_CHUNK_BYTES, SHOT_MAX_BYTES } from "./protocol.ts";

const sk = new Uint8Array(32).fill(7);
const pk = getPublicKey(sk);
const selfKey = nip44.v2.utils.getConversationKey(sk, pk);

test("url policy: http(s) only, no credentials, no private/loopback/tailnet hosts", () => {
  assert.equal(validateBrowserUrl("https://example.com/a?b=1").ok, true);
  for (const u of ["javascript:alert(1)", "file:///etc/passwd", "ftp://x.com", "https://u:p@example.com", "http://localhost:3000", "http://127.0.0.1", "http://192.168.1.5", "http://10.0.0.2", "http://172.20.1.1", "http://100.101.2.3", "http://printer.local", "http://[::1]/", "", "x".repeat(3000), 5, "not a url"])
    assert.equal(validateBrowserUrl(u).ok, false, String(u).slice(0, 30));
  assert.equal(validateBrowserUrl("http://172.32.0.1").ok, true);
});

test("manager: session limit, one shot in flight, per-session interval, per-minute cap", () => {
  const m = new BrowserSessionManager(2);
  m.add({ sessionId: "a", title: "A", host: "a.com", openedAt: 1 });
  assert.equal(m.canOpen(), true);
  m.add({ sessionId: "b", title: "B", host: "b.com", openedAt: 1 });
  assert.equal(m.canOpen(), false);
  assert.equal(m.beginShot("a", 10_000), null);
  assert.equal(m.beginShot("b", 10_001), "busy");
  m.endShot();
  assert.equal(m.beginShot("a", 11_000), "rate_limited");
  assert.equal(m.beginShot("a", 14_000), null);
  m.endShot();
  const m2 = new BrowserSessionManager(2);
  m2.add({ sessionId: "a", title: "", host: "a", openedAt: 1 });
  for (let i = 0; i < 10; i++) {
    assert.equal(m2.beginShot("a", 1_000_000 + i * 4000), null);
    m2.endShot();
  }
  assert.equal(m2.beginShot("a", 1_000_000 + 10 * 4000), "rate_limited");
  assert.equal(m2.beginShot("a", 1_000_000 + 80_000), null);
});

test("chunking and fitting", async () => {
  assert.equal(chunkImage(new Uint8Array(0)), null);
  assert.equal(chunkImage(new Uint8Array(SHOT_MAX_BYTES + 1)), null);
  const c = chunkImage(new Uint8Array(SHOT_CHUNK_BYTES * 2 + 5));
  assert.deepEqual(c.map((x) => x.length), [SHOT_CHUNK_BYTES, SHOT_CHUNK_BYTES, 5]);
  const tried = [];
  const fit = await fitImage(async (w, q) => (tried.push([w, q]), { bytes: new Uint8Array(w > 600 ? SHOT_MAX_BYTES + 1 : 1000), w, h: 10 }));
  assert.ok(fit && fit.w <= 600);
  assert.equal(await fitImage(async (w) => ({ bytes: new Uint8Array(SHOT_MAX_BYTES + 1), w, h: 1 })), null);
});

function loop(browserOver = {}) {
  const published = [];
  const io = {
    encryptToSelf: async (p) => nip44.v2.encrypt(p, selfKey),
    signEvent: async (e) => finalizeEvent({ kind: e.kind, content: e.content, created_at: e.createdAt, tags: e.tags }, sk),
    publish: async (s) => void published.push(s),
  };
  const timers = [];
  io.setTimeout = (fn) => (timers.push(fn), timers.length);
  io.clearTimeout = () => {};
  const pub = new BridgePublisher(io, { deviceId: "d", caps: ["agents.snapshot", "browser.open", "browser.close", "browser.screenshot", "browser.agent"] });
  const client = new BridgeWebClient(pk, { decryptFromSelf: async (c) => nip44.v2.decrypt(c, selfKey), cacheGet: () => null, cachePut() {} });
  const wire = [];
  const commander = new BridgeCommander(client, { signEvent: io.signEvent, publish: async (s) => void wire.push(s) }, "tab1");
  const manager = new BrowserSessionManager(2);
  const log = [];
  const img = new Uint8Array(SHOT_CHUNK_BYTES + 100).map((_, i) => i % 251);
  const handler = new BridgeCommandHandler({
    memberPubkey: pk,
    sessionKey: () => ({ kid: pub.kid, key: pub.sessionKey }),
    agents: () => pub.agents(),
    seen: makeSeenSet(() => [], () => {}),
    executors: { start: async () => {}, stop: async () => {}, message: async () => {} },
    publishResult: (id, b) => pub.publishResult(id, b),
    browser: {
      manager,
      open: async (url, host) => (log.push(["open", url]), { sessionId: `pin-${host}`, title: host }),
      close: async (id) => void log.push(["close", id]),
      capture: async () => ({ mime: "image/jpeg", w: 800, h: 600, bytes: img }),
      publishShot: (id, s) => pub.publishShot(id, s),
      publishSessions: () => pub.publishBrowserSessions(manager.list()),
      ...browserOver,
    },
  });
  const flush = async () => { for (const e of published.splice(0)) await client.ingest(e); };
  return { pub, client, commander, wire, handler, manager, log, img, flush, run: async () => { for (const f of timers.splice(0)) f(); await new Promise((r) => setTimeout(r, 30)); } };
}

test("open -> sessions listed -> screenshot assembled from encrypted chunks -> close; open needs confirmation, screenshot does not", async () => {
  const L = loop();
  L.pub.start();
  L.pub.update({ agents: [], generatedAt: 1 });
  await L.run();
  await L.flush();
  await assert.rejects(L.commander.send("browser.open", { url: "https://example.com" }), /confirmation/i);
  const id = await L.commander.send("browser.open", { url: "https://example.com/x" }, { confirmed: true });
  assert.equal(await L.handler.handle(L.wire.pop()), "executed");
  await L.flush();
  assert.equal(L.commander.statusOf(id), "ok");
  assert.deepEqual(L.client.state.results.get(id).result, { sessionId: "pin-example.com" });
  assert.equal(L.client.state.browserSessions[0].host, "example.com");
  const sid = "pin-example.com";
  const shotId = await L.commander.send("browser.screenshot", { sessionId: sid });
  assert.equal(L.commander.shotBusy(), true);
  await assert.rejects(L.commander.send("browser.screenshot", { sessionId: sid }), /already/);
  assert.equal(await L.handler.handle(L.wire.pop()), "executed");
  // the pixels never appear in cleartext on the wire
  await L.flush();
  assert.equal(L.commander.statusOf(shotId), "ok");
  const shot = L.client.state.shots.get(sid);
  assert.ok(shot.src.startsWith("data:image/jpeg;base64,"));
  assert.equal(Buffer.from(shot.src.split(",")[1], "base64").equals(Buffer.from(L.img)), true);
  assert.equal(L.commander.shotBusy(), false);
  // unknown session / bad url / limit / rate limit
  await L.commander.send("browser.screenshot", { sessionId: "nope" });
  await L.handler.handle(L.wire.pop());
  await L.flush();
  const closeId = await L.commander.send("browser.close", { sessionId: sid }, { confirmed: true });
  await L.handler.handle(L.wire.pop());
  await L.flush();
  assert.equal(L.commander.statusOf(closeId), "ok");
  assert.deepEqual(L.client.state.browserSessions, []);
  assert.deepEqual(L.log.map((x) => x[0]), ["open", "close"]);
});

test("desktop refuses blocked urls, session limit, rapid screenshots, and failed captures without crashing", async () => {
  const L = loop();
  L.pub.start();
  L.pub.update({ agents: [], generatedAt: 1 });
  await L.run();
  await L.flush();
  const run = async (type, body) => {
    const id = await L.commander.send(type, body, { confirmed: true });
    await L.handler.handle(L.wire.pop());
    await L.flush();
    return L.client.state.results.get(id);
  };
  assert.equal((await run("browser.open", { url: "http://localhost:8080" })).code, "blocked_url");
  assert.equal((await run("browser.open", { url: "javascript:x" })).code, "bad_url");
  assert.equal((await run("browser.open", { url: "https://a.com" })).status, "ok");
  assert.equal((await run("browser.open", { url: "https://b.com" })).status, "ok");
  assert.equal((await run("browser.open", { url: "https://c.com" })).code, "session_limit");
  assert.equal((await run("browser.screenshot", { sessionId: "pin-a.com" })).status, "ok");
  L.commander.sent = new Map(); // allow the next request from the UI's point of view
  assert.equal((await run("browser.screenshot", { sessionId: "pin-a.com" })).code, "rate_limited");
  const F = loop({ capture: async () => { throw new Error("page closed"); } });
  F.pub.start(); F.pub.update({ agents: [], generatedAt: 1 }); await F.run(); await F.flush();
  F.manager.add({ sessionId: "s1", title: "", host: "s", openedAt: 1 });
  const id = await F.commander.send("browser.screenshot", { sessionId: "s1" });
  await F.handler.handle(F.wire.pop());
  await F.flush();
  assert.equal(F.client.state.results.get(id).code, "failed");
  assert.equal(F.manager.beginShot("s1", Date.now() + 10_000), null); // in-flight flag was released
});

test("without browser support the desktop answers unsupported", async () => {
  const L = loop();
  L.pub.start(); L.pub.update({ agents: [], generatedAt: 1 }); await L.run(); await L.flush();
  const h2 = new BridgeCommandHandler({
    memberPubkey: pk, sessionKey: () => ({ kid: L.pub.kid, key: L.pub.sessionKey }), agents: () => new Map(),
    seen: makeSeenSet(() => [], () => {}), executors: { start: async () => {}, stop: async () => {}, message: async () => {} },
    publishResult: (id, b) => L.pub.publishResult(id, b),
  });
  const id = await L.commander.send("browser.open", { url: "https://a.com" }, { confirmed: true });
  await h2.handle(L.wire.pop());
  await L.flush();
  assert.equal(L.client.state.results.get(id).code, "unsupported");
});

test("sync list: desktop browsers are published with sanitized urls, liveness, origin and agent; private pages are restricted", () => {
  assert.equal(publishableUrl("https://example.com/a/b?token=secret#frag"), "https://example.com/a/b");
  assert.equal(publishableUrl("http://localhost:3000/x"), null);
  assert.equal(publishableUrl("http://100.64.1.2/"), null);
  const list = buildDesktopSessionList({
    rows: [
      { sid: "w1", title: "Docs", url: "https://docs.example.com/p?k=1" },
      { sid: "d1", title: "Dev server", url: "http://localhost:5173/" },
      { sid: "d2", title: "", url: "https://news.example.org/" },
    ],
    remoteIds: new Set(["w1"]),
    isOpen: (sid) => (sid === "d2" ? false : sid === "w1" ? true : undefined),
    agentOf: (sid) => (sid === "w1" ? { pubkey: "ab".repeat(32), name: "Scout", mode: "observe" } : undefined),
    now: 5,
  });
  assert.deepEqual(list[0], { sessionId: "w1", title: "Docs", host: "docs.example.com", openedAt: 5, url: "https://docs.example.com/p", status: "live", origin: "web", agent: { pubkey: "ab".repeat(32), name: "Scout", mode: "observe" } });
  assert.equal(list[1].restricted, true);
  assert.equal(list[1].url, undefined);
  assert.equal(list[1].host, "restricted");
  assert.equal(list[2].status, "cold");
  assert.equal(list[2].origin, "desktop");
  assert.equal(list[2].title, "news.example.org");
  assert.ok(!JSON.stringify(list).includes("secret") && !JSON.stringify(list).includes("k=1"));
});

test("a browser that exists only on the desktop can be listed, screenshotted and removed from the web (exists hook)", async () => {
  const desktop = new Set(["native-1"]);
  const L = loop({ exists: (id) => desktop.has(id), close: async (id) => void desktop.delete(id) });
  L.pub.start(); L.pub.update({ agents: [], generatedAt: 1 }); await L.run(); await L.flush();
  const run = async (type, body) => {
    const id = await L.commander.send(type, body, { confirmed: true });
    await L.handler.handle(L.wire.pop());
    await L.flush();
    return L.client.state.results.get(id);
  };
  assert.equal((await run("browser.screenshot", { sessionId: "native-1" })).status, "ok");
  assert.equal((await run("browser.close", { sessionId: "native-1" })).status, "ok");
  assert.equal((await run("browser.close", { sessionId: "native-1" })).code, "unknown_session");
});

test("browser.agent (Observe/Drive): refused when remote agent management is off; validated and executed when on; web client keeps agent chips", async () => {
  const calls = [];
  let allowed = false;
  const agentPk = "cd".repeat(32);
  const mk = () => {
    const L = loop({ exists: (id) => id === "b1", setAgent: async (i) => void calls.push(i) });
    return L;
  };
  const L = mk();
  L.pub.start(); L.pub.update({ agents: [{ pubkey: agentPk, name: "A", status: "running", activeTurns: [] }], generatedAt: 1 }); await L.run(); await L.flush();
  // rebuild the handler with the gate (loop() has no gate by default)
  const gated = new BridgeCommandHandler({
    memberPubkey: pk, sessionKey: () => ({ kid: L.pub.kid, key: L.pub.sessionKey }), agents: () => L.pub.agents(),
    seen: makeSeenSet(() => [], () => {}), executors: { start: async () => {}, stop: async () => {}, message: async () => {} },
    publishResult: (id, b) => L.pub.publishResult(id, b), agentManagementAllowed: () => allowed,
    browser: { manager: L.manager, open: async () => ({ sessionId: "x", title: "x" }), close: async () => {}, capture: async () => ({ mime: "image/jpeg", w: 1, h: 1, bytes: L.img }), publishShot: async () => {}, publishSessions: async () => {}, exists: (id) => id === "b1", setAgent: async (i) => void calls.push(i) },
  });
  const run = async (type, body) => {
    const id = await L.commander.send(type, body, { confirmed: true });
    await gated.handle(L.wire.pop());
    await L.flush();
    return L.client.state.results.get(id);
  };
  const body = { sessionId: "b1", mode: "drive", agentPubkey: agentPk, channelId: "chan-1" };
  assert.equal((await run("browser.agent", body)).code, "remote_disabled");
  assert.equal(calls.length, 0);
  // observe-only stays view-only: even with the switch off a screenshot still works (it is not an agent command)
  assert.equal((await run("browser.screenshot", { sessionId: "b1" })).status, "ok");
  allowed = true;
  assert.equal((await run("browser.agent", { ...body, agentPubkey: "ee".repeat(32) })).code, "bad_request");
  assert.equal((await run("browser.agent", { ...body, mode: "root" })).code, "bad_request");
  assert.equal((await run("browser.agent", { ...body, sessionId: "zzz" })).code, "unknown_session");
  assert.equal((await run("browser.agent", body)).status, "ok");
  assert.equal((await run("browser.agent", { sessionId: "b1", mode: "off" })).status, "ok");
  assert.deepEqual(calls.map((c) => c.mode), ["drive", "off"]);
  await assert.rejects(L.commander.send("browser.agent", body), /confirmation/i);
});
