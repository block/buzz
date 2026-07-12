import type * as React from "react";

import { extractSupportedLinkPreviews } from "@/shared/lib/linkPreview";
import { dimensionsFromDim } from "@/shared/ui/markdown/utils";
import type { TimelineItem } from "./timelineItems";
import type { TimelineMessage } from "../types";
import { parseImetaTags } from "./parseImeta";

/**
 * Estimate a timeline row's rendered height so its `content-visibility`
 * placeholder reserves credible space BEFORE first paint. A flat placeholder
 * makes a never-painted media/code row snap from the floor to its true height
 * as it realizes on scroll-up — the teleport. The browser's `auto` keyword
 * still refines the size once the row paints; this only has to make that first
 * realization land near-correct, not exact.
 *
 * Deliberately conservative and cheap (no DOM, no markdown parse): row chrome
 * + a line-count estimate for text/code + known media `dim`s. Over-reserving a
 * little is harmless (a small downward settle); under-reserving by a lot is the
 * jump we're killing.
 */

// Visual caps mirror the inline image/markdown styles.
const MEDIA_MAX_WIDTH = 384; // max-w-[min(24rem,100%)]
const MEDIA_MAX_HEIGHT = 256; // max-h-64
const TEXT_LINE_HEIGHT = 20;
const CODE_LINE_HEIGHT = 19;
const FALLBACK_CHARS_PER_LINE = 64; // rough wrap width at the timeline column
const AVERAGE_TEXT_CHAR_WIDTH = 7.2; // text-sm, biased toward common prose
const ROW_HORIZONTAL_CHROME = 64; // avatar + row gap + inline padding
const MIN_CHARS_PER_LINE = 32;
const MAX_CHARS_PER_LINE = 96;
const ROW_CHROME = 26; // author/time header + denser row padding
const CONTINUATION_ROW_CHROME = 8; // dense row padding only; header/avatar are hidden
const MEDIA_BLOCK_MARGIN_TOP = 4; // image/video blocks use mt-1 in markdown
const REACTION_ROW = 24;
const PREVIEW_CARD = 70;
const THREAD_SUMMARY_ROW = 38;
const FOOTER_ROW = 32;
const MESSAGE_ITEM_BOTTOM_PADDING = 10; // TimelineMessageList pb-2.5
const MIN_ESTIMATE = 60; // never reserve less than the old flat floor
const CONTINUATION_MIN_ESTIMATE = 34;

export type TimelineRowReserveOptions = {
  /** Timeline column width measured once by the caller; absent keeps the old 64-char estimate. */
  columnWidthPx?: number;
  /** Optional row footer chrome. The main channel timeline currently leaves this unset. */
  hasFooter?: boolean;
};

type EstimateRowHeightOptions = TimelineRowReserveOptions & {
  isContinuation?: boolean;
};

function mediaHeightFromDim(dim: string | undefined): number {
  const dimensions = dimensionsFromDim(dim);
  if (!dimensions) return MEDIA_MAX_HEIGHT; // unknown shape: reserve full box
  const scale = Math.min(
    1,
    MEDIA_MAX_WIDTH / dimensions.width,
    MEDIA_MAX_HEIGHT / dimensions.height,
  );
  return Math.round(dimensions.height * scale);
}

function mediaReserveHeight(dim: string | undefined): number {
  return MEDIA_BLOCK_MARGIN_TOP + mediaHeightFromDim(dim);
}

function charsPerLineFromColumnWidth(
  columnWidthPx: number | undefined,
): number {
  if (columnWidthPx == null || !Number.isFinite(columnWidthPx)) {
    return FALLBACK_CHARS_PER_LINE;
  }

  const textWidth = Math.max(0, columnWidthPx - ROW_HORIZONTAL_CHROME);
  return Math.max(
    MIN_CHARS_PER_LINE,
    Math.min(
      MAX_CHARS_PER_LINE,
      Math.floor(textWidth / AVERAGE_TEXT_CHAR_WIDTH),
    ),
  );
}

function wrappedLineCount(text: string, charsPerLine: number): number {
  let lines = 0;
  for (const raw of text.split("\n")) {
    lines += Math.max(1, Math.ceil(raw.length / charsPerLine));
  }
  return lines;
}

/**
 * Strip fenced code blocks from the body, returning the prose remainder and the
 * total number of code lines (for separate mono line-height accounting).
 */
