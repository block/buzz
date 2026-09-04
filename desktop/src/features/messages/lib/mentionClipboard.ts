import { formatMentionDisplayLabel } from "@/shared/lib/mentionDisplay";
import { truncateInlineChipLabel } from "@/shared/ui/mentionChip";

import { getMentionOffsets } from "./hasMention";

/**
 * Dual-flavor clipboard support for mentions.
 *
 * Every Buzz copy writes two flavors:
 *  - `text/plain` — what a human should read anywhere: sigils restored, no
 *    pubkeys. This is what external apps (TextEdit, Slack, …) receive.
 *  - `text/html` — the same content, with each mention wrapped in a span
 *    carrying its pubkey. Pasting back into a Buzz composer harvests those
 *    records, registers `name → pubkey` with the mention machinery, and the
 *    chip re-lights with the exact tagged identity.
 *
 * The wrapper marker declares what the *plain* flavor holds so paste knows
 * which content path to use — see `BuzzCopyKind`.
 */

/** Marks clipboard HTML that Buzz produced, and what its plain flavor holds. */
export const BUZZ_COPY_ATTRIBUTE = "data-buzz-copy";
/** 64-hex pubkey of the mentioned identity. */
export const MENTION_PUBKEY_ATTRIBUTE = "data-mention-pubkey";
/** `human` or `agent` — decides which highlight the re-lit chip gets. */
export const MENTION_KIND_ATTRIBUTE = "data-mention-kind";
/** Full mention label, so a partially selected chip is detectable. */
export const MENTION_LABEL_ATTRIBUTE = "data-mention-label";
/** Full channel-reference label, same partial-selection role as above. */
export const CHANNEL_LABEL_ATTRIBUTE = "data-channel-label";

/**
 * Canonical form for comparing two spellings of one mention label.
 *
 * Tolerates what a label picks up in transit and nothing more: a pasteboard
 * round trip swaps spaces for U+00A0, markup gains padding, and mention
 * resolution is case-insensitive end to end. Every clipboard comparison of
 * two labels goes through here, so "the same name" means one thing across the
 * copy-side chip classifier and the paste-side trust check.
 */
export function canonicalMentionLabel(value: string): string {
  return value
    .replace(/\u00a0/g, " ")
    .trim()
    .toLowerCase();
}

/** How a chip's copied text relates to the full label it declares. */
export type ChipTextMatch = "full" | "truncated" | "fragment";

/**
 * Classify a chip's copied text against the label its attributes declare.
 *
 * Both clipboard sides ask this one question — the copy handler deciding
 * whether to write a sigil back, and the paste normalizer deciding whether to
 * keep one — so they cannot drift apart on what counts as a whole chip.
 *
 * - `full` — the text carries the whole label, allowing for what a legitimate
 *   chip picks up in transit: Buzz's copy handlers write the sigil into the
 *   text, `buildMentionSpanHtml` keeps the author's casing over the label's,
 *   and a pasteboard round trip can swap spaces for U+00A0.
 * - `truncated` — the text is the ellipsized form `truncateInlineChipLabel`
 *   renders for a label past the inline-chip cap. A *fully* selected long chip
 *   carries this rather than the label, so treating it as a fragment would
 *   strip the identity off every copy of a long channel reference and drop the
 *   whole selection back to the browser's dead-text default.
 * - `fragment` — anything else: the slice a boundary-crossing selection leaves
 *   behind. It must neither regain a sigil nor keep an identity, so that a
 *   paste can never bind a real pubkey to a partial name.
 *
 * Deliberately not an equality test against the rendered text: a chip that
 * grows any text of its own (a badge, a glyph's text fallback) must not
 * silently reclassify every chip as a fragment. Text a chip adds *beyond* its
 * label still reads as `fragment` — the safe direction, since that costs a
 * copy its identity rather than inventing one.
 */
