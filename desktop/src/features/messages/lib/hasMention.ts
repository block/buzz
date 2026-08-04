import { decode } from "nostr-tools/nip19";

/**
 * Escape special regex characters in a string.
 */
function escapeRegExp(str: string): string {
  return str.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function maskRange(
  chars: string[],
  text: string,
  start: number,
  end: number,
): void {
  for (let index = start; index < end; index += 1) {
    if (text[index] !== "\n" && text[index] !== "\r") chars[index] = " ";
  }
}

/**
 * Replace Markdown code with spaces while retaining offsets and line endings.
 * Handles fenced blocks, four-space/tab-indented lines, and backtick code spans.
 */
function maskMarkdownCode(text: string): string {
  const chars = text.split("");
  const lines: Array<{ start: number; end: number; content: string }> = [];

  let lineStart = 0;
  while (lineStart < text.length) {
    let lineEnd = lineStart;
    while (
      lineEnd < text.length &&
      text[lineEnd] !== "\n" &&
      text[lineEnd] !== "\r"
    ) {
      lineEnd += 1;
    }
    lines.push({
      start: lineStart,
      end: lineEnd,
      content: text.slice(lineStart, lineEnd),
    });
    if (text[lineEnd] === "\r" && text[lineEnd + 1] === "\n") lineEnd += 1;
    lineStart = lineEnd + 1;
  }

  let fence: { marker: string; length: number } | null = null;
  for (const line of lines) {
    if (fence) {
      maskRange(chars, text, line.start, line.end);
      const closing = line.content.match(/^ {0,3}(`+|~+)[ \t]*$/);
      if (
        closing &&
        closing[1][0] === fence.marker &&
        closing[1].length >= fence.length
      ) {
        fence = null;
      }
      continue;
    }

    const opening = line.content.match(/^ {0,3}(`{3,}|~{3,})(.*)$/);
    if (opening && !(opening[1][0] === "`" && opening[2].includes("`"))) {
      fence = { marker: opening[1][0], length: opening[1].length };
      maskRange(chars, text, line.start, line.end);
      continue;
    }

    if (/^(?: {4}|\t)/.test(line.content)) {
      maskRange(chars, text, line.start, line.end);
    }
  }

  const isMasked = (index: number) =>
    chars[index] === " " && text[index] !== " ";
  const isEscaped = (index: number) => {
    let slashCount = 0;
    for (
      let cursor = index - 1;
      cursor >= 0 && text[cursor] === "\\";
      cursor -= 1
    ) {
      slashCount += 1;
    }
    return slashCount % 2 === 1;
  };

  for (let index = 0; index < text.length; ) {
    if (text[index] !== "`" || isMasked(index) || isEscaped(index)) {
      index += 1;
      continue;
    }

    let openerEnd = index + 1;
    while (
      openerEnd < text.length &&
      text[openerEnd] === "`" &&
      !isMasked(openerEnd)
    ) {
      openerEnd += 1;
    }
    const delimiterLength = openerEnd - index;
    let closer = openerEnd;

    while (closer < text.length) {
      if (text[closer] !== "`" || isMasked(closer)) {
        closer += 1;
        continue;
      }
      let closerEnd = closer + 1;
      while (
        closerEnd < text.length &&
        text[closerEnd] === "`" &&
        !isMasked(closerEnd)
      ) {
        closerEnd += 1;
      }
      if (closerEnd - closer === delimiterLength) {
        maskRange(chars, text, index, closerEnd);
        index = closerEnd;
        break;
      }
      closer = closerEnd;
    }

    if (closer >= text.length) index = openerEnd;
  }

  return chars.join("");
}

/**
 * Check whether `text` contains an @mention of `name`.
 *
 * Matches `@Name` preceded by start-of-string, whitespace, an opening
 * parenthesis (for team expansions), markdown
 * bold/italic markers (`*`, `**`, `***`, `_`, `__`, `___`), or spoiler
 * delimiters (`||`). This handles the case where a mention is pasted from the
 * chat area and TipTap's Bold extension wraps it in bold marks (font-weight >=
 * 500 -> bold), plus messages whose visible mention text is spoilered.
 *
 * Exported separately so it can be unit-tested without importing React.
 */
export function getMentionOffset(text: string, name: string): number | null {
  const escaped = escapeRegExp(name);
  const pattern = new RegExp(
    `(^|\\s|\\(|[*_]{1,3}|\\|\\|)(@${escaped})(?=\\|\\||[\\s,;.!?:)\\]}*_]|$)`,
    "i",
  );
  const match = pattern.exec(maskMarkdownCode(text));
  return match ? match.index + match[1].length : null;
}

export function hasMention(text: string, name: string): boolean {
  return getMentionOffset(text, name) !== null;
}

// ---------------------------------------------------------------------------
// NIP-27 outbound encoding and inbound materialization
// ---------------------------------------------------------------------------

type Range = { start: number; end: number };

/**
 * Build an array of character ranges that must not be transformed by
 * @-mention substitution: inline/fenced/indented code spans, Markdown link
 * destinations (the URL inside `](...)`), existing `nostr:` URIs (so
 * already-encoded references are not re-encoded), email-like tokens whose `@`
 * is not a mention trigger, and backslash-escaped `\@` literals.
 */
function buildProtectedRanges(text: string, protectNostr = true): Range[] {
  const ranges: Range[] = [];
  const masked = maskMarkdownCode(text);

  // Code spans and blocks — maskMarkdownCode replaces non-space, non-newline
  // chars inside code with spaces. Detect contiguous such runs.
  let codeRangeStart = -1;
  for (let i = 0; i <= text.length; i++) {
    const isCodeChar =
      i < text.length &&
      masked[i] === " " &&
      text[i] !== " " &&
      text[i] !== "\n" &&
      text[i] !== "\r";
    if (isCodeChar && codeRangeStart === -1) {
      codeRangeStart = i;
    } else if (!isCodeChar && codeRangeStart !== -1) {
      ranges.push({ start: codeRangeStart, end: i });
      codeRangeStart = -1;
    }
  }

  let m: RegExpExecArray | null;

  // Markdown link destinations: [text](URL) — protect from `(` to closing `)`
  // so that an `@name` appearing in a URL is not substituted.
  const linkDestRe = /\]\(([^)\n]*)\)/g;
  // biome-ignore lint/suspicious/noAssignInExpressions: scan loop
  while ((m = linkDestRe.exec(text)) !== null) {
    // Protect the `(URL)` portion (index of `]` + 1 through the `)`)
    ranges.push({ start: m.index + 1, end: m.index + m[0].length });
  }

  // Existing nostr: URIs — protect them wholesale so already-encoded
  // references are not re-encoded and non-profile entities are left unchanged.
  const nostrRe = /nostr:[a-zA-Z0-9]+/g;
  // biome-ignore lint/suspicious/noAssignInExpressions: scan loop
  while (protectNostr && (m = nostrRe.exec(text)) !== null) {
    ranges.push({ start: m.index, end: m.index + m[0].length });
  }

  // Email-like tokens: local@domain.tld — the `@` is not a mention trigger.
  const emailRe = /[^\s@()*_|\\[\]]+@[^\s@()*_|\\[\]]+\.[^\s@()*_|\\[\]\s]+/g;
  // biome-ignore lint/suspicious/noAssignInExpressions: scan loop
  while ((m = emailRe.exec(text)) !== null) {
    ranges.push({ start: m.index, end: m.index + m[0].length });
  }

  // Backslash-escaped @ signs: \@ is an escaped literal, not a mention trigger.
  const escapedAtRe = /\\@/g;
  // biome-ignore lint/suspicious/noAssignInExpressions: scan loop
  while ((m = escapedAtRe.exec(text)) !== null) {
    ranges.push({ start: m.index, end: m.index + 2 });
  }

  return ranges;
}

