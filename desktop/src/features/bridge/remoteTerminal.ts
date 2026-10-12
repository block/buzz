// Web Buzz Term, Plan A: the desktop's own emulator runs in its remote listener and pushes the same row/span/cell frames the
// Tauri build gets over its Channel. This file is the web half of `terminalClient`: a TerminalConnectionApi over the direct
// websocket (so TerminalBootstrap/TerminalSubstrate run unchanged), plus the hub that owns the (signed-once) connection.
import { termProbe } from "@/platform/web/kbDebug/termProbe";
import type {
  TerminalAttachRequest,
  TerminalBackend,
  TerminalConnectionApi,
  TerminalDelivery,
  TerminalFrameMessage,
  TerminalMessage,
  TerminalViewport,
} from "../terminal/terminalClient";
import type { RemoteInfo } from "./protocol";
import { RemoteConnection, RemoteError, type EmuHandle, type EmuMessage, type SignJson } from "./remoteClient";

type OnMessage = (message: Exclude<TerminalMessage, { type: "frame" }>) => void;
type OnFrame = (delivery: TerminalDelivery) => void;

const NON_FRAME = new Set(["title", "resetTitle", "bell", "exit"]);

export class RemoteTerminalConnection implements TerminalConnectionApi {
  readonly sessionId: string;
  readonly subscriptionId: string;
  readonly viewport: TerminalViewport;
  /** History depth the desktop applies (its Settings → Buzz Term → Scrollback). */
  readonly scrollback?: number;
  private handle: EmuHandle;
  private exited = false;

  private constructor(handle: EmuHandle) {
    this.handle = handle;
    this.sessionId = handle.id;
    this.subscriptionId = handle.subscriptionId;
    this.viewport = handle.viewport as unknown as TerminalViewport;
    this.scrollback = handle.scrollback;
  }

  static async attach(
    conn: RemoteConnection,
    opts: { cols: number; rows: number; host?: "shell" | "herdr"; session?: string; reattachId?: string },
    onMessage: OnMessage,
    onFrame: OnFrame,
  ): Promise<RemoteTerminalConnection> {
    let self: RemoteTerminalConnection | null = null;
    const pending: EmuMessage[] = [];
    const exit = () => {
      if (self) {
        if (self.exited) return;
        self.exited = true;
      }
      onMessage({ type: "exit" });
    };
    const deliver = (m: EmuMessage) => {
      if (!self) {
        pending.push(m);
        return;
      }
      if (m.type === "frame") {
        const frame = m.payload as TerminalFrameMessage | undefined;
        if (!frame || frame.subscriptionId !== self.subscriptionId) return;
        const c = self;
        onFrame({ frame, acknowledge: () => c.ack(frame.sequence) });
        return;
      }
      if (m.type === "exit") return exit();
      if (NON_FRAME.has(m.type)) onMessage({ type: m.type, payload: m.payload } as Exclude<TerminalMessage, { type: "frame" }>);
    };
    const handle = await conn.openEmuTerminal(opts, deliver, exit);
    self = new RemoteTerminalConnection(handle);
    for (const m of pending.splice(0)) deliver(m);
    if (handle.title) onMessage({ type: "title", payload: handle.title });
    return self;
  }

  private ack(sequence: number): Promise<void> {
    this.handle.fire("emu.ack", { sub: this.subscriptionId, seq: sequence });
    return Promise.resolve();
  }

  input(data: string): Promise<void> {
    this.handle.fire("emu.input", { data });
    return Promise.resolve();
  }

  resize(columns: number, rows: number, pixelWidth: number, pixelHeight: number): Promise<TerminalViewport> {
    return this.handle.request<TerminalViewport>("emu.resize", { cols: columns, rows, pw: pixelWidth, ph: pixelHeight });
  }

  viewportReady(viewport: TerminalViewport): Promise<void> {
    this.handle.fire("emu.viewport_ready", { sub: this.subscriptionId, viewport });
    return Promise.resolve();
  }

  scroll(lines: number): Promise<void> {
    this.handle.fire("emu.scroll", { lines });
    return Promise.resolve();
  }

  mouse(event: Parameters<TerminalConnectionApi["mouse"]>[0]): Promise<boolean> {
    // Moves and wheel ticks are frequent; the desktop doesn't need to answer each one.
    this.handle.fire("emu.mouse", { req: event });
    return Promise.resolve(true);
  }

  focus(focused: boolean): Promise<void> {
    this.handle.fire("emu.focus", { focused });
    return Promise.resolve();
  }

  detach(): Promise<void> {
    this.handle.fire("emu.detach", { sub: this.subscriptionId });
    return Promise.resolve();
  }

  close(): Promise<void> {
    this.exited = true;
    this.handle.close();
    return Promise.resolve();
  }
}

