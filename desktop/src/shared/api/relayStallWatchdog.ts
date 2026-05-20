/**
 * Application-level liveness probe for the relay WebSocket.
 *
 * Tungstenite auto-pongs and the OS keeps the TCP socket open, so a
 * half-open WS (Warp's orange-icon state, an asleep VPN, etc.) presents as
 * "fully connected" to the WS layer indefinitely — no Close, no Error.
 *
 * We work around that by periodically sending a cheap NIP-01 `REQ` with a
 * filter that matches nothing real (kind 9999, far-future `since`) and
 * waiting for the matching `EOSE`. If the relay doesn't answer within
 * `probeTimeoutMs` (or the send itself fails), `onStall` is invoked with an
 * `Error` describing the failure. The relay client then force-resets the
 * socket so its existing reconnect path runs.
 *
 * The watchdog has no opinion on connection state or reconnects; it just
 * detects that the socket is unhealthy and reports it.
 */
export type RelayStallWatchdogConfig = {
  intervalMs: number;
  probeTimeoutMs: number;
  /** Send a raw NIP-01 frame. Returns the same promise as the WS layer. */
  sendRaw: (payload: unknown[]) => Promise<void>;
  /** Called once when a stall is detected. The watchdog stops itself first. */
  onStall: (error: Error) => void;
  /** Optional override for tests. */
  now?: () => number;
};

export class RelayStallWatchdog {
  private readonly intervalMs: number;
  private readonly probeTimeoutMs: number;
  private readonly sendRaw: (payload: unknown[]) => Promise<void>;
  private readonly onStall: (error: Error) => void;
  private readonly now: () => number;

  private intervalHandle: number | null = null;
  private probeTimeoutHandle: number | null = null;
  private currentProbeSubId: string | null = null;

  constructor(config: RelayStallWatchdogConfig) {
    this.intervalMs = config.intervalMs;
    this.probeTimeoutMs = config.probeTimeoutMs;
    this.sendRaw = config.sendRaw;
    this.onStall = config.onStall;
    this.now = config.now ?? (() => Math.floor(Date.now() / 1_000));
  }

  /** Idempotent. Safe to call from `connect()` completion. */
  start(): void {
    if (this.intervalHandle !== null) {
      return;
    }
    this.intervalHandle = window.setInterval(
      () => this.sendProbe(),
      this.intervalMs,
    );
  }

  /** Idempotent. Clears any in-flight probe + the interval. */
  stop(): void {
    if (this.intervalHandle !== null) {
      window.clearInterval(this.intervalHandle);
      this.intervalHandle = null;
    }
    if (this.probeTimeoutHandle !== null) {
      window.clearTimeout(this.probeTimeoutHandle);
      this.probeTimeoutHandle = null;
    }
    this.currentProbeSubId = null;
  }

  /**
   * Called from the relay client's `handleEose` to satisfy the in-flight
   * probe. Returns `true` if the subId belonged to the watchdog (and the
   * caller should not look it up in the normal subscription map).
   */
  handleEose(subId: string): boolean {
    if (subId !== this.currentProbeSubId) {
      return false;
    }
    if (this.probeTimeoutHandle !== null) {
      window.clearTimeout(this.probeTimeoutHandle);
      this.probeTimeoutHandle = null;
    }
    this.currentProbeSubId = null;
    return true;
  }

  private sendProbe(): void {
    if (this.probeTimeoutHandle !== null) {
      // A probe is still outstanding — don't pile on; the timeout handler
      // will declare the stall when it fires.
      return;
    }

    const subId = `probe-${crypto.randomUUID()}`;
    this.currentProbeSubId = subId;
    this.probeTimeoutHandle = window.setTimeout(() => {
      this.probeTimeoutHandle = null;
      this.currentProbeSubId = null;
      this.fail(
        new Error("Relay socket stalled — no response to liveness probe."),
      );
    }, this.probeTimeoutMs);

    const farFuture = this.now() + 86_400;
    void this.sendRaw([
      "REQ",
      subId,
      { kinds: [9999], limit: 0, since: farFuture },
    ]).catch((error) => {
      // Send failed → the socket is dead.
      if (this.probeTimeoutHandle !== null) {
        window.clearTimeout(this.probeTimeoutHandle);
        this.probeTimeoutHandle = null;
      }
      this.currentProbeSubId = null;
      this.fail(
        error instanceof Error
          ? error
          : new Error("Relay socket stalled — probe send failed."),
      );
    });
  }

  private fail(error: Error): void {
    this.stop();
    this.onStall(error);
  }
}
