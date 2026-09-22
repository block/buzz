import { Extension, type KeyboardShortcutCommand } from "@tiptap/core";
import type { ResolvedPos } from "@tiptap/pm/model";
import { Selection } from "@tiptap/pm/state";

import { isMacPlatform } from "@/shared/lib/platform";

/**
 * Bounds of the hard-break-delimited "line" containing `$from`.
 *
 * Chat composers use hard breaks for continuation lines, so a ProseMirror
 * block spans several visual lines. Emacs-style movement has to respect the
 * visual line, not the block.
 */
export function hardBreakLineBounds($from: ResolvedPos) {
  const parentStart = $from.start();
  let start = parentStart;
  let end = parentStart + $from.parent.content.size;

  $from.parent.forEach((node, offset) => {
    if (node.type.name !== "hardBreak") return;
    const breakPosition = parentStart + offset;
    if (breakPosition < $from.pos) {
      start = breakPosition + node.nodeSize;
    } else if (breakPosition >= $from.pos && end > breakPosition) {
      end = breakPosition;
    }
  });

  return { end, start };
}

/**
 * macOS text fields traditionally support a small set of Emacs-style Control
 * shortcuts. Keep movement and kill-line scoped to the current
 * hard-break-delimited line rather than the whole ProseMirror block.
 */
export const MacEmacsTextShortcuts = Extension.create({
  name: "macEmacsTextShortcuts",
  addKeyboardShortcuts() {
    const shortcuts: Record<string, KeyboardShortcutCommand> = {};
    if (!isMacPlatform()) {
      return shortcuts;
    }

    return {
      "Ctrl-a": ({ editor: ed }) => {
        const { $from } = ed.state.selection;
        if (!$from.parent.inlineContent) return false;
        return ed.commands.setTextSelection(hardBreakLineBounds($from).start);
      },
      "Ctrl-e": ({ editor: ed }) => {
        const { $from } = ed.state.selection;
        if (!$from.parent.inlineContent) return false;
        return ed.commands.setTextSelection(hardBreakLineBounds($from).end);
      },
      "Ctrl-b": ({ editor: ed }) => {
        const { empty, from } = ed.state.selection;
        if (!empty || from <= 0) return false;
        return ed.commands.setTextSelection(from - 1);
      },
      "Ctrl-f": ({ editor: ed }) => {
        const { empty, from } = ed.state.selection;
        if (!empty || from >= ed.state.doc.content.size) return false;
        return ed.commands.setTextSelection(from + 1);
      },
      "Ctrl-k": ({ editor: ed }) => {
        const { state, view } = ed;
        const { $from, empty, from, to } = state.selection;

        if (!empty) {
          return ed.commands.deleteSelection();
        }

        if ($from.parent.inlineContent) {
          const lineEnd = hardBreakLineBounds($from).end;
          if (from < lineEnd) {
            return ed.commands.deleteRange({ from, to: lineEnd });
          }

          const nodeAfter = $from.nodeAfter;
          if (nodeAfter?.type.name === "hardBreak") {
            return ed.commands.deleteRange({
              from,
              to: from + nodeAfter.nodeSize,
            });
          }
        }

        const blockEnd = $from.end();
        if (from < blockEnd) {
          return ed.commands.deleteRange({ from, to: blockEnd });
        }

        const nextSelection = Selection.findFrom(
          state.doc.resolve(to),
          1,
          true,
        );
        if (!nextSelection) return false;

        const transaction = state.tr.delete(to, nextSelection.from);
        view.dispatch(transaction.scrollIntoView());
        return true;
      },
    };
  },
});
