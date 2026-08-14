/**
 * App-level serialized handoff for snapshot imports. A request remains owned
 * by AgentsView after acceptance until the corresponding dialog closes, so a
 * later request can never replace a user-visible preview or result dialog.
 */

export type PendingSnapshotImport = {
  id: string;
  fileBytes: number[];
  fileName: string;
  snapshotKind: "agent" | "team";
};

type SnapshotImportRequest = Omit<PendingSnapshotImport, "id"> & {
  id?: string;
  onAccepted?: () => void;
  onRejected?: (id: string) => void;
  onReleased?: () => void;
};

type PendingSnapshotCallbacks = {
  onAccepted?: () => void;
  onRejected?: (id: string) => void;
  onReleased?: () => void;
};

const OPEN_SNAPSHOT_IMPORT_EVENT = "buzz:open-snapshot-import";

let nextSnapshotImportId = 0;
const pendingImports: PendingSnapshotImport[] = [];
const pendingCallbacks = new Map<string, PendingSnapshotCallbacks>();
let deliveringId: string | null = null;
let acceptedId: string | null = null;

function currentPendingImport(): PendingSnapshotImport | null {
  return pendingImports[0] ?? null;
}

function nextImportId(): string {
  nextSnapshotImportId += 1;
  return `snapshot-import-${nextSnapshotImportId}`;
}

/** Queue a verified in-memory snapshot import for eventual AgentsView delivery. */
export function requestOpenSnapshotImport(
  payload: SnapshotImportRequest,
): boolean {
  const request: PendingSnapshotImport = {
    id: payload.id ?? nextImportId(),
    fileBytes: payload.fileBytes,
    fileName: payload.fileName,
    snapshotKind: payload.snapshotKind,
  };
  if (pendingImports.some((pending) => pending.id === request.id)) return false;

  pendingCallbacks.set(request.id, {
    onAccepted: payload.onAccepted,
    onRejected: payload.onRejected,
    onReleased: payload.onReleased,
  });
  pendingImports.push(request);
  if (typeof window !== "undefined") {
    window.dispatchEvent(new Event(OPEN_SNAPSHOT_IMPORT_EVENT));
  }
  return true;
}

/** Peek at the queue head; this does not end route ownership. */
export function consumePendingSnapshotImport(): PendingSnapshotImport | null {
  return currentPendingImport();
}

/** Claim the pending head before beginning an asynchronous route-local preview. */
export function claimPendingSnapshotImport(id: string): boolean {
  if (
    deliveringId !== null ||
    acceptedId !== null ||
    currentPendingImport()?.id !== id
  ) {
    return false;
  }
  deliveringId = id;
  return true;
}

/** Accept the head from exactly one matching route-local consumer. */
export function acceptPendingSnapshotImport(id: string): boolean {
  if (deliveringId !== id || currentPendingImport()?.id !== id) return false;
  deliveringId = null;
  acceptedId = id;
  pendingCallbacks.get(id)?.onAccepted?.();
  return true;
}

/**
 * Remove a preview-rejected head after its route-local error is visibly
 * surfaced. It was never accepted, so it has no dialog ownership to release.
 */
export function rejectPendingSnapshotImport(id: string): boolean {
  if (currentPendingImport()?.id !== id || acceptedId !== null) return false;
  pendingImports.shift();
  const callbacks = pendingCallbacks.get(id);
  pendingCallbacks.delete(id);
  deliveringId = null;
  callbacks?.onRejected?.(id);
  if (typeof window !== "undefined" && pendingImports.length > 0) {
    window.dispatchEvent(new Event(OPEN_SNAPSHOT_IMPORT_EVENT));
  }
  return true;
}

/**
 * Release an accepted import only after `teamSnapshotImportState` was cleared
 * and its dialog closed. Confirmation result/error states never call this.
 */
export function releasePendingSnapshotImport(id: string): boolean {
  if (acceptedId !== id || currentPendingImport()?.id !== id) return false;

  pendingImports.shift();
  const callbacks = pendingCallbacks.get(id);
  pendingCallbacks.delete(id);
  acceptedId = null;
  callbacks?.onReleased?.();
  deliveringId = null;
  if (typeof window !== "undefined" && pendingImports.length > 0) {
    window.dispatchEvent(new Event(OPEN_SNAPSHOT_IMPORT_EVENT));
  }
  return true;
}

/** Subscribe to delivery opportunities while AgentsView is mounted. */
export function subscribeSnapshotImport(
  handler: (payload: PendingSnapshotImport) => void,
): () => void {
  function handleEvent() {
    const payload = currentPendingImport();
    if (payload && acceptedId === null && deliveringId === null)
      handler(payload);
  }

  window.addEventListener(OPEN_SNAPSHOT_IMPORT_EVENT, handleEvent);
  return () =>
    window.removeEventListener(OPEN_SNAPSHOT_IMPORT_EVENT, handleEvent);
}
