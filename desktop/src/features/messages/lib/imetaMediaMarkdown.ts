/**
 * Helpers for round-tripping NIP-92 imeta attachments through the message
 * editor.
 *
 * Background: edit events (kind 40003) carry only the new `content`; imeta
 * tags live on the original event. The renderer overlays the edit body onto
 * the original event but `markdown.tsx` only renders <img>/<video> for URLs
 * literally present in the body.
 *
 * The composer's edit mode now manages attachments as first-class state
 * (mirrors the send path):
 *
 *   - on edit-load, seed the composer's `pendingImeta` with the original
 *     event's imeta entries (full BlobDescriptor shape, so the send-path
 *     mediaTags builder works unchanged); strip any matching trailing
 *     `![image|video](url)` lines from the body so the user only sees text;
 *   - on submit, pass `mediaTags` (built from the current `pendingImeta`)
 *     alongside the edited content so the edit event carries a full new
 *     imeta tag set;
 *   - the receiver overlays the edit's imeta tags onto the rendered message
 *     (`formatTimelineMessages`).
 *
 * `ImetaMedia` is exactly the `BlobDescriptor` shape so it plugs into
 * `setPendingImeta` directly. `uploaded` isn't carried in imeta tags, so
 * `imetaMediaFromTags` zero-fills it (no consumer reads the value today).
 */

import type { BlobDescriptor } from "@/shared/api/tauri";
import { parseImetaTags } from "./parseImeta";

export type ImetaMedia = BlobDescriptor;

/**
 * Project a Nostr event's imeta tags into the `BlobDescriptor[]` shape the
 * composer's media state uses. Preserves tag order.
 *
 * Falls back to `image/jpeg` when an entry is missing `m` (legacy events).
 * The `uploaded` field isn't transmitted in imeta tags — set to 0 since no
 * consumer reads it.
 *
 * Projection ceiling: NIP-92 also defines `alt`, `fallback`, and `service`
 * fields that `BlobDescriptor` doesn't carry. We drop them on edit-load,
 * which means an edit will silently strip those fields from the saved tag
 * set. In practice this only fires on cross-client edits today (our send
 * path doesn't emit them), so the data loss is bounded. If/when those
 * fields become first-class in the composer, widen `BlobDescriptor`
 * (or split `ImetaMedia` from it) and pass them through here.
 */
export function imetaMediaFromTags(
  tags: ReadonlyArray<ReadonlyArray<string>> | undefined,
): ImetaMedia[] {
  if (!tags || tags.length === 0) return [];
  const entries = parseImetaTags(tags as string[][]);
  const out: ImetaMedia[] = [];
  for (const entry of entries.values()) {
    if (!entry.url) continue;
    out.push({
      url: entry.url,
      type: entry.m ?? "image/jpeg",
      sha256: entry.x ?? "",
      size: entry.size ?? 0,
      uploaded: 0,
      ...(entry.dim ? { dim: entry.dim } : {}),
      ...(entry.blurhash ? { blurhash: entry.blurhash } : {}),
      ...(entry.thumb ? { thumb: entry.thumb } : {}),
      ...(entry.duration != null ? { duration: entry.duration } : {}),
      ...(entry.image ? { image: entry.image } : {}),
      ...(entry.filename ? { filename: entry.filename } : {}),
    });
  }
  return out;
}

/**
 * Build the imeta tag set for an outbound event from a list of attachments.
 * Shared by the send path (initial post) and the edit path (full new tag set
 * on the edit event), so the two stay perfectly symmetric.
 *
 * `url` and `m` are always emitted (NIP-92's only de-facto required fields;
 * `m` carries a fallback in `imetaMediaFromTags`). All other fields are
 * conditional — including `x` and `size` — because legacy and cross-client
 * imeta entries can land without a sha256 or size, and our relay validator
 * rejects literal `"x "` / `"size 0"` empties. NIP-92 itself treats every
 * field except `url` as optional, so dropping them is spec-clean.
 */
export function buildImetaTags(
  imetaMedia: ReadonlyArray<ImetaMedia>,
): string[][] {
  return imetaMedia.map((d) => [
    "imeta",
    `url ${d.url}`,
    `m ${d.type}`,
    ...(d.sha256 ? [`x ${d.sha256}`] : []),
    ...(typeof d.size === "number" && d.size > 0 ? [`size ${d.size}`] : []),
    ...(d.dim ? [`dim ${d.dim}`] : []),
    ...(d.blurhash ? [`blurhash ${d.blurhash}`] : []),
    ...(d.thumb ? [`thumb ${d.thumb}`] : []),
    ...(d.duration != null ? [`duration ${d.duration}`] : []),
    ...(d.image ? [`image ${d.image}`] : []),
    ...(d.filename ? [`filename ${d.filename}`] : []),
  ]);
}

