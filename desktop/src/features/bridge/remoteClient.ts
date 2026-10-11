// Web side of slice B: direct WebSocket to a desktop's loopback listener (published on the user's tailnet).
// Identity per connection and per terminal launch is a one-time challenge signed by the member key via the remote signer.
export type SignJson = (e: { kind: number; content: string; createdAt: number; tags: string[][] }) => Promise<string>;
export type WsLike = {
  send(d: string): void;
  close(): void;
  onopen: (() => void) | null;
  onmessage: ((e: { data: unknown }) => void) | null;
  onclose: (() => void) | null;
  onerror: (() => void) | null;
};
export type WsFactory = (url: string) => WsLike;

export type RemoteSession = { name: string; status: string };
export type TerminalHandle = { write(s: string): void; resize(cols: number, rows: number): void; close(): void };

export class RemoteError extends Error {
  /** unreachable = socket never opened; otherwise the desktop's error code */
  code: string;
  constructor(code: string) {
    super(code);
    this.code = code;
  }
}

/** Only wss:// to anything, or ws:// to loopback (dev/e2e). The URL comes from the authenticated heartbeat. */
export function safeRemoteUrl(u: string): boolean {
  try {
    const x = new URL(u);
    return x.protocol === "wss:" || (x.protocol === "ws:" && ["127.0.0.1", "localhost"].includes(x.hostname));
  } catch {
    return false;
  }
}

type Frame = Record<string, unknown> & { t?: string };
const AUTH_KIND = 22242;
const utf8 = (b64: string) => {
  const bin = atob(b64);
  return Uint8Array.from(bin, (c) => c.charCodeAt(0));
};
const b64 = (s: string) => {
  const bytes = new TextEncoder().encode(s);
  let bin = "";
  for (const b of bytes) bin += String.fromCharCode(b);
  return btoa(bin);
};

export class RemoteConnection {
  private ws: WsLike | null = null;
  private waiters: Array<(f: Frame) => boolean> = [];
  /** Frames that arrived before anyone was waiting for them (e.g. the challenge right after the upgrade). */
  private backlog: Frame[] = [];
  private termOut = new Map<string, { data: (b: Uint8Array) => void; closed: () => void }>();
  private ping: ReturnType<typeof setInterval> | null = null;
  ready = false;
  terminals = false;
  onClose: (() => void) | null = null;

  private info: { desktopId: string; url: string };
  private sign: SignJson;
  private makeWs: WsFactory;
  constructor(
    info: { desktopId: string; url: string },
    sign: SignJson,
    makeWs: WsFactory = (u) => new WebSocket(u) as unknown as WsLike,
  ) {
    this.info = info;
    this.sign = sign;
    this.makeWs = makeWs;
  }

  private waitFor<T extends Frame>(pred: (f: Frame) => boolean, ms = 90_000): Promise<T> {
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new RemoteError("timeout")), ms);
      const handler = (f: Frame): boolean => {
        if (f.t === "error" && !pred(f)) {
          clearTimeout(timer);
          reject(new RemoteError(String(f.code)));
          return true;
        }
        if (pred(f)) {
          clearTimeout(timer);
          resolve(f as T);
          return true;
        }
        return false;
      };
      const i = this.backlog.findIndex((f) => handler(f));
      if (i >= 0) this.backlog.splice(i, 1);
      else this.waiters.push(handler);
    });
  }

  private async signChallenge(c: Frame, purpose: string): Promise<string> {
    return this.sign({
      kind: AUTH_KIND,
      content: "",
      createdAt: Math.floor(Date.now() / 1000),
      tags: [
        ["challenge", String(c.nonce)],
        ["conn", String(c.connId)],
        ["desktop", this.info.desktopId],
        ["purpose", purpose],
      ],
    });
  }

  async connect(): Promise<void> {
    if (!safeRemoteUrl(this.info.url)) throw new RemoteError("bad_url");
    const ws = this.makeWs(this.info.url);
    this.ws = ws;
    ws.onmessage = (e) => {
      let f: Frame;
      try {
        f = JSON.parse(String(e.data));
      } catch {
        return;
      }
      if (f.t === "term.out" && typeof f.id === "string") {
        this.termOut.get(f.id)?.data(utf8(String(f.data)));
        return;
      }
      if (f.t === "term.closed" && typeof f.id === "string") {
        this.termOut.get(f.id)?.closed();
        this.termOut.delete(f.id);
        return;
      }
      const i = this.waiters.findIndex((w) => w(f));
      if (i >= 0) this.waiters.splice(i, 1);
      else if (f.t !== "pong") {
        this.backlog.push(f);
        if (this.backlog.length > 20) this.backlog.shift();
      }
    };
    const closed = () => {
      this.ready = false;
      if (this.ping) clearInterval(this.ping);
      for (const w of this.waiters.splice(0)) w({ t: "error", code: "closed" });
      for (const t of this.termOut.values()) t.closed();
      this.termOut.clear();
      this.onClose?.();
    };
    ws.onclose = closed;
    await new Promise<void>((resolve, reject) => {
      ws.onopen = () => resolve();
      ws.onerror = () => reject(new RemoteError("unreachable"));
    });
    const challenge = await this.waitFor((f) => f.t === "challenge");
    const event = await this.signChallenge(challenge, "connect");
    ws.send(JSON.stringify({ t: "auth", event }));
    const ready = await this.waitFor((f) => f.t === "ready");
    this.ready = true;
    this.terminals = ready.terminals === true;
    this.ping = setInterval(() => ws.send(JSON.stringify({ t: "ping" })), 30_000);
  }

  async listSessions(): Promise<RemoteSession[]> {
    this.ws?.send(JSON.stringify({ t: "sessions.list" }));
    const r = await this.waitFor((f) => f.t === "sessions", 15_000);
    return (Array.isArray(r.sessions) ? r.sessions : [])
      .filter((s: RemoteSession) => typeof s?.name === "string")
      .slice(0, 50);
  }

  /** Each launch asks the signer for a fresh signature (purpose terminal:<launchId>). */
  async openTerminal(
    opts: { cols: number; rows: number; session?: string },
    onData: (b: Uint8Array) => void,
    onClosed: () => void,
  ): Promise<TerminalHandle> {
    const ws = this.ws;
    if (!ws || !this.ready) throw new RemoteError("closed");
    const launchId = crypto.randomUUID();
    ws.send(JSON.stringify({ t: "term.challenge", launchId }));
    const challenge = await this.waitFor((f) => f.t === "challenge" && f.purpose === `terminal:${launchId}`);
    const event = await this.signChallenge(challenge, `terminal:${launchId}`);
    this.termOut.set(launchId, { data: onData, closed: onClosed });
    ws.send(JSON.stringify({ t: "term.open", launchId, cols: opts.cols, rows: opts.rows, session: opts.session ?? null, event }));
    try {
      await this.waitFor((f) => f.t === "term.opened" && f.id === launchId, 30_000);
    } catch (e) {
      this.termOut.delete(launchId);
      throw e;
    }
    return {
      write: (s) => ws.send(JSON.stringify({ t: "term.in", id: launchId, data: b64(s) })),
      resize: (cols, rows) => ws.send(JSON.stringify({ t: "term.resize", id: launchId, cols, rows })),
      close: () => ws.send(JSON.stringify({ t: "term.close", id: launchId })),
    };
  }

  close() {
    this.ws?.close();
  }
}
