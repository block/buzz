// Minimal relay watcher: one WebSocket, NIP-42 AUTH with the SERVICE's own key (never a user key),
// live REQ for new events, plus one-shot lookups (profiles, channel metadata, thread roots) with caching.
import WebSocket from "ws";
import { finalizeEvent } from "nostr-tools/pure";

export class RelayWatcher {
  constructor({ url, serviceSecretKey, log = console }) {
    this.url = url;
    this.sk = serviceSecretKey;
    this.log = log;
    this.pending = new Map();
    this.onEvent = () => {};
    this.filters = [];
    this.backoff = 1000;
    this.seq = 0;
  }
  start() {
    const ws = (this.ws = new WebSocket(this.url));
    ws.on("open", () => {
      this.backoff = 1000;
      this.subscribeLive();
    });
    ws.on("message", (raw) => this.handle(raw));
    ws.on("close", () => {
      for (const p of this.pending.values()) p.resolve(p.events);
      this.pending.clear();
      setTimeout(() => this.start(), this.backoff);
      this.backoff = Math.min(this.backoff * 2, 60000);
    });
    ws.on("error", (e) => this.log.warn("relay error", e.message));
  }
  send(msg) {
    if (this.ws?.readyState === WebSocket.OPEN) this.ws.send(JSON.stringify(msg));
  }
  handle(raw) {
    let msg;
    try {
      msg = JSON.parse(raw.toString());
    } catch {
      return;
    }
    const [type, a, b] = msg;
    if (type === "AUTH" && typeof a === "string" && this.sk) {
      const ev = finalizeEvent(
        { kind: 22242, created_at: Math.floor(Date.now() / 1000), tags: [["relay", this.url], ["challenge", a]], content: "" },
        this.sk,
      );
      this.send(["AUTH", ev]);
      setTimeout(() => this.subscribeLive(), 500); // re-REQ once authenticated
    } else if (type === "EVENT") {
      const p = this.pending.get(a);
      if (p) p.events.push(b);
      else if (a === "live") this.onEvent(b);
    } else if (type === "EOSE") {
      const p = this.pending.get(a);
      if (p) {
        this.pending.delete(a);
        this.send(["CLOSE", a]);
        p.resolve(p.events);
      }
    } else if (type === "CLOSED" && this.pending.has(a)) {
      const p = this.pending.get(a);
      this.pending.delete(a);
      p.resolve(p.events);
    }
  }
  setLiveFilters(filters) {
    this.filters = filters;
    this.subscribeLive();
  }
  subscribeLive() {
    if (!this.filters.length) return;
    const since = Math.floor(Date.now() / 1000) - 5;
    this.send(["REQ", "live", ...this.filters.slice(0, 10).map((f) => ({ ...f, since }))]);
  }
  query(filters, timeoutMs = 8000) {
    return new Promise((resolve) => {
      const id = `q${++this.seq}`;
      const entry = { events: [], resolve };
      this.pending.set(id, entry);
      this.send(["REQ", id, ...filters]);
      setTimeout(() => {
        if (this.pending.delete(id)) {
          this.send(["CLOSE", id]);
          resolve(entry.events);
        }
      }, timeoutMs);
    });
  }
}
