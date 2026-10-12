import assert from "node:assert/strict";
import test from "node:test";
import { hostsFrom, probeHost, transportFor } from "./remoteHosts.ts";

const hb = (remote) => ({ heartbeat: { bridge: "v1", agentsRunning: 0, uptimeSec: 1, caps: [], remote }, lastHeardTs: 1000 });
const R = { desktopId: "d1", name: "Mac", url: "wss://mac.ts.net:9443/v1/ws", status: "on", terminals: true, agents: true };

test("hosts: features come from what the desktop advertises; stale hosts are flagged, not dropped", () => {
  const [h] = hostsFrom(hb(R), 1100);
  assert.deepEqual(h.supports, { agents: true, terminal: true });
  assert.equal(h.stale, false);
  assert.equal(hostsFrom(hb(R), 1000 + 90_000)[0].stale, true);
  assert.deepEqual(hostsFrom(hb({ ...R, agents: false, terminals: false }), 1100)[0].supports, { agents: false, terminal: false });
  assert.deepEqual(hostsFrom({ heartbeat: null, lastHeardTs: null }), []);
});

test("transport: direct when reachable, relay fallback for agents only, terminals never fall back", () => {
  const [h] = hostsFrom(hb(R), 1100);
  assert.equal(transportFor("agents", h, "reachable"), "direct");
  assert.equal(transportFor("agents", h, "unreachable"), "relay");
  assert.equal(transportFor("terminal", h, "reachable"), "direct");
  assert.equal(transportFor("terminal", h, "unreachable"), "unavailable");
  const [relayOnly] = hostsFrom(hb({ ...R, status: "off", url: "" }), 1100);
  assert.equal(transportFor("agents", relayOnly, "unknown"), "relay");
  assert.equal(transportFor("terminal", relayOnly, "unknown"), "unavailable");
});

test("probe: true on open, false on error/timeout", async () => {
  const mk = (behave) => () => {
    const w = { close() {}, onopen: null, onerror: null, onclose: null };
    setTimeout(() => behave(w), 5);
    return w;
  };
  assert.equal(await probeHost("x", 200, mk((w) => w.onopen())), true);
  assert.equal(await probeHost("x", 200, mk((w) => w.onerror())), false);
  assert.equal(await probeHost("x", 50, mk(() => {})), false);
});

test("setWebRemotePrefs with unchanged values doesn't notify (the keep-honest effect can't spin), shared reachability notifies once per change", async () => {
  const m = await import("./remoteHosts.ts");
  let n = 0;
  const off = m.subscribeWebRemotePrefs(() => n++);
  m.setWebRemotePrefs({ terminalOn: true });
  m.setWebRemotePrefs({ terminalOn: true });
  assert.equal(n, 1);
  m.setWebRemotePrefs({ terminalOn: false });
  assert.equal(n, 2);
  off();
  let r = 0;
  const offR = m.subscribeReach(() => r++);
  m.updateReach("a|u", () => "reachable");
  m.updateReach("a|u", () => "reachable");
  assert.equal(r, 1);
  assert.equal(m.getReachMap()["a|u"], "reachable");
  offR();
});
