import type { Editor } from "@tiptap/core";
import { TextSelection } from "@tiptap/pm/state";

/**
 * TipTap's markdown `setContent` strips trailing whitespace. Persistent agent
 * mentions (and autocomplete) rely on a trailing space so the next keystroke
 * does not extend the `@Name` token and collapse the mention chip.
 *
 * Parse the markdown body without its trailing run of spaces/tabs, then
 * re-attach that run with a raw `insertText` transaction (which preserves it).
 */
export function setEditorMarkdownPreservingTrailingWhitespace(
  editor: Editor,
  markdown: string,
  options?: { emitUpdate?: boolean; focusEnd?: boolean },
): void {
  const emitUpdate = options?.emitUpdate ?? true;
  const focusEnd = options?.focusEnd ?? false;
  const trailingWhitespace = markdown.match(/[ \t]+$/)?.[0] ?? "";
  const body = trailingWhitespace
    ? markdown.slice(0, -trailingWhitespace.length)
    : markdown;

  editor.commands.setContent(body, { emitUpdate });

  if (trailingWhitespace) {
    const insertAt = editor.state.doc.content.size - 1;
    let tr = editor.state.tr.insertText(trailingWhitespace, insertAt);
    tr = tr.setSelection(
      TextSelection.create(tr.doc, insertAt + trailingWhitespace.length),
    );
    // Mirror TipTap setContent({ emitUpdate: false }): suppress onUpdate and
    // keep programmatic restores out of undo history.
    if (!emitUpdate) {
      tr.setMeta("addToHistory", false);
      tr.setMeta("preventUpdate", true);
    }
    editor.view.dispatch(tr);
  }

  if (focusEnd) {
    editor.commands.focus("end");
  }
}
