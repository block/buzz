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
 * outside fenced code, and only in matched pairs; anything else stays text.
 */

type Node = {
  // biome-ignore lint/suspicious/noExplicitAny: building mdast-compatible nodes
  [key: string]: any;
};

const OPEN_RE = /^:::details[ \t]+(\S.*)$/;
const CLOSE_RE = /^:::[ \t]*$/;
const FENCE_RE = /^ {0,3}(`{3,}|~{3,})/;

/** Returns the indices of opening and closing marker lines that pair up. */
function matchedMarkerLines(lines: string[]): Set<number> {
  const matched = new Set<number>();
  const open: number[] = [];
  let fence: string | null = null;

  lines.forEach((line, index) => {
    const fenceMatch = FENCE_RE.exec(line);
    if (fence) {
      if (
        fenceMatch &&
        fenceMatch[1][0] === fence[0] &&
        fenceMatch[1].length >= fence.length
      ) {
        fence = null;
      }
      return;
    }
    if (fenceMatch) {
      fence = fenceMatch[1];
      return;
    }
    if (OPEN_RE.test(line)) {
      open.push(index);
    } else if (CLOSE_RE.test(line) && open.length > 0) {
      matched.add(open.pop() as number);
      matched.add(index);
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
    !node.children.some((child: Node) => child.type === "break") &&
    plainText(summaryChildren(node)).trim() !== ""
  );
}

function isCloseMarker(node: Node): boolean {
  const text = markerText(node);
  return text !== null && node.children.length === 1 && CLOSE_RE.test(text);
}

/** Summary children: the marker paragraph minus its `:::details ` prefix. */
function summaryChildren(marker: Node): Node[] {
  const [first, ...rest] = marker.children as Node[];
  const value = String(first.value).replace(/^:::details[ \t]+/, "");
  return value ? [{ ...first, value }, ...rest] : rest;
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
    const target = () => stack[stack.length - 1]?.children ?? root;

    for (const child of tree.children as Node[]) {
      if (isOpenMarker(child)) {
        stack.push({ marker: child, children: [] });
        continue;
      }
      const frame = stack[stack.length - 1];
      if (frame && isCloseMarker(child)) {
        stack.pop();
        const summary = summaryChildren(frame.marker);
        const title = plainText(summary).trim();
        const occurrence = occurrences.get(title) ?? 0;
        occurrences.set(title, occurrence + 1);
        target().push({
          type: "details",
          children: [
            {
              type: "detailsSummary",
              children: summary,
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
