// Per-message send status for optimistic rows, platform-neutral.
//  - desktop never configures it, so nothing changes there (failed sends still roll the optimistic row back).
//  - the web build (slow remote signers) calls `sendStatus.configure({ keepFailedRows: true, waitProbe })`:
//    a failed send keeps its row, marked "Failed — <reason>" with a Retry button, and a row that is still pending shows what it
//    is waiting on ("Waiting for your signer…") instead of a bare "Sending…".
import * as React from "react";

export type SendEntry = {
  phase: "sending" | "failed";
  reason?: string;
  /** Re-run the send (the platform layer reuses an already-signed event, so a retry never double-signs/double-posts). */
  retry?: () => void;
  /** Drop the failed row. */
  discard?: () => void;
};

const entries = new Map<string, SendEntry>();
const subs = new Set<() => void>();
const publishedSubs = new Set<(eventId: string) => void>();
const failedSubs = new Set<() => void>();
let version = 0;
let keepFailedRows = false;
let waitProbe: (() => string | null) | null = null;
const emit = () => {
  version++;
  for (const f of subs) f();
};

export const sendStatus = {
  configure(opts: { keepFailedRows?: boolean; waitProbe?: () => string | null }) {
    if (opts.keepFailedRows !== undefined) keepFailedRows = opts.keepFailedRows;
    if (opts.waitProbe) waitProbe = opts.waitProbe;
  },
  keepFailedRows: () => keepFailedRows,
  /** What the signer/relay is currently busy with, or null. */
  waiting: () => (waitProbe ? waitProbe() : null),
  get: (id: string) => entries.get(id),
  begin(id: string) {
    entries.set(id, { phase: "sending" });
    emit();
  },
  fail(id: string, reason: string, retry?: () => void, discard?: () => void) {
    entries.set(id, { phase: "failed", reason, retry, discard });
    for (const f of failedSubs) f();
    emit();
  },
  clear(id: string) {
    if (entries.delete(id)) emit();
  },
  /** A signed event was accepted by the relay: platform layers may forget any cached signature for it. */
  published(eventId: string) {
    for (const f of publishedSubs) f(eventId);
  },
  onPublished(f: (eventId: string) => void) {
    publishedSubs.add(f);
    return () => void publishedSubs.delete(f);
  },
  /** A send failed (timeout/relay/signer): platform layers may keep its signed event for a retry. */
  onFailed(f: () => void) {
    failedSubs.add(f);
    return () => void failedSubs.delete(f);
  },
  /** Re-render subscribers (e.g. when the platform's wait label changes). */
  notify: emit,
  subscribe(f: () => void) {
    subs.add(f);
    return () => void subs.delete(f);
  },
  version: () => version,
};

/** Status for one optimistic row; null for rows with no tracked send. Re-renders when the status or the wait label changes. */
export function useSendStatus(id: string): (SendEntry & { waitingFor: string | null }) | null {
  React.useSyncExternalStore(sendStatus.subscribe, sendStatus.version, sendStatus.version);
  const e = entries.get(id);
  if (!e) return null;
  return { ...e, waitingFor: e.phase === "sending" ? sendStatus.waiting() : null };
}
