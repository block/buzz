/**
 * Parsed-document cache for the Documents editor.
 *
 * The live editor is deliberately remounted per file (see the `key` in
 * `DocumentEditorPane`) so undo can never resurrect a different note's text.
 * Creating the editor is cheap — measured at ~10ms — but the `setContent` that
 * follows re-parses the whole note through markdown-it every single time, and
 * that is not cheap at all: 249ms for a 110KB note, 62ms for a 22KB one.
 * Switching tabs paid it on every switch.
 *
 * Handing `setContent` the ProseMirror JSON it produced last time skips the
 * markdown parse entirely, which measured **30–47x faster** across a range of
 * real notes (249ms → 8ms on that 110KB note).
 *
 * The cache is keyed by path *and* the exact markdown it was built from, so a
 * stale entry is impossible: any difference in the source text is a miss, and a
 * miss simply re-parses.
 */

import type { JSONContent } from "@tiptap/core";

/** Enough for a working set of open tabs without holding a whole vault. */
const MAX_ENTRIES = 12;

type CachedDocument = {
  /** The exact markdown this JSON was parsed from. */
  markdown: string;
  /** ProseMirror document JSON, as returned by `editor.getJSON()`. */
  json: JSONContent;
};

const cache = new Map<string, CachedDocument>();

/** The parsed form of `markdown`, or null when it was never parsed here. */
export function getCachedDocument(
  path: string,
  markdown: string,
): JSONContent | null {
  const entry = cache.get(path);
  if (!entry || entry.markdown !== markdown) return null;

  // Refresh insertion order so the working set survives eviction.
  cache.delete(path);
  cache.set(path, entry);
  return entry.json;
}

export function cacheDocument(
  path: string,
  markdown: string,
  json: JSONContent,
): void {
  cache.delete(path);
  cache.set(path, { json, markdown });
  while (cache.size > MAX_ENTRIES) {
    const oldest = cache.keys().next();
    if (oldest.done) break;
    cache.delete(oldest.value);
  }
}

/** Drops one entry — a closed tab, or a file that changed underneath us. */
export function forgetCachedDocument(path: string): void {
  cache.delete(path);
}

/**
 * Drops everything. Called when leaving a vault.
 *
 * Keys are absolute paths, so entries from another vault can never be served by
 * mistake; this only avoids holding a closed vault's documents until they age
 * out of the cap.
 */
export function clearDocumentCache(): void {
  cache.clear();
}
