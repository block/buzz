/**
 * Per-pubkey localStorage record store for channels manually marked unread via
 * right-click → "mark unread". Keyed by channelId (not thread-root). Persisted
 * so the sidebar badge survives reload and the rail observer can read it for
 * inactive workspaces.
 *
 * Each entry stores the channel's own NIP-RS read marker (unix seconds) at the
 * moment mark-unread was invoked, or null if no marker existed yet. The rail
 * observer gates the forced-unread OR on this baseline: if the observed synced
 * read marker has since advanced past markerAtWhenForced, the cross-device read
 * wins and the dot is not lit.
 *
 * On identity change, the in-memory map is swapped to the current pubkey's
 * persisted data. Old pubkey data is NOT wiped from localStorage.
 *
 * NOT synced to the relay — NIP-RS markers are monotonic and cannot represent
 * a retrograde "unread" state. localStorage is best-effort (per-device).
 */

/** channelId → NIP-RS read marker (unix seconds) at force-time, or null */
export type ForcedUnreadMap = Record<string, number | null>;

const STORAGE_PREFIX = "buzz-forced-unread.v1";
const storageKey = (pubkey: string) => `${STORAGE_PREFIX}:${pubkey}`;

export const forcedUnreadStore = {
  read(pubkey: string): ForcedUnreadMap {
    try {
      const raw = window.localStorage.getItem(storageKey(pubkey));
      if (!raw) return {};
      const parsed: unknown = JSON.parse(raw);
      if (
        typeof parsed !== "object" ||
        parsed === null ||
        Array.isArray(parsed)
      )
        return {};
      const result: ForcedUnreadMap = {};
      for (const [k, v] of Object.entries(parsed)) {
        if (typeof k === "string" && (v === null || typeof v === "number")) {
          result[k] = v as number | null;
        }
      }
      return result;
    } catch {
      return {};
    }
  },
  write(pubkey: string, map: ForcedUnreadMap): void {
    try {
      window.localStorage.setItem(storageKey(pubkey), JSON.stringify(map));
    } catch {
      // Ignore storage errors (private browsing, quota exceeded).
    }
  },
};
