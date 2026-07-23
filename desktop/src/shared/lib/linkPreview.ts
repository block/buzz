export type SupportedLinkPreviewKind =
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

const SUPPORTED_URL_RE =
  /(^|[\s([{<>"'])((?:https?:\/\/)?(?:(?:www\.)?github\.com|(?:www\.)?linear\.app|drive\.google\.com|docs\.google\.com)\/[^\s<>"'\]]+)/gi;
const MARKDOWN_SUPPORTED_LINK_RE =
  /!?\[([^\]\n]+)\]\(((?:https?:\/\/)?(?:(?:www\.)?github\.com|(?:www\.)?linear\.app|drive\.google\.com|docs\.google\.com)\/[^)\s<>"']+)\)/gi;
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
): SupportedLinkPreview | null {
  let parsed: URL;
  try {
    const candidate = trimUrlCandidate(href);
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
    parseGithubLink(parsed) ??
    parseLinearIssue(parsed) ??
    parseGoogleDriveLink(parsed) ??
    parseGoogleDocsLink(parsed)
  );
}

export function isSupportedLinkAutolinkLabel(
  label: string,
  preview: SupportedLinkPreview,
): boolean {
  return parseSupportedLinkPreview(label)?.href === preview.href;
}

function titleFromMarkdownLabel(
  label: string,
  preview: SupportedLinkPreview,
): string | null {
  const title = label.replace(/\s+/g, " ").trim();
  if (!title || isSupportedLinkAutolinkLabel(title, preview)) {
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

  for (const candidate of candidates) {
    const preview = parseSupportedLinkPreview(candidate.href);
    if (!preview || seen.has(preview.href)) continue;

    seen.add(preview.href);
    previews.push(
      withTitle(
        preview,
        candidate.label
          ? titleFromMarkdownLabel(candidate.label, preview)
          : null,
      ),
    );
    if (previews.length >= MAX_PREVIEWS) break;
  }

  return previews;
}
