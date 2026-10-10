// One-shot reads over a dedicated, NIP-42-authenticated WebSocket.
// Why: HTTP /query needs a fresh NIP-98 signature per request, i.e. one extension prompt per read (the channel list alone is ~6).
// A single AUTH per socket lets the whole read path run on one signature. Filters that use bridge-only extension fields
// (before_id, top_level, include_*, consistency, ...) or ephemeral kinds still go through HTTP /query (see queryRelay).
import { getSigner } from "./signer";
import type { Filter, NostrEvent } from "./nostrTypes";
import { relayWsUrl } from "./relayConfig";

const BRIDGE_ONLY_KEYS = [
  "before_id",
  "top_level",
  "include_summaries",
  "include_aux",
  "consistency",
  "depth_limit",
  "feed_types",
  "thread_window",
];
const TIMEOUT_MS = 40_000; // connect + AUTH: remote (NIP-46) signing adds a network round trip to the AUTH step
const QUERY_TIMEOUT_MS = 20_000; // one read on an authenticated socket (then HTTP fallback in queryRelay)
const DEAD_AFTER_MS = 30_000; // a timed-out read on a socket that has been silent this long means the socket is dead (iOS froze it)
const MAX_INFLIGHT = 8;

export function canUseWs(filters: Filter[]): boolean {
  if (
    typeof localStorage !== "undefined" &&
    localStorage.getItem("buzz-web-ws-reads") === "0"
  )
    return false;
  return filters.every(
    (f) =>
      !BRIDGE_ONLY_KEYS.some((k) => k in f) &&
      !(
        Array.isArray(f.kinds) &&
        (f.kinds as number[]).some((k) => k >= 20000 && k < 30000)
      ),
  );
}

type Pending = {
  events: NostrEvent[];
  resolve: (e: NostrEvent[]) => void;
  reject: (e: Error) => void;
  timer: ReturnType<typeof setTimeout>;
};

class QuerySocket {
  private ws: WebSocket | null = null;
  private ready: Promise<void> | null = null;
  private subs = new Map<string, Pending>();
  private waiting: (() => void)[] = [];
  private inflight = 0;
  lastInbound = 0;
  constructor(private url: string) {}

  /** The one authenticated socket is reused for every read (AUTH happens once per connection). Drop it if iOS killed it in the background. */
  checkAlive() {
    if (this.ready && this.ws && this.ws.readyState !== WebSocket.OPEN && this.ws.readyState !== WebSocket.CONNECTING)
      this.teardown(new Error("relay websocket closed"));
  }

  private connect(): Promise<void> {
    if (this.ready) return this.ready;
    this.ready = new Promise<void>((resolve, reject) => {
      const ws = new WebSocket(this.url);
      this.ws = ws;
      const fail = (m: string) => {
        this.teardown(new Error(m));
        reject(new Error(m));
      };
      const timer = setTimeout(() => fail("relay auth timed out"), TIMEOUT_MS);
      let authId: string | null = null;
      ws.onerror = () => fail("relay unreachable: websocket error");
      ws.onclose = () => {
        clearTimeout(timer);
        fail("relay websocket closed");
      };
      ws.onmessage = async (m) => {
        let msg: unknown[];
        try {
          msg = JSON.parse(String(m.data));
        } catch {
          return;
        }
        const [type, a, b, c] = msg as [string, any, any, any];
        if (type === "AUTH" && authId === null) {
          try {
            const ev = await getSigner().signEvent({
              kind: 22242,
              content: "",
              tags: [
                ["relay", this.url],
                ["challenge", String(a)],
              ],
            });
            authId = ev.id;
            ws.send(JSON.stringify(["AUTH", ev]));
          } catch (e) {
            fail(e instanceof Error ? e.message : String(e));
          }
        } else if (type === "OK" && a === authId) {
          clearTimeout(timer);
          if (b === true) {
            ws.onclose = () =>
              this.teardown(new Error("relay websocket closed"));
            ws.onerror = null;
            ws.onmessage = (mm) => this.onMessage(mm);
            this.lastInbound = Date.now();
            resolve();
          } else fail(`relay auth rejected: ${c ?? ""}`);
        }
      };
    });
    this.ready.catch(() => {});
    return this.ready;
  }

  private onMessage(m: MessageEvent) {
    this.lastInbound = Date.now();
    let msg: unknown[];
    try {
      msg = JSON.parse(String(m.data));
    } catch {
      return;
    }
    const [type, id, x] = msg as [string, string, any];
    const p = this.subs.get(id);
    if (!p) return;
    if (type === "EVENT") p.events.push(x as NostrEvent);
    else if (type === "EOSE") {
      this.finish(id);
      p.resolve(p.events);
    } else if (type === "CLOSED") {
      this.finish(id, false);
      p.reject(new Error(`relay closed subscription: ${x ?? ""}`));
    }
  }

  private finish(id: string, sendClose = true) {
    const p = this.subs.get(id);
    if (!p) return;
    clearTimeout(p.timer);
    this.subs.delete(id);
    if (sendClose && this.ws?.readyState === WebSocket.OPEN)
      this.ws.send(JSON.stringify(["CLOSE", id]));
    this.inflight--;
    this.waiting.shift()?.();
  }

  private teardown(err: Error) {
    for (const [id, p] of this.subs) {
      clearTimeout(p.timer);
      p.reject(err);
      this.subs.delete(id);
    }
    this.inflight = 0;
    this.ready = null;
    try {
      this.ws?.close();
    } catch {
      /* ignore */
    }
    this.ws = null;
    for (const w of this.waiting.splice(0)) w();
  }

  async query(filters: Filter[]): Promise<NostrEvent[]> {
    this.checkAlive();
    await this.connect();
    while (this.inflight >= MAX_INFLIGHT)
      await new Promise<void>((r) => this.waiting.push(r));
    this.checkAlive();
    await this.connect();
    const id = `q-${crypto.randomUUID().slice(0, 12)}`;
    this.inflight++;
    return new Promise<NostrEvent[]>((resolve, reject) => {
      const timer = setTimeout(() => {
        const silent = Date.now() - this.lastInbound > DEAD_AFTER_MS;
        this.finish(id);
        reject(new Error("relay query timed out"));
        if (silent) this.teardown(new Error("relay socket looks dead")); // next read reconnects (one AUTH)
      }, QUERY_TIMEOUT_MS);
      this.subs.set(id, { events: [], resolve, reject, timer });
      this.ws?.send(JSON.stringify(["REQ", id, ...filters]));
    });
  }
  close() {
    this.teardown(new Error("closed"));
  }
}

let socket: QuerySocket | null = null;
let socketUrl = "";
export function wsQuery(filters: Filter[]): Promise<NostrEvent[]> {
  const url = relayWsUrl();
  if (!socket || socketUrl !== url) {
    socket?.close();
    socket = new QuerySocket(url);
    socketUrl = url;
  }
  return socket.query(filters);
}
if (typeof document !== "undefined")
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "visible") socket?.checkAlive();
  });
export function resetWsQuery() {
  socket?.close();
  socket = null;
}
