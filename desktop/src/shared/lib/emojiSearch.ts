import data from "@emoji-mart/data";

/**
 * Shortcode fuzzy matching for the `:shortcode` emoji autocomplete.
 *
 * emoji-mart's own search is token-prefix based, so it can't cross the `_`
 * separator in a shortcode — `pointup` never finds `point_up`. This module adds
 * separator-insensitive matching in tiers so recall improves without reordering
 * the good hits: exact > prefix > substring > subsequence. Tiers 1–3 keep normal
 * queries identical to before; the subsequence tier only surfaces looser matches
 * (`pntup` → `point_up`) and always ranks last.
 */

// Lower tier = stronger match.
const TIER_EXACT = 0;
const TIER_PREFIX = 1;
const TIER_SUBSTRING = 2;
const TIER_SUBSEQUENCE = 3;

/** Lowercase and drop separators (`:`, `_`, `-`, whitespace) so matching is
 *  insensitive to shortcode punctuation. `point_up` and `pointup` collapse to
 *  the same normalized form. */
export function normalizeShortcode(value: string): string {
  return value.toLowerCase().replace(/[:_\s-]/g, "");
}

export interface ShortcodeMatch {
  tier: number;
  /** Lower is better within a tier. */
  score: number;
}

/**
 * If `query` is a subsequence of `target` (all chars in order, gaps allowed),
 * return the span of the match (`lastIndex - firstIndex`) as a tightness score —
 * smaller is tighter, hence better. Returns null when not a subsequence.
 */
function subsequenceSpan(query: string, target: string): number | null {
  let first = -1;
  let last = -1;
  let qi = 0;
  for (let ti = 0; ti < target.length && qi < query.length; ti++) {
    if (target[ti] === query[qi]) {
      if (first === -1) first = ti;
      last = ti;
      qi++;
    }
  }
  if (qi < query.length) return null;
  return last - first;
}

/**
 * Score `query` against a single `shortcode`. Returns null when there is no
 * match at any tier. Both sides are normalized first, so separators are ignored.
 */
export function scoreShortcodeMatch(
  query: string,
  shortcode: string,
): ShortcodeMatch | null {
  const q = normalizeShortcode(query);
  if (q.length === 0) return null;
  const target = normalizeShortcode(shortcode);
  if (target.length === 0) return null;

  if (q === target) return { tier: TIER_EXACT, score: 0 };
  if (target.startsWith(q)) return { tier: TIER_PREFIX, score: 0 };
  const idx = target.indexOf(q);
  if (idx !== -1) return { tier: TIER_SUBSTRING, score: idx };
  const span = subsequenceSpan(q, target);
  if (span !== null) return { tier: TIER_SUBSEQUENCE, score: span };
  return null;
}

/**
 * Rank `items` by how well their shortcode matches `query`, best first, capped
 * at `limit`. Ordering: tier, then in-tier score, then shorter shortcode (more
 * specific), then alphabetical for a stable result.
 */
export function rankByShortcode<T>(
  query: string,
  items: readonly T[],
  shortcodeOf: (item: T) => string,
  limit: number,
): T[] {
  if (limit <= 0) return [];
  const scored: Array<{
    item: T;
    tier: number;
    score: number;
    len: number;
    code: string;
  }> = [];
  for (const item of items) {
    const code = shortcodeOf(item);
    const match = scoreShortcodeMatch(query, code);
    if (match) {
      scored.push({
        item,
        tier: match.tier,
        score: match.score,
        len: code.length,
        code,
      });
    }
  }
  scored.sort(
    (a, b) =>
      a.tier - b.tier ||
      a.score - b.score ||
      a.len - b.len ||
      (a.code < b.code ? -1 : a.code > b.code ? 1 : 0),
  );
  return scored.slice(0, limit).map((s) => s.item);
}

export interface StandardEmoji {
  id: string;
  name: string;
  native: string;
}

type EmojiMartData = {
  emojis?: Record<
    string,
    {
      id?: string;
      name?: string;
      skins?: Array<{ native?: string }>;
    }
  >;
};

// Flat index of standard emoji built once from the emoji-mart dataset. ~1.8k
// short entries; scanning it per keystroke (behind the composer's debounce) is
// sub-millisecond. Module-level so the cost is paid once, not per hook mount.
let standardIndex: StandardEmoji[] | null = null;

function getStandardIndex(): StandardEmoji[] {
  if (standardIndex) return standardIndex;
  const emojis = (data as EmojiMartData).emojis ?? {};
  const out: StandardEmoji[] = [];
  for (const [id, emoji] of Object.entries(emojis)) {
    const native = emoji.skins?.[0]?.native ?? "";
    if (!native) continue;
    out.push({ id: emoji.id ?? id, name: emoji.name ?? id, native });
  }
  standardIndex = out;
  return out;
}

/**
 * Fuzzy shortcode matches over the standard emoji set, used to top up the
 * autocomplete when emoji-mart's own search misses (it can't cross `_`). Skips
 * ids already shown (`excludeIds`) so it only adds, never duplicates.
 */
export function fuzzyStandardEmoji(
  query: string,
  limit: number,
  excludeIds: ReadonlySet<string>,
): StandardEmoji[] {
  if (limit <= 0) return [];
  const candidates = getStandardIndex().filter((e) => !excludeIds.has(e.id));
  return rankByShortcode(query, candidates, (e) => e.id, limit);
}
