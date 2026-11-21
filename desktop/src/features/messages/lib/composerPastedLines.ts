import { Extension } from "@tiptap/core";
import { Fragment, type Node as PMNode, Slice } from "@tiptap/pm/model";
import { Plugin } from "@tiptap/pm/state";

/**
 * Keeps pasted and loaded text on the composer's line model.
 *
 * The composer has no table nodes and Enter sends, so a line break can only
 * be a hard break (what Shift+Enter inserts) and a blank line a paragraph
 * break. Two paths broke Markdown tables against that model:
 *
 * - Markdown parsing (a normal paste, drafts, editing a sent message) ran
 *   markdown-it's table rule. The schema dropped the resulting `<table>` and
 *   kept the cell text run together. With the rule off, rows stay lines, and
 *   `breaks: true` turns each line ending into a hard break.
 * - Plain-text paste (Mod+Shift+V) skips Markdown, and ProseMirror's default
 *   makes every line a paragraph, which sends as blank lines between rows.
 */
export const ComposerPastedLines = Extension.create({
  name: "composerPastedLines",

  addStorage() {
    return {
      markdown: {
        parse: {
          // biome-ignore lint/suspicious/noExplicitAny: markdown-it is untyped here
          setup(md: any) {
            md.disable("table");
          },
        },
      },
    };
  },

  addProseMirrorPlugins() {
    return [
      new Plugin({
        props: {
          clipboardTextParser(text, $context, plainText, view) {
            // A formatted paste belongs to tiptap-markdown's parser, whatever
            // order the plugins run in. ProseMirror moves on to the next
            // parser when this returns null, which its typing does not
            // express.
            if (!plainText) return null as unknown as Slice;

            const { schema } = view.state;
            const marks = $context.marks();
            const paragraphs = text.split(/(?:\r\n?|\n){2,}/).map((block) => {
              const inline: PMNode[] = [];
              block.split(/\r\n?|\n/).forEach((line, index) => {
                if (index > 0) inline.push(schema.nodes.hardBreak.create());
                if (line) inline.push(schema.text(line, marks));
              });
              return schema.nodes.paragraph.create(null, inline);
            });
            return Slice.maxOpen(Fragment.from(paragraphs));
          },
        },
      }),
    ];
  },
});
