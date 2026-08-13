import { Node } from "@tiptap/core";

/**
 * Composer text node with Markdown serialization that leaves angle brackets
 * untouched. tiptap-markdown's stock text serializer HTML-escapes them,
 * which makes literal inline-code Markdown render `&lt;`/`&gt;` visibly.
 */
export const ComposerText = Node.create({
  name: "text",
  group: "inline",

  addStorage() {
    return {
      markdown: {
        serialize(
          state: { text: (value: string) => void },
          node: { text: string },
        ) {
          state.text(node.text);
        },
        parse: {
          // handled by markdown-it
        },
      },
    };
  },
});
