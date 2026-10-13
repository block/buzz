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

/** Tab order survives a refresh: launch ids → creation sequence (the desktop's term.list order is arbitrary). */
const TAB_ORDER_KEY = "buzz.webterm.tabOrder";
function readTabOrder(): Record<string, number> {
  try {
    const v = JSON.parse(globalThis.localStorage?.getItem(TAB_ORDER_KEY) ?? "{}");
    return v && typeof v === "object" ? v : {};
  } catch {
    return {};
  }
}
function noteTabCreated(id: string) {
  try {
    const o = readTabOrder();
    if (o[id] !== undefined) return;
    // keep it small: only the last 32 launches matter (the desktop caps live terminals at 3)
    const entries = Object.entries(o).sort((a, b) => a[1] - b[1]).slice(-31);
    const next = Object.fromEntries(entries);
    next[id] = Date.now();
    globalThis.localStorage?.setItem(TAB_ORDER_KEY, JSON.stringify(next));
  } catch {
    // storage unavailable: order falls back to the desktop's
  }
}

export type RemoteSession = { name: string; status: string };
/** A desktop terminal kept alive across connections (term.list). */
export type LiveTerminal = { id: string; emu: boolean; title?: string | null; attached: boolean; mine?: boolean };
export type TerminalHandle = {
  write(s: string): void;
  resize(cols: number, rows: number): void;
  close(): void;
  /** Lines of history to keep, from the desktop's Settings → Buzz Term → Scrollback (plain shells only). */
  scrollback?: number;
};

/** One server-side-emulated terminal (Plan A): structured frames come in as `term.msg`, ops go out as `emu.*`. */
export type EmuMessage = { type: string; payload?: unknown };
export type EmuHandle = {
  id: string;
  subscriptionId: string;
  /** Viewport of the bootstrap snapshot, exactly as the desktop's renderer expects it. */
  viewport: Record<string, unknown>;
  scrollback?: number;
  /** Title the program last set (reattached terminals). */
  title?: string;
  /** Fire-and-forget op (input, ack, scroll, mouse, focus, viewport_ready, detach). */
  fire(op: string, fields?: Record<string, unknown>): void;
  /** Op with a correlated `emu.reply` (resize, attach). */
  request<T = unknown>(op: string, fields?: Record<string, unknown>, ms?: number): Promise<T>;
  close(): void;
};

export class RemoteError extends Error {
  /** unreachable = socket never opened; otherwise the desktop's error code */
  code: string;
  /** Human-readable detail from the desktop, when it sent one. */
  detail?: string;
  constructor(code: string, detail?: string) {
    super(detail ? `${code}: ${detail}` : code);
    this.code = code;
    this.detail = detail;
  }
}

