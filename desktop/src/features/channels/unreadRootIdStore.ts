// Per-pubkey JSON array of thread root ids, capped to newest entries and
// tolerant of malformed or unavailable localStorage.
export function makeRootIdStore(prefix: string, maxEntries = 1000) {
  const storageKey = (pubkey: string) => `${prefix}:${pubkey}`;
  return {
    read(pubkey: string): Set<string> {
      try {
        const raw = window.localStorage.getItem(storageKey(pubkey));
        if (!raw) return new Set();
        const parsed = JSON.parse(raw);
        if (!Array.isArray(parsed)) return new Set();
        return new Set(
          parsed.filter((id): id is string => typeof id === "string"),
        );
      } catch {
        return new Set();
      }
    },
    write(pubkey: string, rootIds: Set<string>): void {
      try {
        const arr = [...rootIds];
        const capped =
          arr.length > maxEntries ? arr.slice(arr.length - maxEntries) : arr;
        window.localStorage.setItem(storageKey(pubkey), JSON.stringify(capped));
      } catch {
        // Ignore storage errors (private browsing, quota exceeded).
      }
    },
  };
}
