/**
 * Rehype plugin that highlights text matching a search query by wrapping
 * matches in `<mark>` elements during the HAST (HTML AST) phase.
 *
 * This runs inside the react-markdown pipeline, so it works correctly with
 * ReactMarkdown's architecture — no post-render tree walking needed.
 */

import { splitSearchMatches } from "@/features/search/lib/searchMatch";
import { SEARCH_MATCH_HIGHLIGHT_CLASS } from "@/shared/lib/searchHighlightStyle";

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

export default function rehypeSearchHighlight({ query }: { query: string }) {
  return (tree: HastRoot) => {
    function walk(nodes: HastNode[]): HastNode[] {
      const result: HastNode[] = [];

      for (const node of nodes) {
        if (isText(node)) {
          const parts = splitSearchMatches(node.value, query);
          if (!parts.some((part) => part.isMatch)) {
            result.push(node);
            continue;
          }

          for (const part of parts) {
            if (part.isMatch) {
              result.push({
                type: "element",
                tagName: "mark",
                properties: {
                  className: SEARCH_MATCH_HIGHLIGHT_CLASS,
                  "data-search-match": "true",
                },
                children: [{ type: "text", value: part.text }],
              });
            } else {
              result.push({ type: "text", value: part.text });
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
