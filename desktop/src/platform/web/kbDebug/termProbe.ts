// Buzz Term latency probe (web diagnostics). Cheap no-ops unless the kbdebug overlay is on.
// input -> first frame back (echo) and input -> first canvas paint after that frame; plus Direct ping RTT.
export type ProbeStats = { n: number; avg: number; p95: number; last: number };
const WINDOW = 60;
let enabled = false;
let pendingInput: number | null = null;
let echoAt: number | null = null;
const echo: number[] = [];
const paint: number[] = [];
const ping: number[] = [];
let framesSinceInput = 0;
let pingFn: (() => Promise<number>) | null = null;
let transport = "";

const push = (a: number[], v: number) => {
  a.push(v);
  if (a.length > WINDOW) a.shift();
};
export function stats(a: readonly number[]): ProbeStats {
  if (!a.length) return { n: 0, avg: 0, p95: 0, last: 0 };
  const sorted = [...a].sort((x, y) => x - y);
  return {
    n: a.length,
    avg: a.reduce((x, y) => x + y, 0) / a.length,
    p95: sorted[Math.min(sorted.length - 1, Math.ceil(sorted.length * 0.95) - 1)],
    last: a[a.length - 1],
  };
}
export const termProbe = {
  setEnabled(on: boolean) {
    enabled = on;
  },
  inputSent() {
    if (!enabled) return;
    // a held key: measure from the oldest unanswered key (that's what you feel)
    if (pendingInput === null) pendingInput = performance.now();
    framesSinceInput = 0;
  },
  frameArrived() {
    if (!enabled) return;
    framesSinceInput++;
    if (pendingInput !== null && echoAt === null) {
      echoAt = performance.now();
      push(echo, echoAt - pendingInput);
    }
  },
  painted() {
    if (!enabled || pendingInput === null || echoAt === null) return;
    push(paint, performance.now() - pendingInput);
    pendingInput = null;
    echoAt = null;
  },
  registerPing(fn: (() => Promise<number>) | null, kind = "") {
    pingFn = fn;
    transport = kind;
  },
  async pingOnce(): Promise<void> {
    if (!enabled || !pingFn) return;
    try {
      push(ping, await pingFn());
    } catch {
      /* no connection */
    }
  },
  snapshot() {
    return { echo: stats(echo), paint: stats(paint), ping: stats(ping), transport, hasPing: !!pingFn, framesSinceInput };
  },
  reset() {
    echo.length = paint.length = ping.length = 0;
    pendingInput = echoAt = null;
  },
};
