import assert from "node:assert/strict";
import test from "node:test";
import { RemoteConnection, RemoteError, safeRemoteUrl } from "./remoteClient.ts";
import { describeRemote } from "./remoteCopy.ts";
import { TermBuffer } from "./termBuffer.ts";

test("remote url policy", () => {
  assert.equal(safeRemoteUrl("wss://mac.tailnet.ts.net:9443/v1/ws"), true);
  assert.equal(safeRemoteUrl("ws://127.0.0.1:5000/v1/ws"), true);
  for (const u of ["ws://evil.example/v1/ws", "https://x", "javascript:1", "nope"]) assert.equal(safeRemoteUrl(u), false);
});

function fakeServer({ refuse } = {}) {
  const sent = [];
  const ws = {
    onopen: null, onmessage: null, onclose: null, onerror: null,
    send(d) {
      const f = JSON.parse(d);
      sent.push(f);
      const reply = (o) => queueMicrotask(() => ws.onmessage({ data: JSON.stringify(o) }));
      if (f.t === "auth") reply(refuse ? { t: "error", code: "wrong_member" } : { t: "ready", desktopId: "dsk", terminals: true });
      if (f.t === "term.challenge") reply({ t: "challenge", nonce: "n2", connId: "c1", purpose: `terminal:${f.launchId}` });
      if (f.t === "term.open") { reply({ t: "term.opened", id: f.launchId }); reply({ t: "term.out", id: f.launchId, data: btoa("hi\r\n") }); }
      if (f.t === "sessions.list") reply({ t: "sessions", sessions: [{ name: "buzz", status: "running" }] });
    },
    close() { ws.onclose?.(); },
  };
  queueMicrotask(() => { ws.onopen?.(); ws.onmessage({ data: JSON.stringify({ t: "challenge", nonce: "n1", connId: "c1", desktopId: "dsk" }) }); });
  return { ws, sent };
}

test("connect signs a connect challenge; each terminal launch signs a fresh terminal challenge", async () => {
  const { ws, sent } = fakeServer();
  const signed = [];
  const c = new RemoteConnection({ desktopId: "dsk", url: "ws://127.0.0.1:1/v1/ws" }, async (e) => (signed.push(e), JSON.stringify(e)), () => ws);
  await c.connect();
  assert.equal(signed[0].kind, 22242);
  assert.deepEqual(signed[0].tags, [["challenge", "n1"], ["conn", "c1"], ["desktop", "dsk"], ["purpose", "connect"]]);
  assert.deepEqual(await c.listSessions(), [{ name: "buzz", status: "running" }]);
  const out = [];
  const t = await c.openTerminal({ cols: 80, rows: 24, session: "buzz" }, (b) => out.push(new TextDecoder().decode(b)), () => {});
  assert.equal(signed.length, 2);
  assert.match(signed[1].tags[3][1], /^terminal:[0-9a-f-]{36}$/);
  await new Promise((r) => setTimeout(r, 5));
  assert.deepEqual(out, ["hi\r\n"]);
  t.write("ls\n");
  assert.equal(atob(sent.at(-1).data), "ls\n");
  await c.openTerminal({ cols: 80, rows: 24 }, () => {}, () => {});
  assert.equal(signed.length, 3); // a second launch needs another signature
  assert.equal(sent.filter((f) => f.t === "term.open").length, 2);
});

test("refusals and unreachable surface as RemoteError codes", async () => {
  const { ws } = fakeServer({ refuse: true });
  const c = new RemoteConnection({ desktopId: "dsk", url: "ws://127.0.0.1:1/x" }, async () => "{}", () => ws);
  await assert.rejects(c.connect(), (e) => e instanceof RemoteError && e.code === "wrong_member");
  const bad = { onopen: null, onerror: null, onmessage: null, onclose: null, send() {}, close() {} };
  queueMicrotask(() => bad.onerror());
  await assert.rejects(new RemoteConnection({ desktopId: "d", url: "wss://x.ts.net:9443/v1/ws" }, async () => "{}", () => bad).connect(), (e) => e.code === "unreachable");
  await assert.rejects(new RemoteConnection({ desktopId: "d", url: "ws://evil.example/" }, async () => "{}").connect(), (e) => e.code === "bad_url");
});

test("error-state copy", () => {
  const on = { desktopId: "d", name: "Mac", url: "wss://x", status: "on", terminals: true };
  assert.equal(describeRemote("online", on).problem, "none");
  assert.match(describeRemote("offline", on).message, /asleep or app closed/);
  assert.match(describeRemote("online", { ...on, status: "off", reason: "Tailscale isn't running on this Mac." }).message, /Remote access isn't available on Mac.*Tailscale isn't running/);
  assert.match(describeRemote("online", on, "unreachable").message, /Connect to Tailscale on this device/);
  assert.match(describeRemote("online", on, "declined").message, /declined in your signer/);
  assert.match(describeRemote("online", on, "terminals_disabled").message, /"Allow remote terminal" is off on Mac/);
});

test("term buffer: CR/BS/erase, escapes dropped", () => {
  const b = new TermBuffer();
  b.write("\x1b[32mhello\x1b[0m world\r\nabc\b\b\x1b[K!\r\nprogress 10%\rprogress 99%\x1b]0;title\x07\r\n");
  assert.equal(b.text(), "hello world\na!\nprogress 99%\n");
  b.write("\x1b[2J");
  assert.equal(b.text(), "");
});