/** True if the half-open interval [start, end) overlaps any range in `set`. */
function overlapsRanges(start: number, end: number, set: Range[]): boolean {
  for (const r of set) {
    if (start < r.end && end > r.start) return true;
  }
  return false;
}

/**
 * Returns true when `ch` is a character that may validly precede an @mention
 * in Buzz prose: start-of-string, whitespace, `(`, or a Markdown/spoiler
 * delimiter (`*`, `_`, `|`).
 */
function isValidMentionPrecursor(ch: string | undefined): boolean {
  if (ch === undefined) return true;
  return (
    ch === " " ||
    ch === "\t" ||
    ch === "\n" ||
    ch === "\r" ||
    ch === "(" ||
    ch === "*" ||
    ch === "_" ||
    ch === "|"
  );
}

/**
 * Returns true when `ch` is a character that may validly follow an @mention
 * in Buzz prose: end-of-string, whitespace, or common punctuation/delimiters.
 */
function isValidMentionSuccessor(ch: string | undefined): boolean {
  if (ch === undefined) return true;
  return (
    ch === " " ||
    ch === "\t" ||
    ch === "\n" ||
    ch === "\r" ||
    ch === "," ||
    ch === ";" ||
    ch === "." ||
    ch === "!" ||
    ch === "?" ||
    ch === ":" ||
    ch === ")" ||
    ch === "]" ||
    ch === "}" ||
    ch === "*" ||
    ch === "_" ||
    ch === "|"
  );
}

