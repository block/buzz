/**
 * Links shared in a channel, treated as first-class entries in the Files tab
 * alongside uploaded files.
 *
 * Why this exists: increasingly the useful artefact in a channel is a link —
 * a Google Doc, a dashboard, a Drive file too large to upload — and the Files
 * tab could not see any of them. Worse, the version-chain machinery already
 * worked on anything in that list, because the supersedes tag
 * (`["e", "<older-event-id>", "", "supersedes"]`) points at an *event*, not at
 * a file. So making links entries also makes them versionable, with no new tag
 * and no relay change.
 *
 * Everything here is pure. Naming a link well ultimately wants the Drive API
 * (see `docs/google-drive-integration-spec.md`), but the fallbacks below are
 * good enough that a link row is never unreadable while that is unavailable.
 */

/**
 * Matches bare http(s) URLs in message content.
 *
 * Deliberately stops at whitespace and at the characters that habitually
 * terminate a URL in prose — a link at the end of a sentence must not swallow
 * the full stop, and one inside markdown link syntax must not swallow the
 * closing paren. Trailing punctuation that survives this is stripped below.
 */
const URL_PATTERN = /https?:\/\/[^\s<>"'`\]]+/gi;

/** Punctuation that is far more likely to be prose than part of the URL. */
const TRAILING_PUNCTUATION = /[.,;:!?)\]}'"]+$/;

/**
 * Path segments that carry no meaning as a name. `d` and `file` are Google's
 * routing prefixes; `view`/`edit`/`preview` are the trailing verb on almost
 * every Google and Notion URL; `index.html` names a directory, not a document.
 */
const NOISE_SEGMENTS = new Set([
  "d",
  "e",
  "edit",
  "file",
  "folders",
  "index.htm",
  "index.html",
  "preview",
  "u",
  "view",
]);

/** Below this, an id-looking segment is probably a real (short) slug. */
const OPAQUE_SEGMENT_MIN_LENGTH = 16;

/** Purely numeric segments this long are ids, not names. */
const OPAQUE_NUMERIC_MIN_LENGTH = 6;

/** Parse without throwing. Returns null for anything not a usable http(s) URL. */
function parseUrl(value) {
  if (typeof value !== "string" || value === "") return null;
  let parsed;
  try {
    parsed = new URL(value);
  } catch {
    return null;
  }
  if (parsed.protocol !== "http:" && parsed.protocol !== "https:") return null;
  if (!parsed.hostname) return null;
  return parsed;
}

/** Hostname without the `www.` that no one reads. */
export function linkHost(url) {
  const parsed = parseUrl(url);
  if (!parsed) return null;
  return parsed.hostname.replace(/^www\./i, "");
}

/**
 * True if a path segment is an opaque identifier rather than something a
 * person would recognise. Drive file ids, UUIDs and numeric record ids all
 * land here; `q3-report.pdf` and `Q3 Planning` do not.
 */
function isOpaqueSegment(segment) {
  if (NOISE_SEGMENTS.has(segment.toLowerCase())) return true;
  if (/^\d+$/.test(segment)) {
    return segment.length >= OPAQUE_NUMERIC_MIN_LENGTH;
  }
  // A name almost always contains a space, a dot, or several words. An id is
  // one long run of id-safe characters.
  if (!/^[A-Za-z0-9_-]+$/.test(segment)) return false;
  if (segment.length < OPAQUE_SEGMENT_MIN_LENGTH) return false;
  return true;
}

/**
 * Google's document surfaces, which are the ones most likely to be shared and
 * the ones whose URLs are pure opaque id. Naming the *kind* of thing is far
 * more use than `1a2B3c...`, which the spec calls out as unacceptable.
 */
function googleSurfaceLabel(parsed) {
  const host = parsed.hostname.replace(/^www\./i, "").toLowerCase();
  const path = parsed.pathname.toLowerCase();
  if (host === "docs.google.com") {
    if (path.startsWith("/document")) return "Google Doc";
    if (path.startsWith("/spreadsheets")) return "Google Sheet";
    if (path.startsWith("/presentation")) return "Google Slides";
    if (path.startsWith("/forms")) return "Google Form";
    if (path.startsWith("/drawings")) return "Google Drawing";
    return "Google Docs file";
  }
  if (host === "drive.google.com") {
    return path.includes("/folders")
      ? "Google Drive folder"
      : "Google Drive file";
  }
  if (host === "meet.google.com") return "Google Meet link";
  return null;
}

/**
 * Markdown link syntax: `[label](https://…)`.
 *
 * The label is whatever the sender chose to call the thing, which beats
 * anything derivable from the URL. It is also how Buzz names its own Drive
 * uploads — the composer posts `[Q3 Budget.xlsx](https://drive.google.com/…)`
 * so the Files tab can show the real filename without a Drive API call. See
 * `driveLinkMarkdown` in `features/messages/lib/driveUploadRouting.mjs`.
 */
const MARKDOWN_LINK_PATTERN = /\[([^\]\n]*)\]\((https?:\/\/[^\s)]+)\)/g;

/**
 * URL → the sender's own label for it, for every markdown link in `content`.
 *
 * First label wins when one URL is labelled twice in a message; an empty label
 * (`[](url)`) is ignored rather than becoming a blank row.
 */