export function matchChipTextToLabel(
  text: string,
  label: string,
  sigil: "@" | "#",
  pubkey?: string,
): ChipTextMatch {
  const body = canonicalMentionLabel(text);
  const matches = (form: string) => body === form || body === `${sigil}${form}`;
  if (matches(canonicalMentionLabel(label))) return "full";
  // Derived from the helper the chips render with, so the tolerated form
  // cannot drift from what a fully selected capped chip actually carries.
  const truncated = truncateInlineChipLabel(label);
  if (truncated !== label && matches(canonicalMentionLabel(truncated))) {
    return "truncated";
  }
  // Read-only mentions abbreviate a bound key, never the identity carried by
  // the clipboard. Restore the full literal label only for the whole display.
  const compact =
    sigil === "@" ? formatMentionDisplayLabel(label, pubkey) : label;
  if (compact !== label && matches(canonicalMentionLabel(compact))) {
    return "truncated";
  }
  return "fragment";
}

/**
 * What the `text/plain` flavor of a Buzz copy contains.
 *
 * - `markdown` — Markdown source (copy-message, composer copy/cut). Paste
 *   inserts the plain flavor so TipTap's Markdown parsing behaves exactly as
 *   it does for a plain-text paste.
 * - `rich` — rendered timeline HTML. Paste keeps the HTML content path.
 */
export type BuzzCopyKind = "markdown" | "rich";

/** A `name → pubkey` pair the composer can register on paste. */
export type MentionIdentity = {
  label: string;
  pubkey: string;
  isAgent: boolean;
};

/**
 * Clipboard HTML is untrusted input — a foreign app can put anything on the
 * pasteboard. Bound the record count and label length, and require a
 * well-formed pubkey, before any of it can become an outbound `p` tag.
 */
const MAX_MENTION_RECORDS = 50;
const MAX_MENTION_LABEL_LENGTH = 200;

const HTML_ESCAPES: Record<string, string> = {
  "&": "&amp;",
  "<": "&lt;",
  ">": "&gt;",
  '"': "&quot;",
  "'": "&#39;",
};

const HTML_UNESCAPES: Record<string, string> = {
  "&amp;": "&",
  "&lt;": "<",
  "&gt;": ">",
  "&quot;": '"',
  "&#39;": "'",
};

