/**
 * Rehype plugin that groups consecutive image-only paragraphs into a single
 * merged `<p>` containing all the images. The custom `p` component in
 * markdown.tsx detects 2+ images and renders them as an adaptive mosaic.
 *
 * This runs at the HAST (HTML AST) level, before React rendering, so
 * consecutive `![](a)\n![](b)` paragraphs get merged and the `p` component
 * receives all images together.
 *
 * A paragraph is "image-only" when it contains only `<img>` elements
 * (plus optional whitespace text nodes and `<br>` from remark processing).
 */

// Minimal HAST types — avoids adding @types/hast as a dependency.
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

function isElement(node: HastNode): node is HastElement {
  return node.type === "element";
}

function isText(node: HastNode): node is HastText {
  return node.type === "text";
}

function isIgnorableImageSeparator(node: HastNode): boolean {
  return (
    (isText(node) && node.value.trim() === "") ||
    (isElement(node) && node.tagName === "br")
  );
}

function isImageOnlyParagraph(node: HastNode): node is HastElement {
  if (!isElement(node) || node.tagName !== "p") {
    return false;
  }

  const meaningful = node.children.filter(
    (child) => !isIgnorableImageSeparator(child),
  );

  return (
    meaningful.length >= 1 &&
    meaningful.every((child) => isElement(child) && child.tagName === "img")
  );
}

/**
 * Composer attachments are appended with soft line breaks, so a post with text
 * and multiple images initially arrives as one mixed paragraph:
 * `text<br><img><br><img>`. Split that trailing image run into its own paragraph
 * so it can use the same gallery path as image-only Markdown.
 *
 * Only a trailing run of 2+ images is split. A lone inline image and images
 * separated by meaningful content retain their original Markdown flow.
 */
function splitTrailingImageRun(node: HastNode): HastNode[] {
  if (!isElement(node) || node.tagName !== "p") return [node];

  let cursor = node.children.length - 1;
  const trailingImages: HastElement[] = [];

  while (cursor >= 0) {
    const child = node.children[cursor];
    if (isElement(child) && child.tagName === "img") {
      trailingImages.unshift(child);
      cursor -= 1;
      continue;
    }
    if (isIgnorableImageSeparator(child)) {
      cursor -= 1;
      continue;
    }
    break;
  }

  if (trailingImages.length < 2 || cursor < 0) return [node];

  return [
    { ...node, children: node.children.slice(0, cursor + 1) },
    {
      type: "element",
      tagName: "p",
      properties: {},
      children: trailingImages,
    },
  ];
}

export default function rehypeImageGallery() {
  return (tree: HastRoot) => {
    const normalizedChildren = tree.children.flatMap(splitTrailingImageRun);
    const newChildren: HastNode[] = [];
    let imageRun: HastElement[] = [];

    function flushRun() {
      if (imageRun.length <= 1) {
        newChildren.push(...imageRun);
      } else {
        // Merge consecutive single-image paragraphs into one paragraph
        // containing all the images. The `p` component in markdown.tsx
        // will detect 2+ images and render the mosaic gallery.
        const allImages: HastNode[] = [];
        for (const p of imageRun) {
          for (const child of p.children) {
            if (isElement(child) && child.tagName === "img") {
              allImages.push(child);
            }
          }
        }
        newChildren.push({
          type: "element",
          tagName: "p",
          properties: {},
          children: allImages,
        });
      }
      imageRun = [];
    }

    for (const child of normalizedChildren) {
      if (isImageOnlyParagraph(child)) {
        imageRun.push(child);
        continue;
      }
      flushRun();
      newChildren.push(child);
    }
    flushRun();

    tree.children = newChildren;
  };
}
