/**
 * App-level event that routes a verified snapshot attachment (fetched
 * in-memory from a timeline card) to the existing AgentsView importer.
 *
 * Pattern mirrors openCreateAgentEvent.ts: a module-level pending payload
 * is set by the requester (the AgentSnapshotCard), consumed exactly once by
 * AgentsView when it mounts or is navigated to, and cleared on consumption
 * or cancel.  No localStorage, no React context — just a module-scoped
 * pending value + a window event for live navigation.
 */

export type PendingSnapshotImport = {
  fileBytes: number[];
  fileName: string;
};

const OPEN_SNAPSHOT_IMPORT_EVENT = "buzz:open-snapshot-import";

let pendingImport: PendingSnapshotImport | null = null;

/**
 * Enqueue a snapshot import and dispatch the navigation event.
 * The caller must have already fetched and validated the bytes in memory.
 * Clears any prior pending import so double-clicks don't stack.
 */
export function requestOpenSnapshotImport(payload: PendingSnapshotImport) {
  pendingImport = { fileBytes: payload.fileBytes, fileName: payload.fileName };
  if (typeof window !== "undefined") {
    window.dispatchEvent(new Event(OPEN_SNAPSHOT_IMPORT_EVENT));
  }
}

/**
 * Consume and clear the pending import — call this in AgentsView's useEffect.
 * Returns the payload if one was waiting, null otherwise.
 */
export function consumePendingSnapshotImport(): PendingSnapshotImport | null {
  const payload = pendingImport;
  pendingImport = null;
  return payload;
}

/**
 * Subscribe to the snapshot-import event — call this in AgentsView's useEffect
 * to handle navigations that happen while the view is already mounted.
 */
export function subscribeSnapshotImport(
  handler: (payload: PendingSnapshotImport) => void,
): () => void {
  function handleEvent() {
    const payload = consumePendingSnapshotImport();
    if (payload) {
      handler(payload);
    }
  }
  window.addEventListener(OPEN_SNAPSHOT_IMPORT_EVENT, handleEvent);
  return () => {
    window.removeEventListener(OPEN_SNAPSHOT_IMPORT_EVENT, handleEvent);
  };
}
