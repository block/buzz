// Browser implementation of tauri-plugin-websocket as used by relayClientSession.
//
// Liveness: iOS suspends a backgrounded PWA and its sockets can come back "open" but dead (no close event ever fires), so
// the relay session would believe it is connected and the user had to refresh. Every socket is probed with a tiny
// heartbeat REQ (answered by any NIP-01 relay with EOSE) when the app resumes and whenever it has been quiet; a socket
// that doesn't answer in time is torn down and reported closed, which hands control to the session's own reconnect
// (exponential backoff with jitter, NIP-42 re-auth and `since`-backfilled re-subscription).
import { onOffline, onResume, isPageVisible } from "@/shared/lib/resumeSignals";

type Sink = { onmessage: (m: unknown) => void };
type Entry = {
  ws: WebSocket;
  sink: Sink;
  lastRx: number;
  probe: ReturnType<typeof setTimeout> | null;
  dead: boolean;
};
const sockets = new Map<number, Entry>();
let next = 1;
let hbSeq = 0;

export const HEARTBEAT_PREFIX = "__buzz_hb_";
const IDLE_PROBE_MS = 25_000; // probe a socket that has been silent this long
const PROBE_TIMEOUT_MS = 6_000; // ...and give it this long to answer
const ZERO_ID = "0".repeat(64);

function report(id: number, e: Entry, code: number, reason: string) {
  if (e.dead) return;
  e.dead = true;
  if (e.probe) clearTimeout(e.probe);
  sockets.delete(id);
  e.sink.onmessage({ type: "Close", data: { code, reason } });
}

function kill(id: number, e: Entry, reason: string) {
  try {
    e.ws.close(4000, reason);
  } catch {
    /* already closing */
  }
  report(id, e, 4000, reason); // don't wait for onclose: a frozen socket may never deliver it
}

/** Send a heartbeat REQ; if nothing at all arrives within the timeout the socket is dead. */
export function probeSocket(id: number, timeoutMs = PROBE_TIMEOUT_MS) {
  const e = sockets.get(id);
  if (!e || e.dead || e.probe) return;
  if (e.ws.readyState !== WebSocket.OPEN) {
    if (e.ws.readyState !== WebSocket.CONNECTING)
      kill(id, e, "socket not open");
    return;
  }
  const sub = `${HEARTBEAT_PREFIX}${++hbSeq}`;
  const sentAt = Date.now();
  try {
    e.ws.send(JSON.stringify(["REQ", sub, { ids: [ZERO_ID], limit: 1 }]));
  } catch {
    kill(id, e, "heartbeat send failed");
    return;
  }
  e.probe = setTimeout(() => {
    e.probe = null;
    if (e.lastRx < sentAt) kill(id, e, "heartbeat timeout");
  }, timeoutMs);
}

export function probeAllSockets() {
  for (const id of [...sockets.keys()]) probeSocket(id);
}

let timersInstalled = false;
function installTimers() {
  if (timersInstalled || typeof window === "undefined") return;
  timersInstalled = true;
  onResume(probeAllSockets);
  onOffline(() => {
    for (const [id, e] of [...sockets]) kill(id, e, "offline");
  });
  setInterval(() => {
    if (!isPageVisible()) return;
    const now = Date.now();
    for (const [id, e] of sockets)
      if (now - e.lastRx > IDLE_PROBE_MS) probeSocket(id);
  }, 10_000);
}

/** Heartbeat replies are ours: swallow them so the session never sees an unknown subscription. */
function isHeartbeatFrame(data: string): boolean {
  if (!data.includes(HEARTBEAT_PREFIX)) return false;
  try {
    const m = JSON.parse(data);
    return (
      Array.isArray(m) &&
      typeof m[1] === "string" &&
      m[1].startsWith(HEARTBEAT_PREFIX)
    );
  } catch {
    return false;
  }
}

export function wsConnect(url: string, sink: Sink): Promise<number> {
  installTimers();
  return new Promise((resolve, reject) => {
    const id = next++;
    const ws = new WebSocket(url);
    const e: Entry = { ws, sink, lastRx: Date.now(), probe: null, dead: false };
    sockets.set(id, e);
    ws.onopen = () => {
      e.lastRx = Date.now();
      resolve(id);
    };
    ws.onerror = () => {
      reject(new Error("WebSocket connection failed"));
      report(id, e, 1006, "error");
    };
    ws.onmessage = (m) => {
      e.lastRx = Date.now();
      const data = typeof m.data === "string" ? m.data : "";
      if (isHeartbeatFrame(data)) {
        const parsed = JSON.parse(data);
        if (parsed[0] === "EOSE" && e.ws.readyState === WebSocket.OPEN)
          e.ws.send(JSON.stringify(["CLOSE", parsed[1]]));
        return;
      }
      sink.onmessage({ type: "Text", data });
    };
    ws.onclose = (c) => report(id, e, c.code, c.reason);
  });
}
export function wsSend(id: number, message: { type: string; data: string }) {
  const e = sockets.get(id);
  if (!e) throw new Error(`WebSocket ${id} not found`);
  e.ws.send(message.data);
}
export function wsClose(id: number) {
  const e = sockets.get(id);
  if (!e) return;
  if (e.probe) clearTimeout(e.probe);
  sockets.delete(id);
  e.ws.close();
}
export function wsCloseAll() {
  for (const id of [...sockets.keys()]) wsClose(id);
}
