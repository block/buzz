/**
 * Rehype plugin that highlights text matching a search query by wrapping
 * matches in `<mark>` elements during the HAST (HTML AST) phase.
 *
 * This runs inside the react-markdown pipeline, so it works correctly with
 * ReactMarkdown's architecture — no post-render tree walking needed.
 */

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

function isElement(node: HastNode): node is HastElement {
  return node.type === "element";
}

function isText(node: HastNode): node is HastText {
  return node.type === "text";
}

function escapeRegExp(string: string): string {
  return string.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

export default function rehypeSearchHighlight({ query }: { query: string }) {
  return (tree: HastRoot) => {
    const trimmed = query.trim();
    if (trimmed.length < 2) return;

    const pattern = new RegExp(`(${escapeRegExp(trimmed)})`, "i");

    function walk(nodes: HastNode[]): HastNode[] {
      const result: HastNode[] = [];

      for (const node of nodes) {
        if (isText(node)) {
          const parts = node.value.split(pattern);
          if (parts.length === 1) {
            result.push(node);
            continue;
          }

          for (let i = 0; i < parts.length; i++) {
            const part = parts[i];
            if (!part) continue;

            if (i % 2 === 1) {
              // Odd indices from split-with-capture are always the match.
              result.push({
                type: "element",
                tagName: "mark",
                properties: {
                  className:
                    "rounded-xs bg-primary/20 text-foreground dark:bg-primary/30",
                },
                children: [{ type: "text", value: part }],
              });
            } else {
              result.push({ type: "text", value: part });
            }
          }
        } else if (isElement(node)) {
          // Don't descend into <code> or <pre> — keep code blocks untouched.
          if (node.tagName === "code" || node.tagName === "pre") {
            result.push(node);
          } else {
            result.push({
              ...node,
              children: walk(node.children),
            });
          }
        } else {
          result.push(node);
        }
      }

      return result;
    }

    tree.children = walk(tree.children);
  };
}
