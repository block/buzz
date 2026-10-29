/**
 * Passive liveness watchdog for the relay WebSocket.
 *
 * This intentionally does not write to the socket. tauri-plugin-websocket
 * 2.4.2 holds a global connection-manager mutex while awaiting `send()`, so a
 * watchdog probe sent into a half-open TCP path can block future reconnects
 * from registering. Instead we rely on inbound relay traffic (including the
 * relay's heartbeat pings) as the liveness signal.
 */
export type RelayStallWatchdogConfig = {
  intervalMs: number;
  idleTimeoutMs: number;
  /** Called once when a stall is detected. The watchdog stops itself first. */
  onStall: (error: Error) => void;
  /** Optional override for tests. */
  now?: () => number;
};

export class RelayStallWatchdog {
  private readonly intervalMs: number;
  private readonly idleTimeoutMs: number;
  private readonly onStall: (error: Error) => void;
  private readonly now: () => number;

  private intervalHandle: number | null = null;
  private lastInboundAt = 0;

  constructor(config: RelayStallWatchdogConfig) {
    this.intervalMs = config.intervalMs;
    this.idleTimeoutMs = config.idleTimeoutMs;
    this.onStall = config.onStall;
    this.now = config.now ?? (() => Date.now());
  }

  /** Idempotent. Safe to call from `connect()` completion. */
  start(): void {
    this.lastInboundAt = this.now();
    if (this.intervalHandle !== null) {
      return;
    }
    this.intervalHandle = window.setInterval(
      () => this.checkIdle(),
      this.intervalMs,
    );
  }

  /** Idempotent. Clears the passive idle check interval. */
  stop(): void {
    if (this.intervalHandle !== null) {
      window.clearInterval(this.intervalHandle);
      this.intervalHandle = null;
    }
    this.lastInboundAt = 0;
  }

  /** Record any inbound WS frame as proof the socket is still alive. */
  recordInbound(): void {
    if (this.intervalHandle === null) {
      return;
    }
    this.lastInboundAt = this.now();
  }

  private checkIdle(): void {
    if (this.lastInboundAt === 0) {
      this.lastInboundAt = this.now();
      return;
    }

    if (this.now() - this.lastInboundAt < this.idleTimeoutMs) {
      return;
    }

    this.stop();
    this.onStall(new Error("Relay socket stalled — no inbound frames."));
  }
}
