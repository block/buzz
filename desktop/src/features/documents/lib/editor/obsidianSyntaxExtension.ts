/**
 * Decorations for Obsidian's inline and block syntax, plus heading extraction
 * for the outline panel and click-to-toggle task checkboxes.
 *
 * All of it is decoration-driven. Nothing here becomes a schema node, so the
 * underlying markdown is unchanged and notes using these constructs still pass
 * the round-trip guard. That is the whole design constraint — see
 * `obsidianSyntax.ts`.
 */
import { Extension } from "@tiptap/core";
import { Plugin, PluginKey } from "@tiptap/pm/state";
import type { Node as ProseMirrorNode } from "@tiptap/pm/model";
import { Decoration, DecorationSet } from "@tiptap/pm/view";

import {
  findBlockId,
  findComments,
  findHighlights,
  findTags,
  parseCallout,
  type OutlineHeading,
} from "@/features/documents/lib/obsidianSyntax";

export const obsidianSyntaxKey = new PluginKey("documentsObsidianSyntax");

export type ObsidianSyntaxStorage = {
  /** Headings in document order, refreshed on every doc change. */
  headings: OutlineHeading[];
  /**
   * Called when a `#tag` is clicked.
   *
   * Nothing supplies this yet — a tag click has nowhere to lead until the vault
   * gains a search, which v1 deliberately leaves out. It stays as the seam that
   * search will plug into; the click handler already falls through when it is
   * null, and the CSS does not advertise tags as clickable in the meantime.
   */
  onTagClick: ((tag: string) => void) | null;
  /** Notified whenever `headings` changes, so React can re-render. */
  onHeadingsChange: ((headings: OutlineHeading[]) => void) | null;
};

/** `- [ ]` / `- [x]` at the start of a paragraph inside a list item. */
const TASK_PATTERN = /^(\s*(?:[-*+]|\d+\.)\s+)\[([ xX])\]\s/;

function decorateText(
  text: string,
  from: number,
  decorations: Decoration[],
): void {
  for (const match of findHighlights(text)) {
    decorations.push(
      Decoration.inline(
        from + match.index,
        from + match.index + match.raw.length,
        {
          class: "obsidian-highlight",
        },
      ),
    );
  }
  for (const match of findComments(text)) {
    decorations.push(
      Decoration.inline(
        from + match.index,
        from + match.index + match.raw.length,
        {
          class: "obsidian-comment",
        },
      ),
    );
  }
  const blockId = findBlockId(text);
  if (blockId) {
    decorations.push(
      Decoration.inline(
        from + blockId.index,
        from + blockId.index + blockId.raw.length,
        { class: "obsidian-block-id", "data-block-id": blockId.content },
      ),
    );
  }
  for (const match of findTags(text)) {
    decorations.push(
      Decoration.inline(
        from + match.index,
        from + match.index + match.raw.length,
        {
          class: "obsidian-tag",
          "data-tag": match.content,
        },
      ),
    );
  }
}

function build(
  doc: ProseMirrorNode,
  storage: ObsidianSyntaxStorage,
): { decorations: DecorationSet; headings: OutlineHeading[] } {
  const decorations: Decoration[] = [];
  const headings: OutlineHeading[] = [];

  doc.descendants((node, position) => {
    if (node.type.name === "heading") {
      headings.push({
        level: Number(node.attrs.level ?? 1),
        position,
        text: node.textContent,
      });
      return;
    }

    if (node.type.name === "blockquote") {
      // Only the first line of a blockquote carries the callout marker.
      const [firstLine = ""] = node.textContent.split("\n");
      const callout = parseCallout(`> ${firstLine}`);
      if (callout) {
        decorations.push(
          Decoration.node(position, position + node.nodeSize, {
            class: `callout callout-${callout.canonical}`,
            "data-callout": callout.type,
          }),
        );
      }
      return;
    }

    if (node.isText && node.text) {
      decorateText(node.text, position, decorations);
      return;
    }

    if (node.type.name === "paragraph" && TASK_PATTERN.test(node.textContent)) {
      decorations.push(
        Decoration.node(position, position + node.nodeSize, {
          class: "obsidian-task",
        }),
      );
    }
  });

  storage.headings = headings;
  return { decorations: DecorationSet.create(doc, decorations), headings };
}

export const ObsidianSyntaxExtension = Extension.create({
  name: "documentsObsidianSyntax",

  addStorage(): ObsidianSyntaxStorage {
    return { headings: [], onHeadingsChange: null, onTagClick: null };
  },

  addProseMirrorPlugins() {
    const extension = this;

    return [
      new Plugin({
        key: obsidianSyntaxKey,
        props: {
          decorations(state) {
            return obsidianSyntaxKey.getState(state) as
              | DecorationSet
              | undefined;
          },
          handleClick(view, position, event) {
            const element = event.target as HTMLElement | null;
            const storage = extension.storage as ObsidianSyntaxStorage;

            if (element?.classList.contains("obsidian-tag")) {
              const tag = element.getAttribute("data-tag");
              // Without a handler, fall through so the click still places the
              // caret — swallowing it would make tagged text unselectable.
              if (tag && storage.onTagClick) {
                event.preventDefault();
                storage.onTagClick(tag);
                return true;
              }
            }

            // Toggle a task checkbox by rewriting its marker text.
            //
            // Onyx hit-tests this with a hardcoded `clickX > 30`, measured
            // against its own CSS — a number that breaks under Cmd +/- zoom.
            // Matching the rendered marker element instead survives any
            // font size.
            const taskElement = element?.closest(".obsidian-task");
            if (!taskElement) return false;

            const resolved = view.state.doc.resolve(position);
            const paragraph = resolved.parent;
            if (!paragraph.isTextblock) return false;

            const match = TASK_PATTERN.exec(paragraph.textContent);
            if (!match) return false;

            // Only the checkbox itself toggles; clicking the label text should
            // place the cursor as normal.
            const start = resolved.start();
            const boxFrom = start + match[1].length + 1;
            if (position < start + match[1].length || position > boxFrom + 2) {
              return false;
            }

            const next = match[2] === " " ? "x" : " ";
            event.preventDefault();
            view.dispatch(view.state.tr.insertText(next, boxFrom, boxFrom + 1));
            return true;
          },
        },
        state: {
          init(_config, state) {
            return build(state.doc, extension.storage as ObsidianSyntaxStorage)
              .decorations;
          },
          apply(transaction, previous) {
            if (
              !transaction.docChanged &&
              !transaction.getMeta(obsidianSyntaxKey)
            ) {
              return previous;
            }
            const storage = extension.storage as ObsidianSyntaxStorage;
            const { decorations, headings } = build(transaction.doc, storage);
            storage.onHeadingsChange?.(headings);
            return decorations;
          },
        },
      }),
    ];
  },
});