const MEDIA_LINE_RE =
  /^(?:\|\|)?!\[(?:image|video)\]\(([^)\s]+)\)(?:\|\|)?\s*$/;
const SPOILERED_MEDIA_LINE_RE =
  /^\|\|!\[(?:image|video)\]\(([^)\s]+)\)\|\|\s*$/;
const BLOCK_SPOILER_DELIMITER_RE = /^\s*\|\|\s*$/;
/**
 * Matches a generic file-attachment line `[label](url)` (no leading `!`, so it's
 * a link not an image). The label can contain spaces and backslash-escaped
 * brackets (e.g. `a\]`); the URL must be paren- and space-free. Used to strip
 * file attachments from the body in edit mode.
 */
const FILE_LINE_RE = /^\[(?:\\.|[^\]\\])*\]\(([^)\s]+)\)\s*$/;

function findTrailingBlockSpoilerMediaStart(
  lines: string[],
  closingDelimiterIndex: number,
  urls: ReadonlySet<string>,
): number | null {
  let index = closingDelimiterIndex - 1;
  let hasMatchingMedia = false;

  while (index >= 0) {
    const line = lines[index];
    if (line.trim() === "") {
      index -= 1;
      continue;
    }

    if (BLOCK_SPOILER_DELIMITER_RE.test(line)) {
      return hasMatchingMedia ? index : null;
    }

    const match = line.match(MEDIA_LINE_RE);
    if (!match || !urls.has(match[1])) return null;

    hasMatchingMedia = true;
    index -= 1;
  }

  return null;
}

/**
 * Remove trailing `![image|video](url)` lines whose URL matches an entry in
 * `imetaMedia`. Stops at the first non-matching/non-blank line so attachments
 * that have been moved or interleaved with text are left alone (the composer
 * only ever produces trailing lines, but defending against shape drift is
 * cheap).
 */
export function stripImetaMediaLines(
  body: string,
  imetaMedia: ReadonlyArray<ImetaMedia>,
): string {
  if (imetaMedia.length === 0) return body;
  const urls = new Set(imetaMedia.map((m) => m.url));
  const lines = body.split("\n");

  let end = lines.length;
  while (end > 0) {
    const line = lines[end - 1];
    if (line.trim() === "") {
      end -= 1;
      continue;
    }
    if (BLOCK_SPOILER_DELIMITER_RE.test(line)) {
      const start = findTrailingBlockSpoilerMediaStart(lines, end - 1, urls);
      if (start != null) {
        end = start;
        continue;
      }
    }
    const match = line.match(MEDIA_LINE_RE) ?? line.match(FILE_LINE_RE);
    if (match && urls.has(match[1])) {
      end -= 1;
      continue;
    }
    break;
  }

  return lines.slice(0, end).join("\n").replace(/\s+$/, "");
}

export function findSpoileredImetaMediaUrls(
  body: string,
  imetaMedia: ReadonlyArray<ImetaMedia>,
): Set<string> {
  if (imetaMedia.length === 0) return new Set();

  const urls = new Set(imetaMedia.map((m) => m.url));
  const spoileredUrls = new Set<string>();
  const lines = body.split("\n");
  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index];
    const match = line.match(SPOILERED_MEDIA_LINE_RE);
    if (match && urls.has(match[1])) {
      spoileredUrls.add(match[1]);
      continue;
    }

    if (!BLOCK_SPOILER_DELIMITER_RE.test(line)) continue;

    const blockSpoileredUrls = new Set<string>();
    let closingDelimiterIndex = -1;
    for (
      let blockIndex = index + 1;
      blockIndex < lines.length;
      blockIndex += 1
    ) {
      const blockLine = lines[blockIndex];
      if (BLOCK_SPOILER_DELIMITER_RE.test(blockLine)) {
        closingDelimiterIndex = blockIndex;
        break;
      }

      const blockMatch = blockLine.match(MEDIA_LINE_RE);
      if (blockMatch && urls.has(blockMatch[1])) {
        blockSpoileredUrls.add(blockMatch[1]);
      }
    }

    if (closingDelimiterIndex !== -1) {
      for (const url of blockSpoileredUrls) spoileredUrls.add(url);
      index = closingDelimiterIndex;
    }
  }
  return spoileredUrls;
}

