/**
 * Persisted Documents session: which notes were open, and which folders were
 * expanded.
 *
 * Note *content* is deliberately not persisted. Restoring a stale buffer over a
 * file that changed on disk would be a silent overwrite the moment autosave
 * fired — the same class of bug the round-trip guard and the watcher
 * reconciliation exist to prevent. Paths are re-read from disk on restore.
 */

export const DOCUMENT_SESSION_KEY = "buzz.documents.session.v1";

export type DocumentSessionSnapshot = {
  /** The vault this session belongs to. */
  vaultPath: string;
  /** Absolute paths of open tabs, in order. */
  openPaths: string[];
  activePath: string | null;
  expandedPaths: string[];
};

function isStringArray(value: unknown): value is string[] {
  return (
    Array.isArray(value) && value.every((item) => typeof item === "string")
  );
}

/**
 * Parses a stored snapshot, rejecting anything that does not belong to
 * `vaultPath`.
 *
 * Restoring another vault's paths would open tabs for files that do not exist
 * here, so a mismatch discards rather than filters.
 */
export function parseSessionSnapshot(
  raw: string | null,
  vaultPath: string,
): DocumentSessionSnapshot | null {
  if (!raw) return null;

  try {
    const parsed = JSON.parse(raw) as Partial<DocumentSessionSnapshot>;
    if (typeof parsed?.vaultPath !== "string") return null;
    if (parsed.vaultPath !== vaultPath) return null;
    if (!isStringArray(parsed.openPaths)) return null;
    if (!isStringArray(parsed.expandedPaths)) return null;

    const activePath =
      typeof parsed.activePath === "string" &&
      parsed.openPaths.includes(parsed.activePath)
        ? parsed.activePath
        : null;

    return {
      activePath,
      expandedPaths: parsed.expandedPaths,
      openPaths: parsed.openPaths,
      vaultPath,
    };
  } catch {
    return null;
  }
}

export function readSessionSnapshot(
  vaultPath: string,
): DocumentSessionSnapshot | null {
  try {
    return parseSessionSnapshot(
      window.localStorage.getItem(DOCUMENT_SESSION_KEY),
      vaultPath,
    );
  } catch {
    return null;
  }
}

export function writeSessionSnapshot(snapshot: DocumentSessionSnapshot): void {
  try {
    window.localStorage.setItem(DOCUMENT_SESSION_KEY, JSON.stringify(snapshot));
  } catch {
    // A full or unavailable store costs the user their tab layout, nothing
    // more; never let it break editing.
  }
}
