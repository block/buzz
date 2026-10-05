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
 * deep; anything else stays text. Mobile mirrors these rules in
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
    const line = rawLine.replace(/\r$/, "");
    if (fence) {
      if (closesFence(line, fence)) fence = null;
      return;
    }
    fence = fenceOpen(line);
    if (fence) return;
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
    .map((line, index) => (matched.has(index) ? `\n${line}\n` : line))
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

/** Plain-text title: the marker paragraph minus its `:::details ` prefix.
 * Plain so the disclosure control holds no links or other controls. */
function summaryTitle(marker: Node): string {
  const title = plainText(marker.children as Node[])
    .replace(/^:::details[ \t]+/, "")
    .trim();
  return title || DETAILS_FALLBACK_TITLE;
}

function plainText(nodes: Node[]): string {
  return nodes
    .map((node) =>
      typeof node.value === "string"
        ? node.value
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
        const title = summaryTitle(frame.marker);
        const occurrence = occurrences.get(title) ?? 0;
        occurrences.set(title, occurrence + 1);
        target().push({
          type: "details",
          children: [
            {
              type: "detailsSummary",
              children: [{ type: "text", value: title }],
              data: { hName: "summary" },
            },
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
