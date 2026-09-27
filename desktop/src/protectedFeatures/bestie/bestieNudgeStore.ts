import * as React from "react";

/** Why the proactive Bestie nudge fired — due reminders auto-open the popover. */
export type BestieNudgeReason = "due-reminder" | "check-in";

/** Proactive Bestie wake nudge — distinct from ordinary DM notifications. */
export type BestieNudge = {
  body: string;
  createdAt: number;
  id: string;
  /** Open reminder / todo ids that triggered this nudge. */
  itemIds: string[];
  reason: BestieNudgeReason;
  title: string;
};

type Listener = () => void;

const listeners = new Set<Listener>();
let activeNudge: BestieNudge | null = null;

function notify() {
  for (const listener of listeners) listener();
}

export function getBestieNudge(): BestieNudge | null {
  return activeNudge;
}

export function setBestieNudge(nudge: BestieNudge | null): void {
  activeNudge = nudge;
  notify();
}

export function clearBestieNudge(): void {
  if (!activeNudge) return;
  activeNudge = null;
  notify();
}

export function useBestieNudge(): BestieNudge | null {
  return React.useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    getBestieNudge,
    () => null,
  );
}
