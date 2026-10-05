/**
 * Collapsible sections in message markdown:
 *
 *   :::details Section title
 *   Any markdown, collapsed until the reader opens it.
 *   :::
 *
 * `prepareDetailsBlocks` runs on the raw string and isolates every matched
 * marker line as its own paragraph (a `:::` right under a list item would
 * otherwise be read as a lazy continuation of that item). `remarkDetails`
 * then turns the marker paragraphs into a custom node rendered as a
 * disclosure by `markdown.tsx`. Markers are recognised only at column 0,
 * outside fenced code, in matched pairs and at most `MAX_DETAILS_DEPTH`
 * deep; anything else stays text. A marker line wrapped in emphasis as a
 * whole (`**:::details Title**`, `**:::**`, what the composer sends with bold
 * switched on) still counts. A title may carry inline formatting and start
 * with `#`..`######` to render as a heading. Mobile mirrors these rules in
 * `details_blocks.dart`.
 */

type Node = {
  // biome-ignore lint/suspicious/noExplicitAny: building mdast-compatible nodes
  [key: string]: any;
};

/** Deeper markers stay text, which bounds render recursion on hostile input. */
export const MAX_DETAILS_DEPTH = 4;

const OPEN_RE = /^:::details[ \t]+\S.*$/;
const CLOSE_RE = /^:::[ \t]*$/;
const WRAPPED_RE = /^(\*\*|__|\*|_)(:::.*?)\1[ \t]*$/;
const HEADING_RE = /^(#{1,6})[ \t]+/;

/** The marker a line stands for, with a whole-line emphasis wrapper moved
 * into the title (`**:::details X**` → `:::details **X**`), or null. */
export function normalizeMarkerLine(line: string): string | null {
  if (OPEN_RE.test(line) || CLOSE_RE.test(line)) return line;
  const wrapped = WRAPPED_RE.exec(line);
  if (!wrapped) return null;
  const [, mark, inner] = wrapped;
  if (CLOSE_RE.test(inner)) return ":::";
  if (!OPEN_RE.test(inner)) return null;
  const title = inner.replace(/^:::details[ \t]+/, "");
  const heading = HEADING_RE.exec(title)?.[0] ?? "";
  return `:::details ${heading}${mark}${title.slice(heading.length)}${mark}`;
}
const FENCE_OPEN_RE = /^ {0,3}(`{3,}|~{3,})(.*)$/;
const FENCE_CLOSE_RE = /^ {0,3}(`{3,}|~{3,})[ \t]*$/;

/** The fence a line opens, or null. Backtick info strings may not contain a
 * backtick (CommonMark). */
function fenceOpen(line: string): string | null {
  const match = FENCE_OPEN_RE.exec(line);
  if (!match) return null;
  return match[1][0] === "`" && match[2].includes("`") ? null : match[1];
}

function closesFence(line: string, fence: string): boolean {
  const match = FENCE_CLOSE_RE.exec(line);
  return (
    match !== null &&
    match[1][0] === fence[0] &&
    match[1].length >= fence.length
  );
}

/** Returns the indices of opening and closing marker lines that pair up. */
function matchedMarkerLines(lines: string[]): Set<number> {
  const matched = new Set<number>();
  // Opener line index, or -1 for an opener past the depth limit: it still
  // consumes its closer, so both stay text.
  const open: number[] = [];
  let depth = 0;
  let fence: string | null = null;

  lines.forEach((rawLine, index) => {
    const raw = rawLine.replace(/\r$/, "");
    if (fence) {
      if (closesFence(raw, fence)) fence = null;
      return;
    }
    fence = fenceOpen(raw);
    if (fence) return;
    const line = normalizeMarkerLine(raw);
    if (line === null) return;
    if (OPEN_RE.test(line)) {
      const allowed = depth < MAX_DETAILS_DEPTH;
      open.push(allowed ? index : -1);
      if (allowed) depth += 1;
    } else if (CLOSE_RE.test(line) && open.length > 0) {
      const start = open.pop() as number;
      if (start >= 0) {
        depth -= 1;
        matched.add(start);
        matched.add(index);
      }
    }
  });

  return matched;
}

export function prepareDetailsBlocks(content: string): string {
  if (!content.includes(":::details")) return content;
  const lines = content.split("\n");
  const matched = matchedMarkerLines(lines);
  if (matched.size === 0) return content;
  return lines
    .map((line, index) =>
      matched.has(index)
        ? `\n${normalizeMarkerLine(line.replace(/\r$/, ""))}\n`
        : line,
    )
    .join("\n");
}

function markerText(node: Node): string | null {
  if (node.type !== "paragraph" || !Array.isArray(node.children)) return null;
  const first = node.children[0];
  return first?.type === "text" ? String(first.value ?? "") : null;
}

function isOpenMarker(node: Node): boolean {
  const text = markerText(node);
  return (
    text !== null &&
    /^:::details[ \t]/.test(text) &&
    !node.children.some((child: Node) => child.type === "break")
  );
}

function isCloseMarker(node: Node): boolean {
  const text = markerText(node);
  return text !== null && node.children.length === 1 && CLOSE_RE.test(text);
}

/** Shown when a title has no text of its own, e.g. `:::details [](url)`. */
export const DETAILS_FALLBACK_TITLE = "Details";

/** Inline formatting a title keeps; links, mentions and other controls
 * become their text, so the disclosure button is the only control. */
const TITLE_NODE_TYPES = new Set([
  "text",
  "strong",
  "emphasis",
  "delete",
  "inlineCode",
]);

function sanitizeTitle(nodes: Node[]): Node[] {
  return nodes.map((node) => {
    if (!TITLE_NODE_TYPES.has(node.type)) {
      return { type: "text", value: plainText([node]) };
    }
    return Array.isArray(node.children)
      ? { ...node, children: sanitizeTitle(node.children) }
      : node;
  });
}

type Summary = { children: Node[]; level: number; text: string };

/** Title of a marker paragraph: inline nodes after `:::details `, an
 * optional heading level, and its plain text (the section key). */
function summaryOf(marker: Node): Summary {
  // Earlier plugins may split the leading text (e.g. `##` read as a channel
  // link); flatten and rejoin it so the prefix and heading hashes are read
  // from one node.
  const merged: Node[] = [];
  for (const node of sanitizeTitle(marker.children as Node[])) {
    const last = merged[merged.length - 1];
    if (node.type === "text" && last?.type === "text") {
      merged[merged.length - 1] = { ...last, value: last.value + node.value };
    } else {
      merged.push(node);
    }
  }
  const [first, ...rest] = merged;
  let value = String(first.value).replace(/^:::details[ \t]+/, "");
  const heading = HEADING_RE.exec(value);
  if (heading) value = value.slice(heading[0].length);
  let children = value ? [{ ...first, value }, ...rest] : rest;
  let text = plainText(children).trim();
  if (!text) {
    text = DETAILS_FALLBACK_TITLE;
    children = [{ type: "text", value: text }];
  }
  return { children, level: heading ? heading[1].length : 0, text };
}

function plainText(nodes: Node[]): string {
  return nodes
    .map((node) =>
      typeof node.value === "string"
        ? node.value
        : node.type === "image"
          ? String(node.alt ?? "")
          : plainText((node.children as Node[] | undefined) ?? []),
    )
    .join("");
}

type Frame = { marker: Node; children: Node[] };

export default function remarkDetails() {
  return (
    // biome-ignore lint/suspicious/noExplicitAny: remark tree types are not available
    tree: any,
  ) => {
    const occurrences = new Map<string, number>();
    const root: Node[] = [];
    const stack: Frame[] = [];
    // Openers past the depth limit, still waiting for their closers. They
    // and their closers stay text in place, so the work stays linear.
    let skipped = 0;
    const target = () => stack[stack.length - 1]?.children ?? root;

    for (const child of tree.children as Node[]) {
      if (isOpenMarker(child)) {
        if (stack.length >= MAX_DETAILS_DEPTH) {
          skipped += 1;
          target().push(child);
        } else {
          stack.push({ marker: child, children: [] });
        }
        continue;
      }
      if (skipped > 0 && isCloseMarker(child)) {
        skipped -= 1;
        target().push(child);
        continue;
      }
      const frame = stack[stack.length - 1];
      if (frame && isCloseMarker(child)) {
        stack.pop();
        const { children, level, text: title } = summaryOf(frame.marker);
        const occurrence = occurrences.get(title) ?? 0;
        occurrences.set(title, occurrence + 1);
        const summary = {
          type: "detailsSummary",
          children,
          data: {
            hName: "summary",
            hProperties: level > 0 ? { "data-heading": "" } : {},
          },
        };
        target().push({
          type: "details",
          children: [
            // A heading title keeps the message's heading style, with the
            // toggle inside it (the WAI-ARIA accordion pattern).
            level > 0
              ? { type: "heading", depth: level, children: [summary] }
              : summary,
            ...frame.children,
          ],
          data: {
            hName: "details",
            // Stable across edits elsewhere in the message, so a reader's
            // open/closed choice survives in-place updates.
            hProperties: { "data-details-key": `${occurrence}:${title}` },
          },
        });
        continue;
      }
      target().push(child);
    }

    // `prepareDetailsBlocks` only isolates matched pairs, but a marker can
    // still lose its partner inside a container (e.g. a closing `:::` in a
    // list item). Unmatched openers fall back to their original paragraphs.
    while (stack.length > 0) {
      const frame = stack.pop() as Frame;
      target().push(frame.marker, ...frame.children);
    }
    tree.children = root;
  };
}
