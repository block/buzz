/**
 * One-summary-per-send timing probe for the composer's send path.
 *
 * The send spinner spans every awaited step between the composer clearing and
 * the relay accepting the event, and none of those steps logged anything — so
 * "the send still feels slow" could only be answered by guessing which step
 * dominated. `createSendPerfTimer` wraps each awaited step and emits a single
 * `[send-perf]` line naming the winner, alongside the flags that decide which
 * steps ran at all (deferred upload, link previews, relay side effects).
 *
 * The summary is mirrored into the backend's stderr stream through the
 * `log_send_perf` command: WKWebView drops `console` output produced before a
 * Web Inspector attaches, and the Rust half of the same send already logs its
 * phases there. A terminal running `just dev` then shows both halves in order
 * with no inspector attached — which also makes a missing `[send-perf]` line
 * proof that the running app predates this build.
 *
 * These fire once per send click, never per keystroke, so they stay on by
 * default without disturbing the render-perf measurement guidance in AGENTS.md.
 */

import { invokeTauri } from "@/shared/api/tauri";

/** Non-timing values recorded with a send: counts, flags, the branch taken. */
export type SendPerfFacts = Record<string, unknown>;

export type SendPerfPayload = SendPerfFacts & {
  totalMs: number;
  /** Awaited step name to milliseconds, in the order the steps first ran. */
  steps: Record<string, number>;
};

/** Destination for a finished summary. Overridden in tests. */
export type SendPerfSink = (label: string, payload: SendPerfPayload) => void;

export type SendPerfTimer = {
  /** Time one awaited step. Its resolved value — or thrown error — passes through. */
  step<T>(name: string, run: () => Promise<T>): Promise<T>;
  /** Record a fact about this send. Later notes win on a repeated key. */
  note(facts: SendPerfFacts): void;
  /** Emit the summary. Repeat calls are ignored, so `finally` blocks are safe. */
  finish(facts?: SendPerfFacts): void;
};

function now(): number {
  return typeof performance === "undefined" ? Date.now() : performance.now();
}

/** Tenths of a millisecond — finer than the differences being chased. */
function roundMs(elapsed: number): number {
  return Math.round(elapsed * 10) / 10;
}

export const defaultSendPerfSink: SendPerfSink = (label, payload) => {
  console.info(`[send-perf] ${label}`, payload);
  void invokeTauri("log_send_perf", {
    label,
    payload: JSON.stringify(payload),
  }).catch(() => {
    // A probe must never be able to fail — or even warn about — a send.
  });
};

export function createSendPerfTimer(
  label: string,
  facts: SendPerfFacts = {},
  sink: SendPerfSink = defaultSendPerfSink,
): SendPerfTimer {
  const startedAt = now();
  const steps: Record<string, number> = {};
  const recorded: SendPerfFacts = { ...facts };
  let finished = false;
  return {
    async step(name, run) {
      const stepStartedAt = now();
      try {
        return await run();
      } finally {
        // Accumulate rather than overwrite: a step that runs twice in one send
        // should read as its total contribution, not just the last attempt.
        steps[name] = roundMs((steps[name] ?? 0) + (now() - stepStartedAt));
      }
    },
    note(next) {
      Object.assign(recorded, next);
    },
    finish(next) {
      if (finished) return;
      finished = true;
      Object.assign(recorded, next);
      sink(label, { ...recorded, totalMs: roundMs(now() - startedAt), steps });
    },
  };
}
