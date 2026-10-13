import assert from "node:assert/strict";
import test, { after } from "node:test";

const open = [];
after(() => open.forEach((c) => c.close()));
import { RemoteConnection, RemoteError } from "./remoteClient.ts";
import { RemoteTerminalConnection, RemoteTerminalHub } from "./remoteTerminal.ts";

const frameMsg = (sub, seq) => ({ t: "term.msg", id: "", msg: { type: "frame", payload: { subscriptionId: sub, sequence: seq, full: true, rows: [], viewport: { columns: 80, screenLines: 24, generation: 0 }, bracketedPaste: false, focusReporting: false } } });

// A fake listener speaking the emu.* protocol; the bootstrap frame arrives BEFORE term.opened, like the real one.
function fakeServer({ emu = true } = {}) {
  const sent = [];
  const ws = {
    onopen: null, onmessage: null, onclose: null, onerror: null,
    send(d) {
      const f = JSON.parse(d);
      sent.push(f);
      const reply = (o) => queueMicrotask(() => ws.onmessage({ data: JSON.stringify(o) }));
      if (f.t === "auth") reply({ t: "ready", desktopId: "dsk", terminals: true });
      if (f.t === "term.challenge") reply({ t: "challenge", nonce: "n2", connId: "c1", purpose: `terminal:${f.launchId}` });
      if (f.t === "term.open") {
        if (emu) {
          const m = frameMsg("sub-1", 0); m.id = f.launchId; reply(m);
          reply({ t: "term.opened", id: f.launchId, emu: true, scrollback: 5000, subscriptionId: "sub-1", viewport: { columns: f.cols, screenLines: f.rows, generation: 0 } });
        } else reply({ t: "term.opened", id: f.launchId });
      }
      if (f.t === "emu.resize") reply({ t: "emu.reply", id: f.id, rid: f.rid, ok: true, result: { columns: f.cols, screenLines: f.rows, generation: 1 } });
      if (f.t === "emu.attach") reply({ t: "emu.reply", id: f.id, rid: f.rid, ok: false, error: "nope" });
    },
    close() { ws.onclose?.(); },
  };
  queueMicrotask(() => { ws.onopen?.(); ws.onmessage({ data: JSON.stringify({ t: "challenge", nonce: "n1", connId: "c1", desktopId: "dsk" }) }); });
  return { ws, sent };
}
const tick = () => new Promise((r) => setTimeout(r, 5));

async function conn(opts) {
  const { ws, sent } = fakeServer(opts);
  const c = new RemoteConnection({ desktopId: "dsk", url: "ws://127.0.0.1:1/v1/ws" }, async (e) => JSON.stringify(e), () => ws);
  await c.connect();
  open.push(c);
  return { c, sent, ws };
}

test("emu launch: signs, sends emu:true, replays frames that beat term.opened, correlates rid replies", async () => {
  const { c, sent } = await conn();
  const frames = [], msgs = [];
  const t = await RemoteTerminalConnection.attach(c, { cols: 100, rows: 30, host: "herdr", session: "buzz" }, (m) => msgs.push(m), (d) => frames.push(d));
  const launch = sent.find((f) => f.t === "term.open");
  assert.equal(launch.emu, true);
  assert.equal(launch.host, "herdr");
  assert.equal(launch.session, "buzz");
  assert.equal(t.subscriptionId, "sub-1");
  assert.equal(t.viewport.columns, 100);
  assert.equal(frames.length, 1, "bootstrap frame buffered then delivered");
  await frames[0].acknowledge();
  assert.deepEqual(sent.at(-1), { t: "emu.ack", id: t.sessionId, sub: "sub-1", seq: 0 });
  const vp = await t.resize(120, 40, 1000, 800);
  assert.equal(vp.columns, 120);
  const rs = sent.find((f) => f.t === "emu.resize");
  assert.deepEqual([rs.cols, rs.rows, rs.pw, rs.ph], [120, 40, 1000, 800]);
  await t.viewportReady(vp);
  assert.equal(sent.at(-1).t, "emu.viewport_ready");
  await t.input("ls\r");
  assert.deepEqual(sent.at(-1), { t: "emu.input", id: t.sessionId, data: "ls\r" });
  await t.scroll(-3);
  assert.equal(sent.at(-1).lines, -3);
  await t.mouse({ button: "wheelUp", action: "press", column: 2, row: 3 });
  assert.deepEqual(sent.at(-1).req, { button: "wheelUp", action: "press", column: 2, row: 3 });
  await t.focus(true);
  assert.equal(sent.at(-1).focused, true);
  assert.equal(msgs.length, 0);
});

test("frames for another subscription are dropped; title/bell pass through; exit is reported once", async () => {
  const { c, ws } = await conn();
  const frames = [], msgs = [];
  const t = await RemoteTerminalConnection.attach(c, { cols: 80, rows: 24 }, (m) => msgs.push(m), (d) => frames.push(d));
  const push = (o) => ws.onmessage({ data: JSON.stringify({ id: t.sessionId, ...o }) });
  push(frameMsg("other", 1));
  push({ t: "term.msg", msg: { type: "title", payload: "vim" } });
  push({ t: "term.msg", msg: { type: "bell" } });
  push({ t: "term.msg", msg: { type: "exit" } });
  push({ t: "term.closed" });
  await tick();
  assert.equal(frames.length, 1);
  assert.deepEqual(msgs.map((m) => m.type), ["title", "bell", "exit"]);
});

test("an old desktop without the emulator is refused clearly", async () => {
  const { c, sent } = await conn({ emu: false });
  await assert.rejects(RemoteTerminalConnection.attach(c, { cols: 80, rows: 24 }, () => {}, () => {}), (e) => e instanceof RemoteError && e.code === "emu_unsupported");
  assert.ok(sent.some((f) => f.t === "term.close"));
});

test("the hub connects once, uses the desktop's host/session, forces a shell for handoffs, and reconnects by itself after a drop", async () => {
  const servers = [];
  const info = { desktopId: "dsk", url: "ws://127.0.0.1:1/v1/ws", termHost: "herdr", termSession: "buzz" };
  const hub = new RemoteTerminalHub(() => info, async (e) => JSON.stringify(e), (i, s) => {
    const srv = fakeServer();
    servers.push(srv);
    return new RemoteConnection(i, s, () => srv.ws);
  });
  const states = [];
  hub.subscribe(() => states.push(hub.getStatus().state));
  const req = { columns: 80, rows: 24, channelId: "x", channelName: "x", npub: "n", relayUrl: "r", pixelWidth: 1, pixelHeight: 1 };
  const [a, b] = await Promise.all([hub.attach(req, () => {}, () => {}), hub.attach({ ...req, remoteHost: "shell" }, () => {}, () => {})]);
  assert.equal(servers.length, 1, "one connection, one connect signature");
  const opens = servers[0].sent.filter((f) => f.t === "term.open");
  assert.deepEqual([opens[0].host, opens[0].session], ["herdr", "buzz"]);
  assert.deepEqual([opens[1].host, opens[1].session], ["shell", null]);
  assert.ok(a.sessionId !== b.sessionId);
  assert.equal(hub.getStatus().state, "connected");
  servers[0].ws.close();
  assert.equal(hub.getStatus().state, "reconnecting");
  await new Promise((r) => setTimeout(r, 800));
  assert.equal(servers.length, 2, "reconnected on its own");
  assert.equal(hub.getStatus().state, "connected");
  await hub.attach(req, () => {}, () => {});
  assert.equal(servers.length, 2, "reuses the recovered connection");
  assert.ok(states.includes("connecting"));
  hub.dispose();
});