/** Escape a value for interpolation into clipboard HTML text or attributes. */
export function escapeClipboardHtml(value: string): string {
  return value.replace(/[&<>"']/g, (char) => HTML_ESCAPES[char] ?? char);
}

function unescapeClipboardHtml(value: string): string {
  return value.replace(
    /&(?:amp|lt|gt|quot|#39);/g,
    (entity) => HTML_UNESCAPES[entity] ?? entity,
  );
}

function isMentionPubkey(value: string): boolean {
  return /^[0-9a-f]{64}$/.test(value);
}

/** Wrap one mention occurrence so paste can recover its exact identity. */
export function buildMentionSpanHtml({
  identity,
  text,
}: {
  identity: MentionIdentity;
  /** The matched `@Name` run exactly as it appears in the plain flavor. */
  text: string;
}): string {
  return [
    `<span data-mention=""`,
    ` ${MENTION_PUBKEY_ATTRIBUTE}="${escapeClipboardHtml(identity.pubkey)}"`,
    ` ${MENTION_KIND_ATTRIBUTE}="${identity.isAgent ? "agent" : "human"}"`,
    ` ${MENTION_LABEL_ATTRIBUTE}="${escapeClipboardHtml(identity.label)}"`,
    `>${escapeClipboardHtml(text)}</span>`,
  ].join("");
}

type MentionMatch = {
  offset: number;
  length: number;
  identity: MentionIdentity;
};

/**
 * Locate every mention of a known identity in `text`.
 *
 * Longest-match-wins at each offset, mirroring `extractMentionPubkeys` so the
 * span we write and the pubkey the send path recovers can't disagree when one
 * display name prefixes another ("Alex" vs "Alex Kim").
 */
function findMentionMatches(
  text: string,
  identities: readonly MentionIdentity[],
): MentionMatch[] {
  const byOffset = new Map<number, MentionMatch>();

  for (const identity of identities) {
    const label = identity.label.trim();
    // Callers hand over whatever their own lookup holds; the clipboard is the
    // wire format here, so it always carries a canonical lowercase pubkey.
    const pubkey = identity.pubkey.trim().toLowerCase();
    if (!label || !isMentionPubkey(pubkey)) continue;
    // `@` + label; `getMentionOffsets` returns the offset of the sigil.
    const length = label.length + 1;
    for (const offset of getMentionOffsets(text, label)) {
      const existing = byOffset.get(offset);
      if (!existing || existing.length < length) {
        byOffset.set(offset, {
          offset,
          identity: { ...identity, label, pubkey },
          length,
        });
      }
    }
  }

  const matches = [...byOffset.values()].sort((a, b) => a.offset - b.offset);
  // A shorter name nested inside a longer one can match at a later offset
  // ("@Alex Kim" also matches "Kim" if someone is called that). The outer
  // match already carries an identity, so drop anything it covers.
  const disjoint: MentionMatch[] = [];
  let consumedTo = 0;
  for (const match of matches) {
    if (match.offset < consumedTo) continue;
    disjoint.push(match);
    consumedTo = match.offset + match.length;
  }
  return disjoint;
}

/**
 * Build the `text/html` identity sidecar for a plain-text (Markdown) body.
 *
 * Returns `null` when the body carries no known mention — callers use that to
 * leave ordinary copies on their default path rather than replacing the
 * clipboard's rich flavor for no gain.
 */
export function buildMentionClipboardHtml({
  text,
  identities,
  kind = "markdown",
}: {
  text: string;
  identities: readonly MentionIdentity[];
  kind?: BuzzCopyKind;
}): string | null {
  const matches = findMentionMatches(text, identities);
  if (matches.length === 0) return null;

  const parts: string[] = [];
  const pushText = (value: string) => {
    if (value) parts.push(escapeClipboardHtml(value).replace(/\n/g, "<br>"));
  };

  let cursor = 0;
  for (const match of matches) {
    pushText(text.slice(cursor, match.offset));
    parts.push(
      buildMentionSpanHtml({
        identity: match.identity,
        // Match casing as written, not the identity's canonical casing:
        // mention resolution is case-insensitive end to end.
        text: text.slice(match.offset, match.offset + match.length),
      }),
    );
    cursor = match.offset + match.length;
  }
  pushText(text.slice(cursor));

  return `<span ${BUZZ_COPY_ATTRIBUTE}="${kind}">${parts.join("")}</span>`;
}

/** The Buzz copy marker on clipboard HTML, or `null` for foreign HTML. */
export function getBuzzCopyKind(html: string): BuzzCopyKind | null {
  const match = html.match(
    new RegExp(
      `\\b${BUZZ_COPY_ATTRIBUTE}\\s*=\\s*["'](markdown|rich)["']`,
      "i",
    ),
  );
  return (match?.[1] as BuzzCopyKind | undefined) ?? null;
}

function readAttribute(tag: string, name: string): string | null {
  const match = tag.match(
    new RegExp(`\\b${name}\\s*=\\s*(?:"([^"]*)"|'([^']*)')`, "i"),
  );
  const value = match?.[1] ?? match?.[2];
  return value === undefined ? null : unescapeClipboardHtml(value);
}

/**
 * Recover the `label → pubkey` records a Buzz copy embedded in its HTML.
 *
 * Reads the label from `data-mention-label` rather than the element's text so
 * the result never depends on how a pasteboard round-trip reformatted the
 * markup. Malformed or oversized records are dropped, not repaired.
 */
export function parseMentionClipboardRecords(html: string): MentionIdentity[] {
  const tagPattern = new RegExp(
    `<[a-zA-Z][^>]*\\b${MENTION_PUBKEY_ATTRIBUTE}\\s*=[^>]*>`,
    "g",
  );
  const records: MentionIdentity[] = [];
  const seen = new Set<string>();

  for (const [tag] of html.matchAll(tagPattern)) {
    if (records.length >= MAX_MENTION_RECORDS) break;
    const pubkey = readAttribute(tag, MENTION_PUBKEY_ATTRIBUTE)
      ?.trim()
      .toLowerCase();
    const label = readAttribute(tag, MENTION_LABEL_ATTRIBUTE)?.trim();
    if (
      !pubkey ||
      !isMentionPubkey(pubkey) ||
      !label ||
      label.length > MAX_MENTION_LABEL_LENGTH
    ) {
      continue;
    }
    const key = `${label.toLowerCase()} ${pubkey}`;
    if (seen.has(key)) continue;
    seen.add(key);
    records.push({
      label,
      pubkey,
      isAgent: readAttribute(tag, MENTION_KIND_ATTRIBUTE) === "agent",
    });
  }

  return records;
}

/**
 * Keep only the records the paste actually shows.
 *
 * A record binds a display name for the rest of the composer session, well
 * past the paste that carried it — so an empty or hidden
 * `<span data-mention-pubkey … data-mention-label="Jane Doe">` on any copied
 * page would silently rebind a real member's name, and a later hand-typed
 * `@Jane Doe` would chip-light convincingly against the attacker's pubkey.
 * Requiring the label to be mentioned in the inserted content keeps every
 * binding visible to the user who accepted it.
 *
 * `getMentionOffsets` is the same matcher the send-time extractor uses, so a
 * dropped record is one that could not have tagged anyone from this content
 * anyway — code spans and fences excluded on the same terms.
 */
export function selectVisibleMentionIdentities(
  records: readonly MentionIdentity[],
  text: string,
): MentionIdentity[] {
  return records.filter(
    (record) => getMentionOffsets(text, record.label).length > 0,
  );
}

/**
 * Narrow copied records to the pairs trusted Buzz state vouches for.
 *
 * Must resolve to a subset of what it was handed; callers enforce that rather
 * than assume it, so the seam cannot widen into "the verifier decides what
 * gets bound". See `useVerifyMentionIdentities` for the implementation and
 * `mentionIdentityTrust` for why the check exists.
 */
export type VerifyMentionIdentities = (
  records: readonly MentionIdentity[],
) => Promise<readonly MentionIdentity[]>;

/** Case-insensitive identity of a `label → pubkey` pair. */
function mentionIdentityKey(identity: MentionIdentity): string {
  return `${canonicalMentionLabel(identity.label)} ${identity.pubkey.trim().toLowerCase()}`;
}

/**
 * Narrow already-visible records to the pairs the verifier vouches for.
 *
 * The result is filtered back down to what was asked about, so the verifier
 * stays a seam rather than an authority: a bug or a future implementation that
 * answers with a pair nobody copied cannot widen the paste.
 */
export async function selectVouchedMentionIdentities(
  visible: readonly MentionIdentity[],
  verifyMentionIdentities: VerifyMentionIdentities,
): Promise<MentionIdentity[]> {
  if (visible.length === 0) return [];
  const vouched = new Set(
    (await verifyMentionIdentities(visible)).map(mentionIdentityKey),
  );
  return visible.filter((record) => vouched.has(mentionIdentityKey(record)));
}

/**
 * The identities a paste is allowed to bind.
 *
 * Three conditions, all necessary. The clipboard has to *carry* the record;
 * the content the paste inserts has to *show* its label, so no binding
 * outlives a paste the user could not see; and trusted Buzz state has to
 * *vouch* for the pair, because a visible `@John Smith` beside an attacker's
 * key is visible either way.
 *
 * Binding is what makes a pasted multi-word name known to the mention
 * decorations *and* to the send-time extractor — so the chip re-lights and
 * the original pubkey survives the round trip. Everything dropped here stays
 * readable text that tags nobody.
 *
 * The two halves are separately exported because the binder needs the first
 * one *synchronously*, at paste time, to claim each label it is about to
 * verify — see `useMentionPasteBinding`.
 */
export async function selectBindableMentionIdentities({
  html,
  text,
  verifyMentionIdentities,
}: {
  /** Clipboard HTML holding the identity records — untrusted. */
  html: string;
  /** The text the paste inserts; a record unmentioned there is discarded. */
  text: string;
  verifyMentionIdentities: VerifyMentionIdentities;
}): Promise<MentionIdentity[]> {
  return selectVouchedMentionIdentities(
    selectVisibleMentionIdentities(parseMentionClipboardRecords(html), text),
    verifyMentionIdentities,
  );
}
