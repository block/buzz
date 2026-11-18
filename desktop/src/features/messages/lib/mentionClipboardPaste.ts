import type { EditorView } from "@tiptap/pm/view";

import { getBuzzCopyKind } from "./mentionClipboard";
import type { BindPastedMentionIdentities } from "./mentionPasteBinding";
import { normalizeMentionClipboardContent } from "./normalizeMentionClipboard";

/**
 * Insert `text` through ProseMirror's plain-text paste pipeline.
 *
 * `view.pasteText` re-enters `handlePaste` with the original event, so the
 * clipboard data is rebuilt with only the plain flavor — otherwise the HTML
 * branch would claim the paste again, forever. That re-entry also means this
 * function's own call carries no identity records, so it cannot double-bind
 * the identities its caller is about to hand over.
 */
function pastePlainText(view: EditorView, text: string): void {
  const clipboardData = new DataTransfer();
  clipboardData.setData("text/plain", text);
  view.pasteText(text, new ClipboardEvent("paste", { clipboardData }));
}

/**
 * Paste clipboard HTML that carries Buzz mention markers.
 *
 * Content follows the flavor the copy declared:
 *
 * - `markdown` — the copy's plain flavor *is* the Markdown source, so insert
 *   it through the text pipeline and TipTap parses `**bold**` exactly as it
 *   does for any other plain paste.
 * - `rich` (or legacy Buzz HTML with no marker) — keep the HTML path, with
 *   chip wrappers flattened to sigil-bearing text.
 *
 * Identity rides along: binding the records is what makes a pasted multi-word
 * name known to the composer, so its chip re-lights and the send path recovers
 * the original pubkey. Each branch is judged against the content *it* inserts
 * — the plain flavor is not evidence for what the HTML branch shows, and vice
 * versa — and the range it inserted into is handed over, so a verification
 * that lands later can tell the mention tokens this paste put on screen from
 * whatever the user typed next. `bindPastedMentionIdentities` owns the rest; a
 * composer that passes no binder inserts readable text and binds nothing.
 */
export function handleMentionClipboardPaste({
  bindMentionIdentities,
  clipboardData,
  preventDefault,
  view,
}: {
  bindMentionIdentities?: BindPastedMentionIdentities;
  clipboardData: DataTransfer;
  preventDefault: () => void;
  view: EditorView;
}): boolean {
  const html = clipboardData.getData("text/html");
  if (!html) return false;

  // Captured before the insertion; `pasteText`/`pasteHTML` dispatch
  // synchronously, so the caret afterwards closes the range this paste owns.
  const insertedFrom = view.state.selection.from;
  const bind = (insertedText: string) => {
    if (!bindMentionIdentities) return;
    bindMentionIdentities({
      html,
      insertedText,
      insertedRange: { from: insertedFrom, to: view.state.selection.to },
      view,
    });
  };

  const text = clipboardData.getData("text/plain");
  if (getBuzzCopyKind(html) === "markdown" && text) {
    preventDefault();
    pastePlainText(view, text);
    bind(text);
    return true;
  }

  const content = normalizeMentionClipboardContent(html);
  preventDefault();
  view.pasteHTML(content.html);
  bind(content.text);
  return true;
}
