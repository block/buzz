/**
 * Guard that keeps mention resolution out of code.
 *
 * Mention detection reads a plain-text projection of the document, where
 * fences and backticks are already gone — so `@Name` inside code looks
 * exactly like `@Name` in prose. Committing a mention there would swallow
 * the keystroke and rewrite the code text to the canonical display name,
 * so the composer asks this guard before treating Space as a commit.
 */
import type { Editor } from "@tiptap/react";

/** The editor surface this guard reads — narrow so tests can stub it. */
type CodeContextEditor = Pick<Editor, "isActive" | "state">;

/**
 * True when the mention being typed lives inside a fenced code block or an
 * inline code span.
 */
export function isMentionCodeContext(
  editor: CodeContextEditor | null | undefined,
): boolean {
  if (!editor) return false;
  if (editor.isActive("codeBlock") || editor.isActive("code")) return true;
  // A closing backtick converts the span to a code mark and clears the
  // stored mark, so `isActive("code")` already reads false with the caret
  // parked right after the span it just created. The typed mention is still
  // inside that span, so inspect the text immediately before the caret.
  const { $from, empty } = editor.state.selection;
  if (!empty) return false;
  return (
    $from.nodeBefore?.marks.some((mark) => mark.type.name === "code") ?? false
  );
}
