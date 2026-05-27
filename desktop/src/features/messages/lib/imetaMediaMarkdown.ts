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
  ]);
}

const MEDIA_LINE_RE = /^!\[(?:image|video)\]\(([^)\s]+)\)\s*$/;

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
    const match = line.match(MEDIA_LINE_RE);
    if (match && urls.has(match[1])) {
      end -= 1;
      continue;
    }
    break;
  }

  return lines.slice(0, end).join("\n").replace(/\s+$/, "");
}

/**
 * Format a single imeta entry as a leading-newline markdown line. Mime-driven
 * so the alt label is correct regardless of URL suffix.
 */
export function formatImetaMediaLine({ url, type }: ImetaMedia): string {
  const isVideo = type.startsWith("video/");
  return isVideo ? `\n![video](${url})` : `\n![image](${url})`;
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
): { content: string; mediaTags: string[][] | undefined } {
  let content = body;
  for (const d of pendingImeta) content += formatImetaMediaLine(d);
  const mediaTags =
    pendingImeta.length > 0 ? buildImetaTags(pendingImeta) : undefined;
  return { content, mediaTags };
}
