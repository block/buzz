import * as React from "react";

/**
 * Channels whose most recent send failed (timeout or relay rejection) and
 * whose prompt was stashed back into that channel's draft slot. The
 * navigator and tab strip surface these so a failure noticed after
 * switching away still points back at the channel holding the draft.
 * Module-level (like composer drafts) so the record survives display-style
 * toggles; intentionally in-memory only.
 */
const listeners = new Set<() => void>();

let failed: ReadonlySet<string> = new Set();

function write(next: ReadonlySet<string>) {
  failed = next;
  for (const listener of listeners) {
    listener();
  }
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function sendFailureChannelIds(): ReadonlySet<string> {
  return failed;
}

export function useSendFailureChannelIds(): ReadonlySet<string> {
  return React.useSyncExternalStore(
    subscribe,
    sendFailureChannelIds,
    sendFailureChannelIds,
  );
}

export function recordSendFailure(channelId: string): void {
  if (failed.has(channelId)) return;
  write(new Set([...failed, channelId]));
}

/** Opening the channel or landing a send there resolves the failure. */
export function clearSendFailure(channelId: string): void {
  if (!failed.has(channelId)) return;
  const next = new Set(failed);
  next.delete(channelId);
  write(next);
}

/** Tests only — the module-level set would otherwise leak across cases. */
export function clearSendFailures(): void {
  write(new Set());
}