export type HubState = "idle" | "connecting" | "connected" | "lost" | "error";
export type HubStatus = { state: HubState; code?: string; detail?: string };

/**
 * Owns the direct connection for the Buzz term page. One connect signature covers the connection; each terminal launch asks
 * the signer again. If the socket drops, new launches are refused until the user presses Reconnect, so a flaky tailnet can't
 * pop signer prompts by itself.
 */
export class RemoteTerminalHub implements TerminalBackend {
  private conn: RemoteConnection | null = null;
  private connecting: Promise<RemoteConnection> | null = null;
  private lost = false;
  private status: HubStatus = { state: "idle" };
  private listeners = new Set<() => void>();
  private getInfo: () => RemoteInfo | undefined;
  private sign: SignJson;
  private make: (info: RemoteInfo, sign: SignJson) => RemoteConnection;

  constructor(
    getInfo: () => RemoteInfo | undefined,
    sign: SignJson,
    make: (info: RemoteInfo, sign: SignJson) => RemoteConnection = (i, s) => new RemoteConnection(i, s),
  ) {
    this.getInfo = getInfo;
    this.sign = sign;
    this.make = make;
  }

  subscribe = (l: () => void) => {
    this.listeners.add(l);
    return () => void this.listeners.delete(l);
  };
  getStatus = (): HubStatus => this.status;
  /** End (ms) of the live connection's signed terminal window, 0 if none (no connection or no launch signed yet). */
  termSignedUntil = (): number => (this.conn?.ready ? this.conn.termSignedUntil : 0);
  private set(s: HubStatus) {
    this.status = s;
    for (const l of this.listeners) l();
  }

  /** The user asked to try again (Reconnect / Open terminal): allow a fresh connection and clear the error. */
  reset() {
    this.lost = false;
    if (!this.conn?.ready) this.set({ state: "idle" });
  }

  async ensure(): Promise<RemoteConnection> {
    if (this.conn?.ready) return this.conn;
    if (this.lost) throw new RemoteError("lost");
    if (this.connecting) return this.connecting;
    const info = this.getInfo();
    if (!info) throw new RemoteError("no_desktop");
    this.set({ state: "connecting" });
    const c = this.make(info, this.sign);
    c.onClose = () => {
      if (this.conn === c) {
        this.conn = null;
        this.lost = true;
        this.set({ state: "lost", code: "closed" });
      }
    };
    this.connecting = c
      .connect()
      .then(() => {
        this.conn = c;
        termProbe.registerPing(() => c.pingRtt(), c.transportLabel());
        this.set({ state: "connected" });
        return c;
      })
      .finally(() => {
        this.connecting = null;
      });
    return this.connecting;
  }

  async attach(request: TerminalAttachRequest, onMessage: OnMessage, onFrame: OnFrame): Promise<TerminalConnectionApi> {
    try {
      const conn = await this.ensure();
      if (!conn.terminals) throw new RemoteError("terminals_off");
      const info = this.getInfo();
      // The desktop's own Buzz Term setting decides shell vs herdr (the desktop never grants more than that); a handoff
      // session is always a plain shell so its launch line isn't typed into the herdr UI.
      const herdr = info?.termHost === "herdr" && request.remoteHost !== "shell";
      return await RemoteTerminalConnection.attach(
        conn,
        {
          cols: request.columns,
          rows: request.rows,
          ...(info?.termHost ? { host: herdr ? ("herdr" as const) : ("shell" as const) } : {}),
          ...(herdr && info?.termSession ? { session: info.termSession } : {}),
          ...(request.reattachId ? { reattachId: request.reattachId } : {}),
        },
        onMessage,
        onFrame,
      );
    } catch (e) {
      const err = e instanceof RemoteError ? e : new RemoteError(/reject|denied|declin/i.test(String(e)) ? "declined" : "other", String((e as Error)?.message ?? e));
      this.set({ state: this.lost ? "lost" : this.conn?.ready ? "connected" : "error", code: err.code, detail: err.detail });
      throw err;
    }
  }

  /** Desktop terminals that survived a refresh / dropped link (the desktop keeps them ~10 min). */
  async listLive(): Promise<Array<{ id: string; title?: string | null }>> {
    const conn = await this.ensure();
    if (!conn.terminals) return [];
    return conn.listTerminals();
  }

  /** "Close old sessions": end every desktop terminal still running from an earlier page, so a fresh one fits under the cap. */
  async closeAllLive(): Promise<number> {
    this.lost = false;
    const conn = await this.ensure();
    const live = await conn.listTerminals();
    const n = await conn.closeTerminals(live.filter((t) => !t.mine).map((t) => t.id));
    this.set({ state: "connected" });
    return n;
  }

  onError(_error: unknown) {}

  dispose() {
    this.conn?.close();
    this.conn = null;
  }
}
