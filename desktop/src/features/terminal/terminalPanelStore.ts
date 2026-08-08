import * as React from "react";

export type TerminalPanelMode = "closed" | "docked" | "maximized";

type Snapshot = {
  mode: TerminalPanelMode;
  sessionChannelIds: ReadonlySet<string>;
};

let snapshot: Snapshot = { mode: "closed", sessionChannelIds: new Set() };
const listeners = new Set<() => void>();

function publish(next: Snapshot) {
  snapshot = next;
  for (const listener of listeners) listener();
}

export function setTerminalPanelMode(mode: TerminalPanelMode) {
  if (snapshot.mode === mode) return;
  publish({ ...snapshot, mode });
}

export function toggleTerminalPanel() {
  setTerminalPanelMode(snapshot.mode === "closed" ? "docked" : "closed");
}

export function setTerminalSessionChannels(channelIds: Iterable<string>) {
  const next = new Set(channelIds);
  if (
    next.size === snapshot.sessionChannelIds.size &&
    [...next].every((id) => snapshot.sessionChannelIds.has(id))
  )
    return;
  publish({ ...snapshot, sessionChannelIds: next });
}

export function useTerminalPanel() {
  return React.useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    () => snapshot,
  );
}

export function resetTerminalPanelForTests() {
  snapshot = { mode: "closed", sessionChannelIds: new Set() };
}

export function getTerminalPanelSnapshotForTests() {
  return snapshot;
}