/**
 * Format a single imeta entry as a leading-newline markdown line.
 *
 * Images and video use `![image|video](url)` so the renderer draws them inline.
 * Generic files use a plain `[filename](url)` link — the renderer recognises the
 * href as a local media blob with a non-media MIME and upgrades it to a file
 * card. Mime-driven so the form is correct regardless of URL suffix.
 */
export function formatImetaMediaLine(
  { url, type, filename }: ImetaMedia,
  options: { spoiler?: boolean } = {},
): string {
  if (type.startsWith("video/")) {
    const line = `![video](${url})`;
    return options.spoiler ? `\n||${line}||` : `\n${line}`;
  }
  if (type.startsWith("image/")) {
    const line = `![image](${url})`;
    return options.spoiler ? `\n||${line}||` : `\n${line}`;
  }
  // Generic file: plain link, label is the original filename (fallback to url tail).
  const label = filename || url.split("/").pop() || "file";
  // Escape markdown link-label metacharacters so filenames containing `[`, `]`,
  // or `\` (e.g. `a].pdf`) still render as a FileCard with the correct label
  // rather than breaking the link or mangling the visible text.
  const escaped = label.replace(/[\\[\]]/g, "\\$&");
  return `\n[${escaped}](${url})`;
}

/**
 * Build the body + tags pair for an outgoing message (initial send or
 * edit). Appends `![image|video](url)` markdown lines for each attachment
 * to the body so the renderer (which keys on URLs literally present in
 * the content) draws them, and returns the matching imeta tag set.
 *
 * Returns `mediaTags: undefined` when there are no attachments. Callers
 * that need an explicit "wipe attachments" signal (the edit path, where
 * `[]` instructs the receiver overlay to drop existing imeta) should
 * coerce with `?? []`.
 */
export function buildOutgoingMessage(
  body: string,
  pendingImeta: ReadonlyArray<ImetaMedia>,
  spoileredMediaUrls: ReadonlySet<string> = new Set(),
): { content: string; mediaTags: string[][] | undefined } {
  let content = body;
  for (const d of pendingImeta) {
    content += formatImetaMediaLine(d, {
      spoiler: spoileredMediaUrls.has(d.url),
    });
  }
  const mediaTags =
    pendingImeta.length > 0 ? buildImetaTags(pendingImeta) : undefined;
  return { content, mediaTags };
}

/**
 * Merge optional imeta media tags with NIP-30 custom-emoji tags into the final
 * outgoing tag set. Returns `undefined` when there are no tags of either kind
 * (the publish path treats `undefined` as "no extra tags").
 */
export function mergeOutgoingTags(
  mediaTags: string[][] | undefined,
  emojiTags: string[][],
): string[][] | undefined {
  if (!mediaTags && emojiTags.length === 0) return undefined;
  return [...(mediaTags ?? []), ...emojiTags];
}

/**
 * Inverse of `mergeOutgoingTags`: split a merged outgoing tag set back into
 * imeta media tags, NIP-30 `["emoji", ...]` tags, and reference-only mention
 * tags, so the send path can route each to its own validated Tauri arg. Emoji
 * and mention tags must never ride the imeta-only `media` channel (its guard
 * rejects any non-imeta prefix). Any other prefix stays with `mediaTags` — the
 * imeta guard will reject it, which is the intended injection defense.
 */
export function splitOutgoingTags(tags: string[][] | undefined): {
  mediaTags: string[][];
  emojiTags: string[][];
  mentionTags: string[][];
  clientTags: string[][];
} {
  const mediaTags: string[][] = [];
  const emojiTags: string[][] = [];
  const mentionTags: string[][] = [];
  const clientTags: string[][] = [];
  for (const tag of tags ?? []) {
    if (tag[0] === "emoji") {
      emojiTags.push(tag);
    } else if (tag[0] === "mention") {
      mentionTags.push(tag);
    } else if (tag[0] === "client") {
      // Whitelisted client marker tags (e.g. automation prompts) — these
      // must NOT ride the imeta-only media channel, whose builder rejects
      // any non-imeta prefix and fails the whole send.
      clientTags.push(tag);
    } else {
      mediaTags.push(tag);
    }
  }
  return { mediaTags, emojiTags, mentionTags, clientTags };
}
