/**
 * Parse Slack-style search operators out of a free-text query.
 *
 * Remaining text is the FTS / prefix query. Invalid operator tokens (for example
 * `after:yesterday`) are left in the text so they still participate in FTS.
 *
 * Name resolution for `from:` / `in:` happens at the call site (autocomplete or
 * local channel/user lists). This module only splits syntax.
 */

export type ParsedSearchOperators = {
  /** FTS / prefix query with operators removed. */
  text: string;
  /** Raw `from:` value (pubkey hex, npub, @name, …), if present. */
  from: string | null;
  /** Raw `in:` value (channel uuid, #name, …), if present. */
  in: string | null;
  /** `after:YYYY-MM-DD` → unix seconds at local start of day. */
  since: number | null;
  /** `before:YYYY-MM-DD` → unix seconds at local start of day. */
  until: number | null;
};

const OPERATOR_RE = /\b(from|in|after|before):(\S+)/gi;

function parseLocalDayStart(value: string): number | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (!match) {
    return null;
  }

  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  if (month < 1 || month > 12 || day < 1 || day > 31) {
    return null;
  }

  const date = new Date(year, month - 1, day);
  if (
    date.getFullYear() !== year ||
    date.getMonth() !== month - 1 ||
    date.getDate() !== day
  ) {
    return null;
  }

  return Math.floor(date.getTime() / 1000);
}

/**
 * Extract `from:` / `in:` / `after:` / `before:` operators from `raw`.
 *
 * Later occurrences of the same operator win. Date operators that are not
 * `YYYY-MM-DD` stay in the returned `text`.
 */
export function parseSearchOperators(raw: string): ParsedSearchOperators {
  let from: string | null = null;
  let inValue: string | null = null;
  let since: number | null = null;
  let until: number | null = null;

  const kept: string[] = [];
  let lastIndex = 0;

  for (const match of raw.matchAll(OPERATOR_RE)) {
    const index = match.index ?? 0;
    kept.push(raw.slice(lastIndex, index));
    lastIndex = index + match[0].length;

    const kind = match[1].toLowerCase();
    const value = match[2];

    if (kind === "from") {
      from = value;
      continue;
    }
    if (kind === "in") {
      inValue = value;
      continue;
    }
    if (kind === "after") {
      const parsed = parseLocalDayStart(value);
      if (parsed === null) {
        kept.push(match[0]);
      } else {
        since = parsed;
      }
      continue;
    }
    if (kind === "before") {
      const parsed = parseLocalDayStart(value);
      if (parsed === null) {
        kept.push(match[0]);
      } else {
        until = parsed;
      }
    }
  }

  kept.push(raw.slice(lastIndex));

  return {
    text: kept.join("").replace(/\s+/g, " ").trim(),
    from,
    in: inValue,
    since,
    until,
  };
}

const HEX_PUBKEY_RE = /^[0-9a-f]{64}$/i;
const UUID_RE =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/** True when `value` is a 64-char hex pubkey (optional `0x` rejected). */
export function isHexPubkey(value: string): boolean {
  return HEX_PUBKEY_RE.test(value);
}

/** Strip a leading `@` from a `from:` value. */
export function normalizeFromHandle(value: string): string {
  return value.startsWith("@") ? value.slice(1) : value;
}

/** Strip a leading `#` from an `in:` value. */
export function normalizeInChannel(value: string): string {
  return value.startsWith("#") ? value.slice(1) : value;
}

/** True when `value` is a channel UUID. */
export function isChannelUuid(value: string): boolean {
  return UUID_RE.test(value);
}