/**
 * Replace every resolved @mention in `text` with its canonical NIP-27
 * `nostr:npub1…` reference. `mentionMap` maps display names (exactly as the
 * user typed them after autocomplete resolution) to their replacement strings
 * (e.g. `"Alice" → "nostr:npub1abc…"`).
 *
 * Protected zones — code spans, fenced and indented blocks, Markdown link
 * destinations, existing `nostr:` URIs, email-like tokens, and
 * backslash-escaped `@` signs — are never modified.  Longer display names
 * take priority over shorter ones so a match for "Alice Smith" prevents the
 * prefix "Alice" from matching separately.  The operation is idempotent:
 * already-encoded references land in the protected set and are not re-encoded.
 */
export function substituteResolvedMentions(
  text: string,
  mentionMap: ReadonlyMap<string, string>,
): string {
  if (mentionMap.size === 0) return text;

  const protectedRanges = buildProtectedRanges(text);

  // Longer names match before shorter ones to avoid partial substitution
  // of a shorter name that is a prefix of a longer resolved name.
  const sortedEntries = [...mentionMap.entries()].sort(
    ([a], [b]) => b.length - a.length,
  );

  type Edit = { start: number; end: number; replacement: string };
  const edits: Edit[] = [];
  // Ranges already claimed by a longer-name match; prevents re-substitution
  // of their interior by a shorter name.
  const claimedRanges: Range[] = [];

  for (const [name, replacement] of sortedEntries) {
    const trimmedName = name.trim();
    if (!trimmedName) continue;
    const nameLower = trimmedName.toLowerCase();
    const nameLen = trimmedName.length;

    let pos = 0;
    while (pos < text.length) {
      const atIdx = text.indexOf("@", pos);
      if (atIdx === -1) break;

      // Skip @ signs that fall inside a protected range.
      if (overlapsRanges(atIdx, atIdx + 1, protectedRanges)) {
        pos = atIdx + 1;
        continue;
      }

      // Word-boundary: the character before @ must be a valid precursor.
      const precursor = atIdx > 0 ? text[atIdx - 1] : undefined;
      if (!isValidMentionPrecursor(precursor)) {
        pos = atIdx + 1;
        continue;
      }

      // Case-insensitive name match immediately after the @.
      if (
        text.slice(atIdx + 1, atIdx + 1 + nameLen).toLowerCase() !== nameLower
      ) {
        pos = atIdx + 1;
        continue;
      }

      const occEnd = atIdx + 1 + nameLen;

      // Word-boundary: the character after the name must be a valid successor.
      const successor = occEnd < text.length ? text[occEnd] : undefined;
      if (!isValidMentionSuccessor(successor)) {
        pos = atIdx + 1;
        continue;
      }

      // Reject occurrences that span into a protected or already-claimed range.
      if (
        overlapsRanges(atIdx, occEnd, protectedRanges) ||
        overlapsRanges(atIdx, occEnd, claimedRanges)
      ) {
        pos = occEnd;
        continue;
      }

      edits.push({ start: atIdx, end: occEnd, replacement });
      claimedRanges.push({ start: atIdx, end: occEnd });
      pos = occEnd;
    }
  }

  if (edits.length === 0) return text;

  // Apply right-to-left so earlier offsets remain valid after each splice.
  edits.sort((a, b) => b.start - a.start);
  let result = text;
  for (const edit of edits) {
    result =
      result.slice(0, edit.start) + edit.replacement + result.slice(edit.end);
  }
  return result;
}

/**
 * Decode a `nostr:npub1…` or `nostr:nprofile1…` URI to a lowercase hex
 * public key.  Returns `null` for non-profile NIP-21 entities (nevent, note,
 * naddr, …), malformed bech32 strings, or URIs that do not start with
 * `nostr:`.
 */
