/**
 * Bridges native pre-import failures into the existing Agents action-error
 * surface. The native queue entry is acknowledged only after AgentsView has
 * displayed the error for its matching id.
 */

export type NativeTeamSnapshotError = {
  id: string;
  message: string;
};

const NATIVE_TEAM_SNAPSHOT_ERROR_EVENT = "buzz:native-team-snapshot-error";

let pendingError: NativeTeamSnapshotError | null = null;
let onDisplayed: ((id: string) => void) | null = null;

export function requestNativeTeamSnapshotError(
  error: NativeTeamSnapshotError,
  acknowledge: (id: string) => void,
): boolean {
  if (pendingError !== null) return false;
  pendingError = error;
  onDisplayed = acknowledge;
  if (typeof window !== "undefined") {
    window.dispatchEvent(new Event(NATIVE_TEAM_SNAPSHOT_ERROR_EVENT));
  }
  return true;
}

export function consumeNativeTeamSnapshotError(): NativeTeamSnapshotError | null {
  return pendingError;
}

/** Mark the current error as rendered by the existing Agents error surface. */
export function markNativeTeamSnapshotErrorDisplayed(id: string): boolean {
  if (pendingError?.id !== id) return false;
  return true;
}

export function acknowledgeNativeTeamSnapshotError(id: string): boolean {
  if (pendingError?.id !== id) return false;
  const acknowledge = onDisplayed;
  pendingError = null;
  onDisplayed = null;
  acknowledge?.(id);
  return true;
}

export function subscribeNativeTeamSnapshotError(
  handler: (error: NativeTeamSnapshotError) => void,
): () => void {
  function handleError() {
    if (pendingError) handler(pendingError);
  }
  window.addEventListener(NATIVE_TEAM_SNAPSHOT_ERROR_EVENT, handleError);
  return () => {
    window.removeEventListener(NATIVE_TEAM_SNAPSHOT_ERROR_EVENT, handleError);
  };
}
