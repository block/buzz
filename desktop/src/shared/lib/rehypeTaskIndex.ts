/**
 * Rehype plugin that stamps every GFM task-list checkbox with its ordinal
 * position in the document, as `data-task-index`.
 *
 * The checkbox `<input>` is *generated* during the mdast→hast conversion, so
 * unlike authored elements it carries no source `position` — there is no
 * offset available to map a click back to the `- [ ]` marker that produced
 * it. The ordinal is the stable alternative: remark-gfm emits exactly one
 * checkbox per task item, in document order, so the Nth checkbox always
 * corresponds to the Nth task marker in the source. `toggleTaskMarker.mjs`
 * resolves the ordinal on the way back and counts markers the same way
 * (fenced code skipped, whitespace required after the bracket).
 *
 * The stamp depends only on the content, which is what keeps the parsed tree
 * safe to reuse from `nodeCache.ts` — see that module's doc comment.
 */

// Minimal HAST types — matches the pattern in rehypeImageGallery.ts.
interface HastElement {
  type: "element";
  tagName: string;
  properties: Record<string, unknown>;
  children: HastNode[];
}

type HastNode = HastElement | { type: string; children?: HastNode[] };

interface HastRoot {
  type: "root";
  children: HastNode[];
}

function isElement(node: HastNode): node is HastElement {
  return node.type === "element";
}

/**
 * Prop name the stamped ordinal arrives under in the React component. hast
 * lowercases and hyphenates `dataTaskIndex` on the way to the DOM, so the
 * property is written camelCase and read hyphenated.
 */
export const TASK_INDEX_PROP = "data-task-index";

export function rehypeTaskIndex() {
  return (tree: HastRoot) => {
    let index = 0;

    const visit = (node: HastNode) => {
      if (
        isElement(node) &&
        node.tagName === "input" &&
        node.properties?.type === "checkbox"
      ) {
        node.properties.dataTaskIndex = String(index);
        index += 1;
      }
      const children = (node as { children?: HastNode[] }).children;
      if (!children) return;
      for (const child of children) visit(child);
    };

    visit(tree);
  };
}

/**
 * Parse the stamped ordinal back out of the component's props. Returns `null`
 * for anything that is not a well-formed index, so a caller can decline to
 * act rather than guess at which task was clicked.
 */
export function readTaskIndex(value: unknown): number | null {
  if (typeof value !== "string") return null;
  if (!/^\d+$/.test(value)) return null;
  const parsed = Number.parseInt(value, 10);
  return Number.isSafeInteger(parsed) ? parsed : null;
}
