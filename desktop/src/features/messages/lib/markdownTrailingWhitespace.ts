import type { Editor } from "@tiptap/core";
import { Selection } from "@tiptap/pm/state";

/**
 * The run of spaces/tabs at the very end of `markdown`, or `null` when there
 * is none.
 *
 * A whitespace-only string yields `null` on purpose: there is no content for
 * the whitespace to trail, and restoring it would leave a "blank" composer
 * that reads as non-empty (enabling Send, suppressing the placeholder).
 */
export function markdownTrailingWhitespace(markdown: string): string | null {
  return /[^ \t]([ \t]+)$/.exec(markdown)?.[1] ?? null;
}

/**
 * Re-append trailing spaces/tabs that a markdown parse dropped.
 *
 * `setContent` runs its argument through `tiptap-markdown`, and markdown-it
 * discards end-of-line whitespace — `"@Name "` parses to `<p>@Name</p>`. That
 * lone character is load-bearing: mention chips are inline decorations over
 * plain text (see `mentionHighlightExtension`), matched only when a boundary
 * follows the name. Restore the caret flush against `@Name` and the next
 * keystroke produces `@Nameabc`, which stops matching — the chip disappears
 * and `extractMentionPubkeys` no longer resolves the name, so the recipient is
 * silently dropped from the outgoing event.
 *
 * The whitespace survives *serialization* (drafts on disk keep it), so it is
 * recoverable here, at the single parse boundary, rather than at each of the
 * call sites that reload composer content: post-send refill, draft restore on
 * channel/thread switch, loading a message to edit, cancelling an edit, and
 * restoring after a failed send.
 *
 * Uses the `preventUpdate` meta so the repair is invisible to the user-edit
 * observers — the same mechanism `setContent`'s `emitUpdate: false` uses.
 * Without it the insert looks like a keystroke and re-opens the mention
 * autocomplete on every channel switch.
 */
export function restoreMarkdownTrailingWhitespace(
  editor: Editor,
  markdown: string,
): void {
  const trailing = markdownTrailingWhitespace(markdown);
  if (!trailing) return;

  const end = Selection.atEnd(editor.state.doc).from;
  // Idempotence guard: if the parser ever starts preserving the whitespace,
  // this must not double it.
  const existing = editor.state.doc.textBetween(
    Math.max(0, end - trailing.length),
    end,
  );
  if (existing === trailing) return;

  editor.view.dispatch(
    editor.state.tr.insertText(trailing, end).setMeta("preventUpdate", true),
  );
}
