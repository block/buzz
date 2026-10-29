// Minimal HAST types — matches the pattern in rehypeImageGallery.ts.
interface HastText {
  type: "text";
  value: string;
}

interface HastElement {
  type: "element";
  tagName: string;
  properties: Record<string, unknown>;
  children: HastNode[];
}

type HastNode = HastElement | HastText | { type: string };

interface HastRoot {
  type: "root";
  children: HastNode[];
}

const INLINE_TARGETS = new Set([
  "h1",
  "h2",
  "h3",
  "h4",
  "h5",
  "h6",
  "li",
  "p",
  "td",
  "th",
]);

function isElement(node: HastNode): node is HastElement {
  return node.type === "element";
}

function isText(node: HastNode): node is HastText {
  return node.type === "text";
}

function isMediaOnlyParagraph(node: HastElement): boolean {
  if (node.tagName !== "p") return false;

  const meaningful = node.children.filter(
    (child) =>
      !(isText(child) && child.value.trim() === "") &&
      !(isElement(child) && child.tagName === "br"),
  );
  return (
    meaningful.length > 0 &&
    meaningful.every((child) => isElement(child) && child.tagName === "img")
  );
}

function leadingMarker(): HastElement {
  return {
    type: "element",
    tagName: "span",
    properties: { "data-leading-inline-content": "" },
    children: [],
  };
}

function isMeaningfulNode(node: HastNode): boolean {
  return !(isText(node) && node.value.trim() === "");
}

function prependToFirstInlineTarget(node: HastNode): boolean {
  if (!isElement(node)) return false;

  if (INLINE_TARGETS.has(node.tagName) && !isMediaOnlyParagraph(node)) {
    // Nested paragraphs provide the natural prose flow for quotes and loose
    // list items. Tight list items contain text directly, so the <li> itself
    // is the correct fallback target.
    if (node.tagName === "li") {
      const directParagraph = node.children.find(
        (child) => isElement(child) && child.tagName === "p",
      );
      if (directParagraph && prependToFirstInlineTarget(directParagraph)) {
        return true;
      }
    }
    node.children.unshift(leadingMarker());
    return true;
  }

  // Only inspect the first rendered block. If it cannot accept inline content
  // (for example, code or media), the caller inserts the fallback before its
  // containing block instead of moving the marker into later prose.
  const firstChild = node.children.find(isMeaningfulNode);
  return firstChild ? prependToFirstInlineTarget(firstChild) : false;
}

/**
 * Inserts a render-time marker into the first prose-capable Markdown block.
 * Blocks without inline flow, such as code and media, receive a preceding
 * marker paragraph so callers never lose their leading control.
 */
export default function rehypeLeadingInlineContent() {
  return (tree: HastRoot) => {
    for (const child of tree.children) {
      if (isText(child) && child.value.trim() === "") continue;
      if (prependToFirstInlineTarget(child)) return;
      break;
    }

    tree.children.unshift({
      type: "element",
      tagName: "p",
      properties: {},
      children: [leadingMarker()],
    });
  };
}
