// Read-only mirror of TerminalBootstrap's session tabs plus its own select/close handlers, so the phone tab
// switcher can list and act on Buzz Term tabs without a second copy of the session logic.
import * as React from "react";

export type TerminalTabsSnapshot = {
  tabs: { id: string; title: string; active: boolean; channelName?: string | null }[];
  select: (id: string) => void;
  close: (id: string) => void;
};

const EMPTY: TerminalTabsSnapshot = { tabs: [], select: () => {}, close: () => {} };
let snapshot: TerminalTabsSnapshot = EMPTY;
const listeners = new Set<() => void>();

export function publishTerminalTabs(next: TerminalTabsSnapshot | null): void {
  snapshot = next ?? EMPTY;
  for (const l of listeners) l();
}

export function useTerminalTabs(): TerminalTabsSnapshot {
  return React.useSyncExternalStore(
    (cb) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
    () => snapshot,
    () => EMPTY,
  );
}