/** The desktop sends management errors as "<code>: <message>" (or just a code such as agents_disabled / timeout). */
export function parseMgmtError(e: string): RemoteError {
  const i = e.indexOf(": ");
  return i > 0 && /^[a-z_]+$/.test(e.slice(0, i)) ? new RemoteError(e.slice(0, i), e.slice(i + 2)) : new RemoteError(e);
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
  /** Emulated terminals by launch id. `on` is null until `term.opened`; frames that arrive earlier are buffered in `pre`. */
  private emus = new Map<string, { on: ((m: EmuMessage) => void) | null; pre: EmuMessage[] }>();
  private emuPending = new Map<string, { resolve: (v: unknown) => void; reject: (e: Error) => void; timer: ReturnType<typeof setTimeout> }>();
  ready = false;
  terminals = false;
  /** "Allow remote agent management" was on when this connection was signed (it is re-checked on every request). */
  agents = false;
  /** One connect signature covers the whole connection; it ends at `expiresAt` (ms) and the web signs again. */
  sessionTtlS = 0;
  expiresAt = 0;
  /** End (ms) of this connection's signed terminal window: launches before it skip the signer. 0 = none yet. */
  termSignedUntil = 0;
  termSessionTtlS = 0;
  onClose: (() => void) | null = null;
  /** Last time any frame arrived (ms). iOS can freeze a socket without closing it; silence is how we notice. */
  private lastRx = 0;
  private probeTimer: ReturnType<typeof setTimeout> | null = null;
  private closedOnce = false;
  private fireClosed: (() => void) | null = null;

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

  /** Diagnostics: one ping/pong round trip on this socket, in ms. */
  async pingRtt(): Promise<number> {
    const ws = this.ws;
    if (!this.ready || !ws) throw new RemoteError("closed");
    const t = performance.now();
    const pong = this.waitFor((f) => f.t === "pong", 5000);
    ws.send(JSON.stringify({ t: "ping" }));
    await pong;
    return performance.now() - t;
  }

  /** Diagnostics: what kind of path the socket takes (no hostnames). */
  transportLabel(): string {
    try {
      const u = new URL(this.info.url);
      const h = u.hostname;
      const kind = h.endsWith(".ts.net") || /^100\.(6[4-9]|[7-9]\d|1[01]\d|12[0-7])\./.test(h)
        ? "Tailscale direct"
        : /^(10|192\.168|172\.(1[6-9]|2\d|3[01]))\./.test(h) || h.endsWith(".local") ? "LAN direct" : "internet (proxy/relay or public host)";
      return `${u.protocol.replace(":", "")} · ${kind}`;
    } catch {
      return "unknown";
    }
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
      this.lastRx = Date.now();
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
      if (f.t === "term.msg" && typeof f.id === "string") {
        const e = this.emus.get(f.id);
        const m = f.msg as EmuMessage | undefined;
        if (e && m && typeof m.type === "string") {
          if (e.on) e.on(m);
          else if (e.pre.length < 64) e.pre.push(m);
        }
        return;
      }
      if (f.t === "emu.reply") {
        const p = typeof f.rid === "string" ? this.emuPending.get(f.rid) : undefined;
        if (p && typeof f.rid === "string") {
          clearTimeout(p.timer);
          this.emuPending.delete(f.rid);
          if (f.ok === true) p.resolve(f.result);
          else p.reject(new RemoteError("terminal", typeof f.error === "string" ? f.error : undefined));
        }
        return;
      }
      if (f.t === "term.closed" && typeof f.id === "string") {
        this.emus.delete(f.id);
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
      if (this.closedOnce) return;
      this.closedOnce = true;
      if (this.probeTimer) clearTimeout(this.probeTimer);
      this.ready = false;
      if (this.ping) clearInterval(this.ping);
      for (const w of this.waiters.splice(0)) w({ t: "error", code: "closed" });
      for (const t of this.termOut.values()) t.closed();
      this.termOut.clear();
      this.emus.clear();
      for (const [rid, p] of this.emuPending) {
        clearTimeout(p.timer);
        p.reject(new RemoteError("closed"));
        this.emuPending.delete(rid);
      }
      this.onClose?.();
    };
    ws.onclose = closed;
    this.fireClosed = closed;
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
    this.agents = ready.agents === true;
    this.sessionTtlS = typeof ready.sessionTtlS === "number" ? ready.sessionTtlS : 0;
    this.expiresAt = this.sessionTtlS ? Date.now() + this.sessionTtlS * 1000 : 0;
    this.termSessionTtlS = typeof ready.termSessionTtlS === "number" ? ready.termSessionTtlS : 0;
    this.termSignedUntil = 0;
    this.lastRx = Date.now();
    // Heartbeat: ping every 15 s; a connection that stays silent for 40 s is dead (frozen by iOS, dropped by the network).
    this.ping = setInterval(() => {
      if (Date.now() - this.lastRx > 40_000) return this.kill();
      try {
        ws.send(JSON.stringify({ t: "ping" }));
      } catch {
        this.kill();
      }
    }, 15_000);
  }

  async listSessions(): Promise<RemoteSession[]> {
    this.ws?.send(JSON.stringify({ t: "sessions.list" }));
    const r = await this.waitFor((f) => f.t === "sessions", 15_000);
    return (Array.isArray(r.sessions) ? r.sessions : [])
      .filter((s: RemoteSession) => typeof s?.name === "string")
      .slice(0, 50);
  }

  /** One agent-management request over the direct connection (the desktop checks the switch on every request). */
  async mgmt<T = Record<string, unknown>>(op: string, params: Record<string, unknown> = {}, ms = 120_000): Promise<T> {
    const ws = this.ws;
    if (!ws || !this.ready) throw new RemoteError("closed");
    const id = crypto.randomUUID();
    ws.send(JSON.stringify({ t: "mgmt", id, op, params }));
    const r = await this.waitFor((f) => f.t === "mgmt.result" && f.id === id, ms);
    if (r.ok !== true) throw parseMgmtError(String(r.error ?? "failed"));
    return (r.result ?? {}) as T;
  }

  /**
   * Sends term.open and waits for term.opened. Inside the connection's signed terminal window it goes without a signature;
   * otherwise (or if the desktop says the window ended, or it's an older desktop) it signs `terminal:<launchId>` first.
   */
  /** Launches/attaches go one at a time: the first signs, the rest ride its signed window (one signer prompt, not three). */
  private authChain: Promise<unknown> = Promise.resolve();
  private openWithAuth(ws: WsLike, launchId: string, body: Record<string, unknown>, attach = false): Promise<Frame> {
    const run = this.authChain.then(() => this.openWithAuthNow(ws, launchId, body, attach));
    this.authChain = run.catch(() => {});
    return run;
  }

  /** Close desktop terminals left from an earlier page (take each over, then close it). Returns how many closed. */
  async closeTerminals(ids: string[]): Promise<number> {
    const ws = this.ws;
    if (!ws || !this.ready) throw new RemoteError("closed");
    let n = 0;
    for (const id of ids) {
      try {
        await this.openWithAuth(ws, id, {}, true);
        ws.send(JSON.stringify({ t: "term.close", id }));
        n++;
      } catch {
        // already gone, or not ours
      }
    }
    return n;
  }

  private async openWithAuthNow(ws: WsLike, launchId: string, body: Record<string, unknown>, attach = false): Promise<Frame> {
    const go = async (event?: string) => {
      ws.send(JSON.stringify(attach ? { t: "term.attach", id: launchId, ...(event ? { event } : {}) } : { t: "term.open", launchId, ...body, ...(event ? { event } : {}) }));
      const want = attach ? "term.attached" : "term.opened";
      const opened = await this.waitFor((f) => f.t === want && f.id === launchId, 30_000);
      if (typeof opened.termSignedUntil === "number" && opened.termSignedUntil > 0) this.termSignedUntil = opened.termSignedUntil * 1000;
      return opened;
    };
    if (this.termSignedUntil > Date.now() + 5_000) {
      try {
        return await go();
      } catch (e) {
        // window over (or an older desktop that needs a signature every launch): sign this launch instead
        if (!(e instanceof RemoteError) || (e.code !== "term_signature_required" && e.code !== "bad_event")) throw e;
        this.termSignedUntil = 0;
      }
    }
    ws.send(JSON.stringify({ t: "term.challenge", launchId }));
    const challenge = await this.waitFor((f) => f.t === "challenge" && f.purpose === `terminal:${launchId}`);
    return go(await this.signChallenge(challenge, `terminal:${launchId}`));
  }

  /** Signs a terminal launch, which starts a signed window (the desktop's terminal lifetime) for later launches. */
  async openTerminal(
    opts: { cols: number; rows: number; session?: string; host?: "shell" | "herdr" },
    onData: (b: Uint8Array) => void,
    onClosed: () => void,
  ): Promise<TerminalHandle> {
    const ws = this.ws;
    if (!ws || !this.ready) throw new RemoteError("closed");
    const launchId = crypto.randomUUID();
    this.termOut.set(launchId, { data: onData, closed: onClosed });
    let scrollback: number | undefined;
    try {
      const opened = await this.openWithAuth(ws, launchId, { cols: opts.cols, rows: opts.rows, session: opts.session ?? null, host: opts.host ?? null });
      scrollback = typeof opened.scrollback === "number" ? opened.scrollback : undefined;
    } catch (e) {
      this.termOut.delete(launchId);
      throw e;
    }
    return {
      scrollback,
      write: (s) => ws.send(JSON.stringify({ t: "term.in", id: launchId, data: b64(s) })),
      resize: (cols, rows) => ws.send(JSON.stringify({ t: "term.resize", id: launchId, cols, rows })),
      close: () => ws.send(JSON.stringify({ t: "term.close", id: launchId })),
    };
  }

  /**
   * Open a terminal whose emulator runs on the desktop (the same one Buzz Term uses). Launches inside the signed terminal window skip the
   * signer; otherwise it signs `terminal:<launchId>`. Frames that beat `term.opened` (the bootstrap snapshot usually does) are buffered
   * and replayed to `onMsg` right after, in order.
   */
  /** Desktop terminals that outlived an earlier connection (page refresh / dropped link), for reattaching as tabs. */
  async listTerminals(): Promise<LiveTerminal[]> {
    const ws = this.ws;
    if (!ws || !this.ready) throw new RemoteError("closed");
    ws.send(JSON.stringify({ t: "term.list" }));
    const r = await this.waitFor((f) => f.t === "term.list", 15_000);
    return (Array.isArray(r.sessions) ? r.sessions : [])
      .filter((s: LiveTerminal) => typeof s?.id === "string" && s.emu === true)
      .slice(0, 16)
      .sort((a: LiveTerminal, b: LiveTerminal) => {
        const o = readTabOrder();
        return (o[a.id] ?? Number.MAX_SAFE_INTEGER) - (o[b.id] ?? Number.MAX_SAFE_INTEGER);
      });
  }

  async openEmuTerminal(
    opts: { cols: number; rows: number; session?: string; host?: "shell" | "herdr"; reattachId?: string },
    onMsg: (m: EmuMessage) => void,
    onClosed: () => void,
  ): Promise<EmuHandle> {
    const ws = this.ws;
    if (!ws || !this.ready) throw new RemoteError("closed");
    const launchId = opts.reattachId ?? crypto.randomUUID();
    if (!opts.reattachId) noteTabCreated(launchId);
    const entry = { on: null as ((m: EmuMessage) => void) | null, pre: [] as EmuMessage[] };
    this.emus.set(launchId, entry);
    this.termOut.set(launchId, { data: () => {}, closed: onClosed });
    let opened: Frame;
    try {
      opened = await this.openWithAuth(ws, launchId, { cols: opts.cols, rows: opts.rows, session: opts.session ?? null, host: opts.host ?? null, emu: true }, Boolean(opts.reattachId));
    } catch (e) {
      this.emus.delete(launchId);
      this.termOut.delete(launchId);
      throw e;
    }
    if (opened.emu !== true) {
      // an older desktop without the emulator: it would stream raw bytes we can't draw
      ws.send(JSON.stringify({ t: "term.close", id: launchId }));
      this.emus.delete(launchId);
      this.termOut.delete(launchId);
      throw new RemoteError("emu_unsupported", "The desktop needs to be updated for the full Buzz Term on web.");
    }
    entry.on = onMsg;
    for (const m of entry.pre.splice(0)) onMsg(m);
    const fire = (op: string, fields: Record<string, unknown> = {}) => {
      if (this.ready) ws.send(JSON.stringify({ t: op, id: launchId, ...fields }));
    };
    return {
      id: launchId,
      subscriptionId: String(opened.subscriptionId ?? ""),
      viewport: (opened.viewport ?? {}) as Record<string, unknown>,
      scrollback: typeof opened.scrollback === "number" ? opened.scrollback : undefined,
      title: typeof opened.title === "string" ? opened.title : undefined,
      fire,
      request: <T,>(op: string, fields: Record<string, unknown> = {}, ms = 15_000) =>
        new Promise<T>((resolve, reject) => {
          if (!this.ready) return reject(new RemoteError("closed"));
          const rid = crypto.randomUUID();
          const timer = setTimeout(() => {
            this.emuPending.delete(rid);
            reject(new RemoteError("timeout"));
          }, ms);
          this.emuPending.set(rid, { resolve: resolve as (v: unknown) => void, reject, timer });
          ws.send(JSON.stringify({ t: op, id: launchId, rid, ...fields }));
        }),
      close: () => {
        if (this.ready) ws.send(JSON.stringify({ t: "emu.close", id: launchId }));
        this.emus.delete(launchId);
      },
    };
  }

  close() {
    this.ws?.close();
  }

  /** Tear down a connection that stopped answering and report it closed right away (a frozen socket may never fire onclose). */
  kill() {
    try {
      this.ws?.close();
    } catch {
      /* already closing */
    }
    this.fireClosed?.();
  }

  /** On app resume: ping and give the desktop `ms` to answer with anything, else treat the connection as dead. */
  probe(ms = 5_000) {
    const ws = this.ws;
    if (!ws || !this.ready || this.probeTimer) return;
    const sentAt = Date.now();
    try {
      ws.send(JSON.stringify({ t: "ping" }));
    } catch {
      this.kill();
      return;
    }
    this.probeTimer = setTimeout(() => {
      this.probeTimer = null;
      if (this.lastRx < sentAt) this.kill();
    }, ms);
  }
}
