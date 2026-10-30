import { normalizeShortcode } from "@/shared/api/customEmoji";
import type { CustomEmoji } from "@/shared/lib/remarkCustomEmoji";

export const EMOJI_SEARCH_MIN_LENGTH = 2;
export const EMOJI_AUTOCOMPLETE_DEBOUNCE_MS = 250;
export const EMOJI_AUTOCOMPLETE_RESULT_CAP = 20;
export const EMOJI_PICKER_RESULT_CAP = 50;
export const EMOJI_PREFIX_CACHE_CAP = 64;
export const EMOJI_RESOLVED_CACHE_CAP = 500;
export const EMOJI_NEGATIVE_CACHE_CAP = 200;
export const EMOJI_NEGATIVE_TTL_MS = 60_000;

/** Insertion-ordered cache that evicts the least recently used key past `cap`. */
export class BoundedCache<V> {
  private readonly entries = new Map<string, V>();
  private readonly cap: number;

  constructor(cap: number) {
    this.cap = cap;
  }

  get size(): number {
    return this.entries.size;
  }

  get(key: string): V | undefined {
    const value = this.entries.get(key);
    if (value !== undefined) {
      this.entries.delete(key);
      this.entries.set(key, value);
    }
    return value;
  }

  set(key: string, value: V): void {
    this.entries.delete(key);
    this.entries.set(key, value);
    while (this.entries.size > this.cap) {
      const oldest = this.entries.keys().next();
      if (oldest.done) break;
      this.entries.delete(oldest.value);
    }
  }

  clear(): void {
    this.entries.clear();
  }
}

/** Lowercase, drop a leading colon, and turn `_`/`-` into spaces like the entry `content`. */
export function normalizeEmojiSearchPrefix(raw: string): string {
  return raw
    .trim()
    .toLowerCase()
    .replace(/^:+/, "")
    .replace(/[_-]+/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

export type PrefixSearchOutcome =
  | { status: "ok"; prefix: string; results: CustomEmoji[] }
  | { status: "short" }
  | { status: "stale" }
  | { status: "error"; prefix: string; error: unknown };

export type PrefixSearcher = {
  /**
   * Search after the debounce. Only the newest call gets `ok` or `error`; a
   * call that a newer call or `reset` replaced resolves `stale`.
   */
  request(raw: string): Promise<PrefixSearchOutcome>;
  /** Drop the cache and fence every pending request out. */
  reset(): void;
};

export function createPrefixSearcher(options: {
  fetch: (prefix: string) => Promise<CustomEmoji[]>;
  debounceMs: number;
  resultCap: number;
  cacheCap?: number;
  onFresh?: (results: CustomEmoji[]) => void;
}): PrefixSearcher {
  const cache = new BoundedCache<CustomEmoji[]>(
    options.cacheCap ?? EMOJI_PREFIX_CACHE_CAP,
  );
  let requestNumber = 0;
  let generation = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let supersede: (() => void) | undefined;

  function cancelPending() {
    if (timer !== undefined) clearTimeout(timer);
    timer = undefined;
    supersede?.();
    supersede = undefined;
  }

  return {
    request(raw) {
      cancelPending();
      requestNumber += 1;
      const mine = requestNumber;
      const myGeneration = generation;
      const prefix = normalizeEmojiSearchPrefix(raw);
      if (prefix.length < EMOJI_SEARCH_MIN_LENGTH) {
        return Promise.resolve({ status: "short" });
      }
      const cached = cache.get(prefix);
      if (cached) {
        return Promise.resolve({
          status: "ok",
          prefix,
          results: cached.slice(0, options.resultCap),
        });
      }
      const isCurrent = () =>
        mine === requestNumber && myGeneration === generation;
      return new Promise<PrefixSearchOutcome>((resolve) => {
        supersede = () => resolve({ status: "stale" });
        timer = setTimeout(() => {
          timer = undefined;
          supersede = undefined;
          options
            .fetch(prefix)
            .then((results) => {
              if (myGeneration !== generation) {
                resolve({ status: "stale" });
                return;
              }
              cache.set(prefix, results);
              options.onFresh?.(results);
              resolve(
                isCurrent()
                  ? {
                      status: "ok",
                      prefix,
                      results: results.slice(0, options.resultCap),
                    }
                  : { status: "stale" },
              );
            })
            .catch((error: unknown) => {
              resolve(
                isCurrent()
                  ? { status: "error", prefix, error }
                  : { status: "stale" },
              );
            });
        }, options.debounceMs);
      });
    },
    reset() {
      generation += 1;
      requestNumber += 1;
      cancelPending();
      cache.clear();
    },
  };
}

export type EmojiResolveResult = {
  found: Map<string, CustomEmoji>;
  /** The batched lookup failed; names it covered are unresolved, not absent. */
  failed: boolean;
};

export type EmojiResolver = {
  /** Cached emoji for a shortcode, without a network call. */
  lookup(shortcode: string): CustomEmoji | undefined;
  /** Add known emoji, such as search results, to the cache. */
  remember(emoji: ReadonlyArray<CustomEmoji>): void;
  /** Resolve names by cache, then by one batched fetch for the rest. */
  resolve(names: ReadonlyArray<string>): Promise<EmojiResolveResult>;
  reset(): void;
};

export function createEmojiResolver(options: {
  fetch: (names: string[]) => Promise<CustomEmoji[]>;
  now?: () => number;
  /** Most names one fetch covers; the rest stay unresolved and are not cached as absent. */
  maxNames?: number;
}): EmojiResolver {
  const now = options.now ?? Date.now;
  const resolved = new BoundedCache<CustomEmoji>(EMOJI_RESOLVED_CACHE_CAP);
  const missing = new BoundedCache<number>(EMOJI_NEGATIVE_CACHE_CAP);
  let generation = 0;

  return {
    lookup: (shortcode) => resolved.get(shortcode),
    remember(emoji) {
      for (const entry of emoji) resolved.set(entry.shortcode, entry);
    },
    async resolve(names) {
      const found = new Map<string, CustomEmoji>();
      const toFetch: string[] = [];
      const seen = new Set<string>();
      for (const raw of names) {
        const name = normalizeShortcode(raw);
        if (!name || seen.has(name)) continue;
        seen.add(name);
        const hit = resolved.get(name);
        if (hit) {
          found.set(name, hit);
          continue;
        }
        const missingUntil = missing.get(name);
        if (missingUntil !== undefined && missingUntil > now()) continue;
        toFetch.push(name);
      }
      toFetch.splice(options.maxNames ?? Number.POSITIVE_INFINITY);
      if (toFetch.length === 0) return { found, failed: false };
      const myGeneration = generation;
      let entries: CustomEmoji[];
      try {
        entries = await options.fetch(toFetch);
      } catch (error) {
        console.warn("Emoji lookup failed; names stay plain text.", error);
        return { found, failed: true };
      }
      if (myGeneration !== generation) return { found, failed: false };
      const returned = new Set<string>();
      for (const entry of entries) {
        if (!seen.has(entry.shortcode)) continue;
        returned.add(entry.shortcode);
        resolved.set(entry.shortcode, entry);
        found.set(entry.shortcode, entry);
      }
      const expiresAt = now() + EMOJI_NEGATIVE_TTL_MS;
      for (const name of toFetch) {
        if (!returned.has(name)) missing.set(name, expiresAt);
      }
      return { found, failed: false };
    },
    reset() {
      generation += 1;
      resolved.clear();
      missing.clear();
    },
  };
}