export function extractMessageLinkLabels(content) {
  const text = typeof content === "string" ? content : "";
  const labels = new Map();
  for (const match of text.matchAll(MARKDOWN_LINK_PATTERN)) {
    const label = match[1].trim();
    const url = match[2];
    if (!label || !parseUrl(url)) continue;
    const key = normalizeLinkKey(url);
    if (key && !labels.has(key)) labels.set(key, label);
  }
  return labels;
}

/**
 * A display name for a link, derived from the URL alone.
 *
 * Falls back through: a known Google surface, the last meaningful path
 * segment, then the host. Never returns a bare opaque id, and never returns
 * an empty string.
 *
 * A sender-supplied markdown label outranks all of this and is applied by
 * `collectChannelLinkEntries`, not here — this function only knows the URL.
 */
export function readableLinkName(url) {
  const parsed = parseUrl(url);
  if (!parsed) return null;

  const google = googleSurfaceLabel(parsed);
  if (google) return google;

  const segments = parsed.pathname
    .split("/")
    .filter((segment) => segment !== "")
    .map((segment) => {
      try {
        return decodeURIComponent(segment);
      } catch {
        return segment; // malformed percent-encoding — take it verbatim
      }
    });

  for (let index = segments.length - 1; index >= 0; index -= 1) {
    const segment = segments[index];
    if (!isOpaqueSegment(segment)) return segment;
  }

  return parsed.hostname.replace(/^www\./i, "");
}

/**
 * Every distinct http(s) URL in a message's content, in the order it appears.
 *
 * Trailing prose punctuation is stripped, but only while the result still
 * parses — so `https://x.com/a.` loses the stop and `https://x.com/a.pdf`
 * keeps its extension.
 */
export function extractMessageLinks(content) {
  const text = typeof content === "string" ? content : "";
  const found = [];
  const seen = new Set();
  for (const match of text.matchAll(URL_PATTERN)) {
    let candidate = match[0];
    // Strip one trailing punctuation run, then verify. If stripping broke the
    // URL, keep the original — better a slightly long link than none.
    const trimmed = candidate.replace(TRAILING_PUNCTUATION, "");
    if (trimmed !== "" && parseUrl(trimmed)) candidate = trimmed;
    if (!parseUrl(candidate)) continue;
    if (seen.has(candidate)) continue;
    seen.add(candidate);
    found.push(candidate);
  }
  return found;
}

/**
 * The key two URLs must share to be considered the same link.
 *
 * Fragments and a trailing slash are display noise, and the host's case is
 * meaningless — but the query string is not, since it routinely selects the
 * document (`?id=`, `?gid=`). Returns null for anything unparseable.
 */
export function normalizeLinkKey(url) {
  const parsed = parseUrl(url);
  if (!parsed) return null;
  const host = parsed.hostname.replace(/^www\./i, "").toLowerCase();
  const path = parsed.pathname.replace(/\/+$/, "");
  return `${parsed.protocol}//${host}${path}${parsed.search}`;
}

/**
 * Collect the link entries for a channel.
 *
 * `messages` are the channel's non-deleted content events, each reduced to the
 * fields this needs. `excludedUrls` are URLs already represented as uploaded
 * files: the markdown renderer embeds an attachment's own URL in the message
 * body, so without this every upload would also produce a duplicate link row.
 *
 * One entry per unique link per channel, dated at its **earliest** appearance.
 * The same link pasted five times is one row, or the Files tab becomes a
 * transcript. Earliest rather than latest because that is when the thing
 * entered the channel, and because it gives a stable event id for a later
 * message to supersede.
 */
export function collectChannelLinkEntries({ messages, excludedUrls }) {
  const excluded = new Set();
  for (const url of excludedUrls ?? []) {
    const key = normalizeLinkKey(url);
    if (key) excluded.add(key);
  }

  /** normalized key -> the earliest message that carried it. */
  const earliest = new Map();

  for (const message of messages ?? []) {
    if (!message || typeof message.eventId !== "string") continue;
    const urls = extractMessageLinks(message.content);
    const labels = extractMessageLinkLabels(message.content);
    // A supersedes tag names one predecessor, so it can only be attributed to
    // a link when the message carries exactly one and no attachment of its
    // own. Otherwise two links would both claim to supersede the same file, or
    // a link would steal a tag that belongs to the file beside it.
    const claimsSupersedes = urls.length === 1 && !message.hasAttachment;
    for (const url of urls) {
      const key = normalizeLinkKey(url);
      if (!key || excluded.has(key)) continue;
      const existing = earliest.get(key);
      if (
        existing &&
        // Ties broken by event id so the result does not depend on the order
        // the relay happened to page events back in.
        (existing.createdAt < message.createdAt ||
          (existing.createdAt === message.createdAt &&
            existing.eventId <= message.eventId))
      ) {
        continue;
      }
      earliest.set(key, {
        ...message,
        claimsSupersedes,
        label: labels.get(key) ?? null,
        url,
      });
    }
  }

  const entries = [];
  for (const message of earliest.values()) {
    entries.push({
      kind: "link",
      eventId: message.eventId,
      uploadedBy: message.pubkey ?? "",
      uploadedAt: message.createdAt ?? 0,
      // The sender's own label first — it is the only source that knows what
      // the thing is actually called.
      filename: message.label ?? readableLinkName(message.url),
      sha256: null,
      size: null,
      mime: null,
      url: message.url,
      supersedes: message.claimsSupersedes
        ? (message.supersedes ?? null)
        : null,
      supersededBy: null,
    });
  }
  return entries;
}