export function decodeNostrProfilePubkey(uri: string): string | null {
  if (!uri.startsWith("nostr:")) return null;
  const encoded = uri.slice(6);
  if (!encoded.startsWith("npub1") && !encoded.startsWith("nprofile1")) {
    // Non-profile NIP-21 entity — leave unchanged.
    return null;
  }
  try {
    const decoded = decode(encoded);
    if (decoded.type === "npub") {
      return typeof decoded.data === "string"
        ? decoded.data.toLowerCase()
        : null;
    }
    if (decoded.type === "nprofile") {
      const data = decoded.data as { pubkey: string };
      return typeof data.pubkey === "string" ? data.pubkey.toLowerCase() : null;
    }
    return null;
  } catch {
    return null;
  }
}

/**
 * Replace `nostr:npub1…` / `nostr:nprofile1…` profile references in prose
 * with `@<displayName>` mention chips, using the current identity known to
 * the caller.  Code spans, fenced/indented blocks, and Markdown link
 * destinations are left unchanged.  Non-profile NIP-21 entities and malformed
 * references remain as plain text.
 *
 * Returns:
 * - `body`: the transformed text, with each valid profile URI replaced by
 *   `@<displayName>` so the Markdown renderer treats it as a mention chip.
 * - `nameToHexPubkey`: a map from every substituted display name to the
 *   corresponding lowercase hex public key, ready to be merged into the
 *   renderer's `mentionPubkeysByName` so chips resolve to profile popovers
 *   and open the correct decoded public key on click.
 */
export function materializeInboundProfiles(
  text: string,
  getDisplayName: (hexPubkey: string) => string | null,
): { body: string; nameToHexPubkey: Map<string, string> } {
  const nameToHexPubkey = new Map<string, string>();

  // Match nostr:npub1... and nostr:nprofile1... tokens anywhere in prose.
  const nostrProfileRe = /nostr:(?:npub1|nprofile1)[a-zA-Z0-9]+/g;
  const protectedRanges = buildProtectedRanges(text, false);

  type Edit = { start: number; end: number; replacement: string };
  const edits: Edit[] = [];
  let m: RegExpExecArray | null;

  // biome-ignore lint/suspicious/noAssignInExpressions: scan loop
  while ((m = nostrProfileRe.exec(text)) !== null) {
    const start = m.index;
    const end = m.index + m[0].length;

    // Skip references that appear inside code spans or link destinations.
    if (overlapsRanges(start, end, protectedRanges)) continue;

    const pubkey = decodeNostrProfilePubkey(m[0]);
    if (!pubkey) continue;

    const displayName = getDisplayName(pubkey);
    if (!displayName) continue;

    // Last display name wins when the same pubkey appears multiple times;
    // all occurrences use the same replacement string so this is safe.
    nameToHexPubkey.set(displayName, pubkey);
    edits.push({ start, end, replacement: `@${displayName}` });
  }

  if (edits.length === 0) return { body: text, nameToHexPubkey };

  // Apply right-to-left to preserve earlier character offsets.
  edits.sort((a, b) => b.start - a.start);
  let body = text;
  for (const edit of edits) {
    body = body.slice(0, edit.start) + edit.replacement + body.slice(edit.end);
  }
  return { body, nameToHexPubkey };
}

/**
 * Extract the set of lowercase hex public keys that are referenced by
 * `nostr:npub1…` or `nostr:nprofile1…` tokens in prose text.  Tokens that
 * appear inside code spans, fenced/indented code blocks, or Markdown link
 * destinations are excluded.  Duplicates are removed.  This is the companion
 * to `materializeInboundProfiles`: call it first to know which profiles to
 * hydrate, then call `materializeInboundProfiles` once the profile data is
 * available.
 */
export function extractNostrProfilePubkeys(text: string): string[] {
  const nostrProfileRe = /nostr:(?:npub1|nprofile1)[a-zA-Z0-9]+/g;
  const protectedRanges = buildProtectedRanges(text, false);
  const pubkeys: string[] = [];
  let m: RegExpExecArray | null;
  // biome-ignore lint/suspicious/noAssignInExpressions: scan loop
  while ((m = nostrProfileRe.exec(text)) !== null) {
    const start = m.index;
    const end = m.index + m[0].length;
    if (overlapsRanges(start, end, protectedRanges)) continue;
    const pubkey = decodeNostrProfilePubkey(m[0]);
    if (pubkey && !pubkeys.includes(pubkey)) pubkeys.push(pubkey);
  }
  return pubkeys;
}
