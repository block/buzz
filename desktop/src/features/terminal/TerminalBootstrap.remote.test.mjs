// Web: the same TerminalBootstrap/TerminalSubstrate drive a session that lives on the member's desktop (remote listener).
import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";
import { openTerminalPanel, setTerminalPanelMode } from "./terminalPanelStore.ts";

const dom = new JSDOM("<!doctype html><html><body></body></html>", { pretendToBeVisual: true, url: "http://localhost" });
const servers = [];

function fakeServer() {
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
        const sub = `sub-${sent.filter((x) => x.t === "term.open").length}`;
        reply({ t: "term.msg", id: f.launchId, msg: { type: "frame", payload: { subscriptionId: sub, sequence: 3, full: true, rows: [], cursor: { column: 0, line: 0, visible: true }, bracketedPaste: false, focusReporting: false, viewport: { columns: f.cols, screenLines: f.rows, generation: 0 } } } });
        reply({ t: "term.opened", id: f.launchId, emu: true, subscriptionId: sub, viewport: { columns: f.cols, screenLines: f.rows, generation: 0 } });
      }
      if (f.t === "emu.resize") reply({ t: "emu.reply", id: f.id, rid: f.rid, ok: true, result: { columns: f.cols, screenLines: f.rows, generation: 1 } });
    },
    close() { ws.onclose?.(); },
  };
  queueMicrotask(() => { ws.onopen?.(); ws.onmessage({ data: JSON.stringify({ t: "challenge", nonce: "n1", connId: "c1", desktopId: "dsk" }) }); });
  const srv = { ws, sent };
  servers.push(srv);
  return srv;
}

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    HTMLCanvasElement: dom.window.HTMLCanvasElement,
    KeyboardEvent: dom.window.KeyboardEvent,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
  delete globalThis.isTauri;
  dom.window.localStorage.setItem("buzz-follow-system", "false");
  // the local (web) Buzz Term settings must NOT matter: even "herdr" + a shell + a cwd here is ignored
  dom.window.localStorage.setItem("buzz.term.sessionHost", "herdr");
  dom.window.localStorage.setItem("buzz.term.shell", "/bin/zsh");
  dom.window.matchMedia = () => ({ matches: true, addEventListener() {}, removeEventListener() {} });
  dom.window.ResizeObserver = class { observe() {} disconnect() {} };
  dom.window.HTMLElement.prototype.animate = () => ({ cancel() {}, currentTime: 0, finished: new Promise(() => {}), play() {}, playbackRate: 1, reverse() {} });
  dom.window.HTMLCanvasElement.prototype.getBoundingClientRect = () => ({ bottom: 408, height: 408, left: 0, right: 840, top: 0, width: 840, x: 0, y: 0, toJSON() {} });
  dom.window.HTMLCanvasElement.prototype.getContext = () => ({ clearRect() {}, fillRect() {}, fillStyle: "", fillText() {}, font: "", restore() {}, save() {}, setTransform() {}, textBaseline: "" });
});
after(() => dom.window.close());
afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
  setTerminalPanelMode("closed");
  const { setTerminalBackend } = await import("./terminalClient.ts");
  setTerminalBackend(null);
});

async function setup(info) {
  const { createElement } = await import("react");
  const { render } = await import("@testing-library/react");
  const { ThemeProvider } = await import("@/shared/theme/ThemeProvider");
  const { TerminalBootstrap } = await import("./TerminalBootstrap.tsx");
  const { setTerminalBackend } = await import("./terminalClient.ts");
  const { RemoteTerminalHub } = await import("../bridge/remoteTerminal.ts");
  const { RemoteConnection } = await import("../bridge/remoteClient.ts");
  const hub = new RemoteTerminalHub(() => info, async (e) => JSON.stringify(e), (i, s) => new RemoteConnection(i, s, () => fakeServer().ws));
  setTerminalBackend(hub);
  const view = render(createElement(ThemeProvider, null, createElement(TerminalBootstrap, { channelId: null, channelName: null, threadId: null, npub: null, relayUrl: null })));
  return { hub, view };
}

test("web: opening Buzz term starts a desktop-hosted session using the desktop's host/session, ignoring this device's Term settings, and acks frames", async () => {
  const { waitFor, act } = await import("@testing-library/react");
  const { hub } = await setup({ desktopId: "dsk", url: "ws://127.0.0.1:1/v1/ws", termHost: "herdr", termSession: "work" });
  await act(async () => openTerminalPanel("maximized", "all"));
  await waitFor(() => assert.ok(servers[0]?.sent.some((f) => f.t === "term.open")));
  const open = servers[0].sent.find((f) => f.t === "term.open");
  assert.equal(open.emu, true);
  assert.equal(open.host, "herdr");
  assert.equal(open.session, "work");
  assert.equal(open.shell, undefined);
  assert.equal(open.cwd, undefined);
  await waitFor(() => assert.ok(servers[0].sent.some((f) => f.t === "emu.ack" && f.seq === 3)));
  assert.equal(servers[0].sent.filter((f) => f.t === "term.open").length, 1, "one session, not one per render");
  hub.dispose();
});

test("web: a failed launch closes the panel and reports through the backend instead of switching Term off", async () => {
  const { waitFor, act } = await import("@testing-library/react");
  const { hub } = await setup(undefined); // no desktop advertised
  const { getTerminalPanelSnapshotForTests } = await import("./terminalPanelStore.ts");
  await act(async () => openTerminalPanel("maximized", "all"));
  await waitFor(() => assert.equal(hub.getStatus().code, "no_desktop"));
  await waitFor(() => assert.equal(getTerminalPanelSnapshotForTests().mode, "closed"));
});

test("web: a Pass-thread-context handoff opens a plain-shell session on the desktop and types the launch line into it", async () => {
  const { waitFor, act } = await import("@testing-library/react");
  const { requestTermSessionOpen } = await import("../term-session/lib/openRequestStore.ts");
  const { hub } = await setup({ desktopId: "dsk", url: "ws://127.0.0.1:1/v1/ws", termHost: "herdr", termSession: "work" });
  await act(async () =>
    requestTermSessionOpen({ sid: "s1", channelId: "__buzz_term__", channelName: "Buzz Term", threadId: null, tool: "claude", promptPath: "", launchCommand: "claude < /tmp/p.md" }),
  );
  await waitFor(() => assert.ok(servers.at(-1)?.sent.some((f) => f.t === "term.open")));
  const open = servers.at(-1).sent.find((f) => f.t === "term.open");
  assert.equal(open.host, "shell");
  await waitFor(() => assert.ok(servers.at(-1).sent.some((f) => f.t === "emu.input" && f.data === "claude < /tmp/p.md\n")), { timeout: 3000 });
  hub.dispose();
});