function splitFencedCode(body: string): {
  prose: string;
  codeLines: number;
} {
  const parts = body.split(/```/);
  // Even indices are prose, odd indices are inside a fence.
  let prose = "";
  let codeLines = 0;
  for (let i = 0; i < parts.length; i += 1) {
    if (i % 2 === 1) {
      codeLines += parts[i].split("\n").length;
    } else {
      prose += parts[i];
    }
  }
  return { prose, codeLines };
}

// Image/video file extensions the markdown renderer turns into inline media.
const MEDIA_URL_RE =
  /https?:\/\/\S+\.(?:png|jpe?g|gif|webp|avif|svg|mp4|webm|mov)(?:\?\S*)?$/i;
const MARKDOWN_IMAGE_LINE_RE = /^\s*!\[[^\]]*\]\([^)\s]+\)\s*$/;

/**
 * URLs in the body that the markdown renderer shows as inline `<img>`/`<video>`:
 * `![alt](url)` markdown images and bare media URLs on their own line. Used to
 * reserve media height for dim-less inline media (no imeta tag).
 */
function mediaUrlsInBody(body: string): string[] {
  const urls: string[] = [];
  for (const match of body.matchAll(/!\[[^\]]*\]\(([^)\s]+)\)/g)) {
    urls.push(match[1]);
  }
  for (const line of body.split("\n")) {
    const trimmed = line.trim();
    if (MEDIA_URL_RE.test(trimmed)) urls.push(trimmed);
  }
  return urls;
}

function stripMediaOnlyLines(text: string): string {
  return text
    .split("\n")
    .filter((line) => {
      const trimmed = line.trim();
      return !MARKDOWN_IMAGE_LINE_RE.test(line) && !MEDIA_URL_RE.test(trimmed);
    })
    .join("\n");
}

function markdownStructureExtraHeight(text: string): number {
  let extra = 0;
  let tableRunLines = 0;

  const flushTableRun = () => {
    if (tableRunLines >= 2) {
      // GFM tables have cell padding/borders, so they are taller than the same
      // raw markdown counted as plain 20px text lines.
      extra += 14 + tableRunLines * 6;
    }
    tableRunLines = 0;
  };

  for (const line of text.split("\n")) {
    const trimmed = line.trim();
    const isTableLine =
      /^\|.+\|$/.test(trimmed) || /\S\s+\|\s+\S/.test(trimmed);

    if (isTableLine) tableRunLines += 1;
    else flushTableRun();

    if (/^(?:---+|\*\*\*+|___+)\s*$/.test(trimmed)) extra += 12;
  }

  flushTableRun();
  return extra;
}

export function estimateRowHeight(
  message: TimelineMessage,
  {
    columnWidthPx,
    hasFooter = false,
    isContinuation = false,
  }: EstimateRowHeightOptions = {},
): number {
  const body = message.body ?? "";
  const { prose, codeLines } = splitFencedCode(body);
  const proseForLineCount = stripMediaOnlyLines(prose);
  const charsPerLine = charsPerLineFromColumnWidth(columnWidthPx);

  let height = isContinuation ? CONTINUATION_ROW_CHROME : ROW_CHROME;
  height +=
    wrappedLineCount(
      proseForLineCount.trim() === "" ? "" : proseForLineCount,
      charsPerLine,
    ) * TEXT_LINE_HEIGHT;
  height += codeLines * CODE_LINE_HEIGHT;
  height += markdownStructureExtraHeight(proseForLineCount);

  const imetaUrls = new Set<string>();
  if (message.tags && message.tags.length > 0) {
    const imeta = parseImetaTags(message.tags);
    for (const entry of imeta.values()) {
      if (!entry.url) continue;
      imetaUrls.add(entry.url);
      height += mediaReserveHeight(entry.dim);
    }
  }
  for (const url of mediaUrlsInBody(body)) {
    if (imetaUrls.has(url)) continue; // already counted via its imeta dim
    // dim-less inline media reserves the fixed markdown image box plus its mt-1.
    height += mediaReserveHeight(undefined);
  }

  // Reserve only cards the renderer can actually produce. Generic bare URLs do
  // not render preview cards, so keeping the old blanket reserve overestimated
  // unsupported links by about one card height during first realization.
  height += extractSupportedLinkPreviews(body).length * PREVIEW_CARD;

  if (message.reactions && message.reactions.length > 0) height += REACTION_ROW;
  if (hasFooter) height += FOOTER_ROW;

  return Math.max(
    isContinuation ? CONTINUATION_MIN_ESTIMATE : MIN_ESTIMATE,
    Math.round(height),
  );
}

// Dividers are short, fixed-height rows; reserving their true height keeps the
// estimate honest without a content scan.
const DIVIDER_HEIGHT = 32;

/**
 * `contain-intrinsic-size` for a `timeline-row-cv` wrapper. A credible per-row
 * reserve replaces the flat 60px placeholder so a never-painted row realizes
 * near its true height instead of snapping the scroll position. `auto` keeps
 * refining once the row paints.
 */
export function timelineRowReserveStyle(
  item: TimelineItem,
  opts: TimelineRowReserveOptions = {},
): React.CSSProperties {
  const height =
    item.kind === "message"
      ? estimateRowHeight(item.entry.message, {
          ...opts,
          isContinuation: item.isContinuation,
        }) +
        (item.entry.summary ? THREAD_SUMMARY_ROW : 0) +
        (item.isFollowedByContinuation ? 0 : MESSAGE_ITEM_BOTTOM_PADDING)
      : item.kind === "system"
        ? estimateRowHeight(item.entry.message, opts)
        : DIVIDER_HEIGHT;
  return { containIntrinsicSize: `auto ${height}px` };
}
