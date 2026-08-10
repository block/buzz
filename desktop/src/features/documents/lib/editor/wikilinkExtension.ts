/**
 * Renders `[[wikilinks]]` in the editor and routes clicks on them.
 *
 * Onyx keeps the note index and the click handler in module-level mutable
 * globals with setter functions, which makes the editor a singleton — two
 * instances would overwrite each other's state. Here both live in extension
 * `storage`, which is per-instance.
 *
 * `storage` rather than `options` on purpose: options are baked at
 * `configure()` time and changing one means recreating the editor, whereas the
 * note index changes every time a file is created or renamed.
 */
import { Extension } from "@tiptap/core";
import { Plugin, PluginKey } from "@tiptap/pm/state";
import type { Node as ProseMirrorNode } from "@tiptap/pm/model";
import { Decoration, DecorationSet } from "@tiptap/pm/view";

import {
  resolveWikilink,
  type NoteIndex,
} from "@/features/documents/lib/noteIndex";
import { parseWikilinks } from "@/features/documents/lib/wikilinkSyntax";

export const wikilinkKey = new PluginKey("documentsWikilink");

export type WikilinkClickHandler = (input: {
  target: string;
  heading: string | null;
  blockId: string | null;
  /** Whether the note exists; `false` means the link is broken. */
  exists: boolean;
  /** Absolute path the link resolves to, or would create. */
  path: string | null;
}) => void;

export type WikilinkStorage = {
  noteIndex: NoteIndex | null;
  /** Path of the note being edited, for same-folder link resolution. */
  currentPath: string | null;
  onWikilinkClick: WikilinkClickHandler | null;
};

function buildDecorations(
  doc: ProseMirrorNode,
  storage: WikilinkStorage,
): DecorationSet {
  const decorations: Decoration[] = [];

  doc.descendants((node, position) => {
    if (!node.isText || !node.text) return;

    for (const link of parseWikilinks(node.text)) {
      const from = position + link.index;
      const to = from + link.raw.length;

      // Same-note anchors always "exist"; a named target might not.
      const resolved = link.target
        ? resolveWikilink(
            link.target,
            storage.currentPath ?? "",
            storage.noteIndex,
          )
        : null;
      const isBroken = Boolean(link.target) && resolved?.exists === false;

      decorations.push(
        Decoration.inline(from, to, {
          class: isBroken ? "wikilink wikilink-broken" : "wikilink",
          "data-block": link.blockId ?? "",
          "data-heading": link.heading ?? "",
          "data-target": link.target,
        }),
      );
    }
  });

  return DecorationSet.create(doc, decorations);
}

export const WikilinkExtension = Extension.create({
  name: "documentsWikilink",

  addStorage(): WikilinkStorage {
    return { currentPath: null, noteIndex: null, onWikilinkClick: null };
  },

  addProseMirrorPlugins() {
    const extension = this;

    return [
      new Plugin({
        key: wikilinkKey,
        props: {
          decorations(state) {
            return wikilinkKey.getState(state) as DecorationSet | undefined;
          },
          handleClick(_view, _pos, event) {
            const element = event.target as HTMLElement | null;
            if (!element?.classList.contains("wikilink")) return false;

            const target = element.getAttribute("data-target") ?? "";
            const heading = element.getAttribute("data-heading") || null;
            const blockId = element.getAttribute("data-block") || null;
            const storage = extension.storage as WikilinkStorage;

            const resolved = target
              ? resolveWikilink(
                  target,
                  storage.currentPath ?? "",
                  storage.noteIndex,
                )
              : null;

            event.preventDefault();
            storage.onWikilinkClick?.({
              blockId,
              exists: resolved?.exists ?? false,
              heading,
              path: resolved?.path ?? null,
              target,
            });
            return true;
          },
        },
        state: {
          init: (_config, state) =>
            buildDecorations(state.doc, extension.storage as WikilinkStorage),
          apply(transaction, previous) {
            // The note index changed (a file was created, renamed or deleted),
            // so broken-link styling must be recomputed even though the
            // document itself did not change.
            if (transaction.getMeta(wikilinkKey)) {
              return buildDecorations(
                transaction.doc,
                extension.storage as WikilinkStorage,
              );
            }
            if (!transaction.docChanged) return previous;
            return buildDecorations(
              transaction.doc,
              extension.storage as WikilinkStorage,
            );
          },
        },
      }),
    ];
  },
});
