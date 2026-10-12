import assert from "node:assert/strict";
import test from "node:test";
import { DirectAgents, directErrorCopy, isDirectDown } from "./directAgents.ts";
import { parseMgmtError, RemoteError } from "./remoteClient.ts";

function server({ ttl = 3600, agents = true } = {}) {
  const sent = [];
  let opened = 0;
  const make = () => {
    opened++;
    const ws = {
      onopen: null, onmessage: null, onclose: null, onerror: null,
      send(d) {
        const f = JSON.parse(d);
        sent.push(f);
        const reply = (o) => queueMicrotask(() => ws.onmessage({ data: JSON.stringify(o) }));
        if (f.t === "auth") reply({ t: "ready", desktopId: "dsk", terminals: false, agents, sessionTtlS: ttl });
        if (f.t === "mgmt") {
          if (f.op === "agents.list") reply({ t: "mgmt.result", id: f.id, ok: true, result: { agents: [{ pubkey: "a".repeat(64), name: "Ann", status: "stopped" }] } });
          else if (f.op === "agent.create") reply({ t: "mgmt.result", id: f.id, ok: false, error: "bad_request: Name is required." });
          else if (f.op === "agent.off") reply({ t: "mgmt.result", id: f.id, ok: false, error: "agents_disabled" });
          else reply({ t: "mgmt.result", id: f.id, ok: true, result: {} });
        }
      },
      close() { ws.onclose?.(); },
    };
    queueMicrotask(() => { ws.onopen?.(); ws.onmessage({ data: JSON.stringify({ t: "challenge", nonce: "n" + opened, connId: "c" + opened, desktopId: "dsk" }) }); });
    return ws;
  };
  return { make, sent, opens: () => opened };
}

test("one signature covers many requests; list returns every agent", async () => {
  const s = server();
  let signs = 0;
  const d = new DirectAgents({ desktopId: "dsk", url: "ws://127.0.0.1:1/x" }, async () => (signs++, "{}"), s.make);
  assert.equal(d.live, false);
  const list = await d.list();
  await d.start("a".repeat(64));
  await d.stop("a".repeat(64));
  assert.equal(list[0].name, "Ann");
  assert.equal(signs, 1, "the signer is asked once per connection, not per action");
  assert.equal(d.live, true);
  assert.equal(d.state, "connected");
  assert.equal(s.sent.filter((f) => f.t === "mgmt").length, 3);
});

test("the signature lifetime ends the session and the next call signs again", async () => {
  const s = server({ ttl: 60 });
  let t = Date.now();
  let signs = 0;
  const d = new DirectAgents({ desktopId: "dsk", url: "ws://127.0.0.1:1/x" }, async () => (signs++, "{}"), s.make, () => t);
  await d.list();
  assert.equal(d.live, true);
  assert.ok(d.expiresAt >= t + 60_000 && d.expiresAt < t + 62_000);
  const exp = d.expiresAt;
  t = exp + 1000;
  assert.equal(d.live, false);
  await d.list();
  assert.equal(signs, 2);
});

test("desktop errors keep their code and message; down codes are recognised for relay fallback", async () => {
  const s = server();
  const d = new DirectAgents({ desktopId: "dsk", url: "ws://127.0.0.1:1/x" }, async () => "{}", s.make);
  await assert.rejects(d.create({ name: "" }), (e) => e.code === "bad_request" && e.detail === "Name is required.");
  await assert.rejects(d.call("agent.off"), (e) => e.code === "agents_disabled");
  assert.match(directErrorCopy(new RemoteError("agents_disabled")), /Allow remote agent management/);
  assert.equal(directErrorCopy(parseMgmtError("bad_request: Name is required.")), "Name is required.");
  assert.equal(isDirectDown(new RemoteError("unreachable")), true);
  assert.equal(isDirectDown(new RemoteError("bad_request")), false);
});

test("unreachable desktop fails the connect with a down code (caller falls back to the relay)", async () => {
  const bad = () => {
    const ws = { onopen: null, onerror: null, onmessage: null, onclose: null, send() {}, close() {} };
    queueMicrotask(() => ws.onerror());
    return ws;
  };
  const d = new DirectAgents({ desktopId: "dsk", url: "wss://x.ts.net:9443/v1/ws" }, async () => "{}", bad);
  await assert.rejects(d.list(), (e) => isDirectDown(e));
  assert.equal(d.state, "failed");
  assert.equal(d.live, false);
});

test("a signer decline is reported as declined", async () => {
  const s = server();
  const d = new DirectAgents({ desktopId: "dsk", url: "ws://127.0.0.1:1/x" }, async () => { throw new Error("User rejected"); }, s.make);
  await assert.rejects(d.list(), (e) => e.code === "declined");
});

test("autoConnect signs once, reuses a live connection, and tries only once per page visit", async () => {
  const s = server();
  let signs = 0;
  const d = new DirectAgents({ desktopId: "dsk", url: "ws://127.0.0.1:1/x" }, async () => (signs++, "{}"), s.make);
  await d.autoConnect();
  await d.autoConnect();
  assert.equal(d.live, true);
  assert.equal(signs, 1);
});

test("autoConnect after a decline does not retry on its own; a new visit (resetAuto) or an explicit connect does", async () => {
  const s = server();
  let signs = 0;
  let decline = true;
  const d = new DirectAgents({ desktopId: "dsk", url: "ws://127.0.0.1:1/x" }, async () => { signs++; if (decline) throw new Error("User rejected"); return "{}"; }, s.make);
  await d.autoConnect(); // resolves, never throws
  assert.equal(d.state, "failed");
  assert.equal(d.live, false);
  await d.autoConnect();
  await d.autoConnect();
  assert.equal(signs, 1, "no retry loop");
  decline = false;
  d.resetAuto(); // the page was closed and reopened
  await d.autoConnect();
  assert.equal(d.live, true);
  assert.equal(signs, 2);
});

test("after the signature expires nothing reconnects until the next page visit", async () => {
  const s = server({ ttl: 60 });
  let t = Date.now();
  let signs = 0;
  const d = new DirectAgents({ desktopId: "dsk", url: "ws://127.0.0.1:1/x" }, async () => (signs++, "{}"), s.make, () => t);
  await d.autoConnect();
  t = d.expiresAt + 1000;
  await d.autoConnect();
  assert.equal(signs, 1, "expiry alone never pops the signer");
  d.resetAuto();
  await d.autoConnect();
  assert.equal(signs, 2);
});
