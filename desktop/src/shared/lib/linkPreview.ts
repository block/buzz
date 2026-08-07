import {
  buildIssueLink,
  buildPullRequestLink,
  buildRepoLink,
  isEntityLink,
  parseEntityLink,
  type ParsedEntityLink,
} from "./entityLink";

export type SupportedLinkPreviewKind =
  | "buzz-pull-request"
  | "buzz-issue"
  | "buzz-repository"
  | "github-pull-request"
  | "github-issue"
  | "github-repository"
  | "linear-issue"
  | "google-drive-file"
  | "google-drive-folder"
  | "google-docs-document"
  | "google-sheets-spreadsheet"
  | "google-slides-presentation";

export type SupportedLinkPreview = {
  kind: SupportedLinkPreviewKind;
  href: string;
  provider:
    | "Buzz"
    | "GitHub"
    | "Linear"
    | "Google Drive"
    | "Google Docs"
    | "Google Sheets"
    | "Google Slides";
  title: string;
  typeLabel:
    | "PR"
    | "issue"
    | "repo"
    | "file"
    | "folder"
    | "document"
    | "spreadsheet"
    | "presentation";
};

// Buzz relay hosts differ per community, so relay git URLs are recognized by
// their distinctive path shape (`/git/<64-hex-pubkey>/<repo>`) rather than by
// hostname, and require an explicit scheme.
const SUPPORTED_URL_RE =
  /(^|[\s([{<>"'])((?:https?:\/\/)?(?:(?:www\.)?github\.com|(?:www\.)?linear\.app|drive\.google\.com|docs\.google\.com)\/[^\s<>"'\]]+|https?:\/\/[^\s<>"'\]]+\/git\/[a-f0-9]{64}\/[^\s<>"'\]]+|buzz:\/\/(?:pr|issue|repo)\?[^\s<>"'\]]+)/gi;
const MARKDOWN_SUPPORTED_LINK_RE =
  /!?\[([^\]\n]+)\]\(((?:https?:\/\/)?(?:(?:www\.)?github\.com|(?:www\.)?linear\.app|drive\.google\.com|docs\.google\.com)\/[^)\s<>"']+|https?:\/\/[^)\s<>"']+\/git\/[a-f0-9]{64}\/[^)\s<>"']+|buzz:\/\/(?:pr|issue|repo)\?[^)\s<>"']+)\)/gi;
const MAX_PREVIEWS = 8;

type HiddenRange = {
  start: number;
  end: number;
};

function maskRanges(content: string, ranges: HiddenRange[]): string {
  if (ranges.length === 0) return content;

  const merged: HiddenRange[] = [];
  for (const range of [...ranges].sort((a, b) => a.start - b.start)) {
    const last = merged[merged.length - 1];
    if (last && range.start <= last.end) {
      last.end = Math.max(last.end, range.end);
    } else {
      merged.push({ ...range });
    }
  }

  let masked = "";
  let cursor = 0;
  for (const range of merged) {
    masked += content.slice(cursor, range.start);
    masked += content.slice(range.start, range.end).replace(/[^\n]/g, " ");
    cursor = range.end;
  }

  return masked + content.slice(cursor);
}

function isIndexInRanges(index: number, ranges: HiddenRange[]): boolean {
  return ranges.some((range) => index >= range.start && index < range.end);
}

function overlapsRange(
  start: number,
  end: number,
  ranges: HiddenRange[],
): boolean {
  return ranges.some((range) => start < range.end && end > range.start);
}

function collectCodeRanges(content: string): HiddenRange[] {
  const ranges: HiddenRange[] = [];
  for (const match of content.matchAll(/```[\s\S]*?```|~~~[\s\S]*?~~~/g)) {
    ranges.push({
      start: match.index ?? 0,
      end: (match.index ?? 0) + match[0].length,
    });
  }

  for (const match of content.matchAll(/`[^`\n]*`/g)) {
    ranges.push({
      start: match.index ?? 0,
      end: (match.index ?? 0) + match[0].length,
    });
  }

  for (const match of content.matchAll(/^(?: {4}|\t).*(?:\n|$)/gm)) {
    ranges.push({
      start: match.index ?? 0,
      end: (match.index ?? 0) + match[0].length,
    });
  }

  return ranges;
}

function collectMarkdownImageLinkRanges(content: string): HiddenRange[] {
  const ranges: HiddenRange[] = [];

  for (const match of content.matchAll(MARKDOWN_SUPPORTED_LINK_RE)) {
    if (!match[0]?.startsWith("!")) continue;
    ranges.push({
      start: match.index ?? 0,
      end: (match.index ?? 0) + match[0].length,
    });
  }

  return ranges;
}

function collectBlockSpoilerRanges(
  content: string,
  excludedRanges: HiddenRange[],
): HiddenRange[] {
  const ranges: HiddenRange[] = [];
  let openStart: number | null = null;
  let lineStart = 0;

  while (lineStart < content.length) {
    const newlineIndex = content.indexOf("\n", lineStart);
    const lineEnd =
      newlineIndex === -1 ? content.length : newlineIndex + "\n".length;
    const line = content.slice(
      lineStart,
      newlineIndex === -1 ? lineEnd : newlineIndex,
    );

    if (
      line.trim() === "||" &&
      !overlapsRange(lineStart, lineEnd, excludedRanges)
    ) {
      if (openStart == null) {
        openStart = lineStart;
      } else {
        ranges.push({ start: openStart, end: lineEnd });
        openStart = null;
      }
    }

    lineStart = lineEnd;
  }

  return ranges;
}

function collectInlineSpoilerRanges(
  content: string,
  excludedRanges: HiddenRange[],
): HiddenRange[] {
  const ranges: HiddenRange[] = [];
  let openStart: number | null = null;
  let index = 0;

  while (index < content.length - 1) {
    if (
      content[index] === "|" &&
      content[index + 1] === "|" &&
      !isIndexInRanges(index, excludedRanges) &&
      !isIndexInRanges(index + 1, excludedRanges)
    ) {
      if (openStart == null) {
        openStart = index;
      } else {
        ranges.push({ start: openStart, end: index + 2 });
        openStart = null;
      }
      index += 2;
      continue;
    }

    index += 1;
  }

  return ranges;
}

function stripHiddenLinkPreviewContent(content: string): string {
  const codeRanges = collectCodeRanges(content);
  const imageLinkRanges = collectMarkdownImageLinkRanges(content);
  const nonSpoilerHiddenRanges = [...codeRanges, ...imageLinkRanges];
  const blockSpoilerRanges = collectBlockSpoilerRanges(
    content,
    nonSpoilerHiddenRanges,
  );
  const inlineSpoilerRanges = collectInlineSpoilerRanges(content, [
    ...nonSpoilerHiddenRanges,
    ...blockSpoilerRanges,
  ]);

  return maskRanges(content, [
    ...nonSpoilerHiddenRanges,
    ...blockSpoilerRanges,
    ...inlineSpoilerRanges,
  ]);
}

function countChar(value: string, char: string): number {
  let count = 0;
  for (const current of value) {
    if (current === char) count += 1;
  }
  return count;
}

function trimUrlCandidate(candidate: string): string {
  let value = candidate.replace(/[.,!?;:]+$/g, "");

  const pairs: Array<[close: string, open: string]> = [
    [")", "("],
    ["]", "["],
    ["}", "{"],
  ];

  let changed = true;
  while (changed) {
    changed = false;
    for (const [close, open] of pairs) {
      if (
        value.endsWith(close) &&
        countChar(value, close) > countChar(value, open)
      ) {
        value = value.slice(0, -1);
        changed = true;
      }
    }
  }

  return value;
}

function safeDecode(value: string): string {
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
}

function normalizeHostname(parsed: URL): string {
  return parsed.hostname.toLowerCase().replace(/^www\./, "");
}

function createPreview(
  kind: SupportedLinkPreviewKind,
  parsed: URL,
  provider: SupportedLinkPreview["provider"],
  typeLabel: SupportedLinkPreview["typeLabel"],
  title: string,
): SupportedLinkPreview {
  return {
    kind,
    href: parsed.href,
    provider,
    title,
    typeLabel,
  };
}

/**
 * Placeholder title shown before (or instead of) the relay event lookup in
 * `useResolvedLinkPreviews` resolves the real `subject` / repo name.
 * Exported so the resolver can tell "still the fallback" apart from a
 * markdown-label override it must not overwrite.
 */
export function buzzEntityFallbackTitle(link: ParsedEntityLink): string {
  if (link.type === "repo") return link.dtag;
  return `${link.dtag} #${link.id.slice(0, 8)}`;
}

/**
 * Map a `buzz://pr|issue|repo` deep link onto a preview card. The href is
 * rebuilt through the canonical builders so equivalent links (case or query
 * order variants) dedupe to a single card.
 */
function parseBuzzEntityPreview(href: string): SupportedLinkPreview | null {
  const parsed = parseEntityLink(href);
  if (!parsed.ok) return null;

  const link = parsed.value;
  const title = buzzEntityFallbackTitle(link);
  if (link.type === "pr") {
    return {
      kind: "buzz-pull-request",
      href: buildPullRequestLink(link),
      provider: "Buzz",
      title,
      typeLabel: "PR",
    };
  }
  if (link.type === "issue") {
    return {
      kind: "buzz-issue",
      href: buildIssueLink(link),
      provider: "Buzz",
      title,
      typeLabel: "issue",
    };
  }
  return {
    kind: "buzz-repository",
    href: buildRepoLink(link),
    provider: "Buzz",
    title,
    typeLabel: "repo",
  };
}

const BUZZ_GIT_PATH_RE =
  /^\/git\/([a-f0-9]{64})\/([a-zA-Z0-9._-]+?)(?:\.git)?\/?$/;

/**
 * Recognize a Buzz relay git URL (`{relay-origin}/git/<owner-pubkey>/<repo>`,
 * the clone URL shape agents paste when announcing work). The preview href
 * is normalized to the canonical `buzz://repo` deep link: the raw git
 * transport endpoint is not a browsable page, and the buzz:// href gives the
 * card the same in-app click navigation as explicit entity links (and
 * dedupes the two spellings of the same repository).
 *
 * Security: the URL origin must equal `activeRelayOrigin` (the currently
 * connected relay). Path shape alone is not proof that a host belongs to the
 * active Buzz relay — an arbitrary external URL sharing the path shape must
 * remain an ordinary external link. Pass `null` when the relay origin is not
 * yet resolved; the link stays external until it can be verified.
 */
function parseBuzzGitLink(
  parsed: URL,
  activeRelayOrigin: string | null,
): SupportedLinkPreview | null {
  if (!activeRelayOrigin || parsed.origin !== activeRelayOrigin) {
    return null;
  }

  const match = BUZZ_GIT_PATH_RE.exec(parsed.pathname);
  if (!match) return null;

  const [, owner, repo] = match;
  if (repo.startsWith(".") || repo.includes("..") || repo.length > 64) {
    return null;
  }

  return {
    kind: "buzz-repository",
    href: buildRepoLink({ owner, dtag: repo }),
    provider: "Buzz",
    title: repo,
    typeLabel: "repo",
  };
}

function parseGithubLink(parsed: URL): SupportedLinkPreview | null {
  if (normalizeHostname(parsed) !== "github.com") {
    return null;
  }

  const segments = parsed.pathname.split("/").filter(Boolean).map(safeDecode);
  const [owner, repo, resource, number] = segments;
  if (!owner || !repo) return null;

  const repoLabel = `${owner}/${repo}`;
  if (resource === undefined) {
    return createPreview(
      "github-repository",
      parsed,
      "GitHub",
      "repo",
      repoLabel,
    );
  }

  if (/^\d+$/.test(number ?? "")) {
    if (resource === "pull") {
      return createPreview(
        "github-pull-request",
        parsed,
        "GitHub",
        "PR",
        `${repoLabel} #${number}`,
      );
    }

    if (resource === "issues") {
      return createPreview(
        "github-issue",
        parsed,
        "GitHub",
        "issue",
        `${repoLabel} #${number}`,
      );
    }
  }

  return null;
}

function parseLinearIssue(parsed: URL): SupportedLinkPreview | null {
  if (normalizeHostname(parsed) !== "linear.app") {
    return null;
  }

  const segments = parsed.pathname.split("/").filter(Boolean).map(safeDecode);
  const issueSegmentIndex = segments.findIndex(
    (segment) => segment.toLowerCase() === "issue",
  );
  const community = segments[0];
  const issueId = segments[issueSegmentIndex + 1]?.toUpperCase();

  if (
    !community ||
    issueSegmentIndex < 1 ||
    !issueId ||
    !/^[A-Z][A-Z0-9]*-\d+$/.test(issueId)
  ) {
    return null;
  }

  return createPreview("linear-issue", parsed, "Linear", "issue", issueId);
}

function parseGoogleDriveLink(parsed: URL): SupportedLinkPreview | null {
  if (normalizeHostname(parsed) !== "drive.google.com") {
    return null;
  }

  const segments = parsed.pathname.split("/").filter(Boolean);
  const folderSegmentIndex = segments.findIndex(
    (segment) => segment.toLowerCase() === "folders",
  );

  if (folderSegmentIndex >= 0 && segments[folderSegmentIndex + 1]) {
    return createPreview(
      "google-drive-folder",
      parsed,
      "Google Drive",
      "folder",
      "Drive folder",
    );
  }

  if (
    (segments[0] === "file" && segments[1] === "d" && segments[2]) ||
    (segments[0] === "open" && parsed.searchParams.has("id"))
  ) {
    return createPreview(
      "google-drive-file",
      parsed,
      "Google Drive",
      "file",
      "Drive file",
    );
  }

  return null;
}

function parseGoogleDocsLink(parsed: URL): SupportedLinkPreview | null {
  if (normalizeHostname(parsed) !== "docs.google.com") {
    return null;
  }

  const segments = parsed.pathname.split("/").filter(Boolean);
  const [resource, dSegment, id] = segments;
  if (dSegment !== "d" || !id) return null;

  if (resource === "document") {
    return createPreview(
      "google-docs-document",
      parsed,
      "Google Docs",
      "document",
      "Document",
    );
  }

  if (resource === "spreadsheets") {
    return createPreview(
      "google-sheets-spreadsheet",
      parsed,
      "Google Sheets",
      "spreadsheet",
      "Spreadsheet",
    );
  }

  if (resource === "presentation") {
    return createPreview(
      "google-slides-presentation",
      parsed,
      "Google Slides",
      "presentation",
      "Presentation",
    );
  }

  return null;
}

/** Parse a supported external URL into a compact preview. */
export function parseSupportedLinkPreview(
  href: string,
  activeRelayOrigin?: string | null,
): SupportedLinkPreview | null {
  const candidate = trimUrlCandidate(href);
  if (isEntityLink(candidate)) {
    return parseBuzzEntityPreview(candidate);
  }

  let parsed: URL;
  try {
    parsed = new URL(
      /^https?:\/\//i.test(candidate) ? candidate : `https://${candidate}`,
    );
  } catch {
    return null;
  }

  if (parsed.protocol !== "https:" && parsed.protocol !== "http:") {
    return null;
  }

  return (
    parseBuzzGitLink(parsed, activeRelayOrigin ?? null) ??
    parseGithubLink(parsed) ??
    parseLinearIssue(parsed) ??
    parseGoogleDriveLink(parsed) ??
    parseGoogleDocsLink(parsed)
  );
}

export function isSupportedLinkAutolinkLabel(
  label: string,
  preview: SupportedLinkPreview,
  activeRelayOrigin?: string | null,
): boolean {
  return (
    parseSupportedLinkPreview(label, activeRelayOrigin)?.href === preview.href
  );
}

function titleFromMarkdownLabel(
  label: string,
  preview: SupportedLinkPreview,
  activeRelayOrigin: string | null,
): string | null {
  const title = label.replace(/\s+/g, " ").trim();
  if (
    !title ||
    isSupportedLinkAutolinkLabel(title, preview, activeRelayOrigin)
  ) {
    return null;
  }
  return title;
}

function withTitle(
  preview: SupportedLinkPreview,
  title: string | null,
): SupportedLinkPreview {
  return title ? { ...preview, title } : preview;
}

type LinkPreviewCandidate = {
  href: string;
  index: number;
  label?: string;
  order: number;
};

/** Extract supported link previews from message text, preserving first-seen order. */
export function extractSupportedLinkPreviews(
  content: string,
  activeRelayOrigin?: string | null,
): SupportedLinkPreview[] {
  const previews: SupportedLinkPreview[] = [];
  const seen = new Set<string>();
  const searchable = stripHiddenLinkPreviewContent(content);
  const candidates: LinkPreviewCandidate[] = [];
  let order = 0;

  for (const match of searchable.matchAll(MARKDOWN_SUPPORTED_LINK_RE)) {
    if (match[0]?.startsWith("!")) continue;
    candidates.push({
      href: match[2],
      index: match.index ?? 0,
      label: match[1],
      order,
    });
    order += 1;
  }

  for (const match of searchable.matchAll(SUPPORTED_URL_RE)) {
    const prefix = match[1] ?? "";
    const href = match[2];
    if (!href) continue;
    candidates.push({
      href,
      index: (match.index ?? 0) + prefix.length,
      order,
    });
    order += 1;
  }

  candidates.sort((a, b) => a.index - b.index || a.order - b.order);

  const relayOrigin = activeRelayOrigin ?? null;
  for (const candidate of candidates) {
    const preview = parseSupportedLinkPreview(candidate.href, relayOrigin);
    if (!preview || seen.has(preview.href)) continue;

    seen.add(preview.href);
    previews.push(
      withTitle(
        preview,
        candidate.label
          ? titleFromMarkdownLabel(candidate.label, preview, relayOrigin)
          : null,
      ),
    );
    if (previews.length >= MAX_PREVIEWS) break;
  }

  return previews;
}
