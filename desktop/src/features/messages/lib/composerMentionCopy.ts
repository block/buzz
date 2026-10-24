import type { EditorView } from "@tiptap/pm/view";

import {
  buildMentionClipboardHtml,
  type MentionIdentity,
} from "./mentionClipboard";

/**
 * Copy / cut out of the composer.
 *
 * A composer mention is plain `@Name ` text decorated from a known-names list;
 * the pubkey lives outside the document. A default copy therefore moves the
 * words but not the identity, so moving a draft between channels silently
 * re-resolves (or drops) who was tagged. Writing the identity sidecar here
 * keeps the exact pubkey attached to the draft.
 */
export function handleComposerMentionCopy({
  event,
  identities,
  isCut,
  view,
}: {
  event: ClipboardEvent;
  identities: readonly MentionIdentity[];
  isCut: boolean;
  view: EditorView;
}): boolean {
  const clipboardData = event.clipboardData;
  if (!clipboardData || view.state.selection.empty) return false;

  const slice = view.state.selection.content();
  // The same serializer ProseMirror would have used, so the plain flavor is
  // byte-identical to a default copy (Markdown syntax included).
  const serializeText = view.someProp("clipboardTextSerializer");
  const text = serializeText
    ? serializeText(slice, view)
    : slice.content.textBetween(0, slice.content.size, "\n\n");
  if (!text) return false;

  const html = buildMentionClipboardHtml({ identities, text });
  // No known mention in the selection — leave the copy on its default path
  // rather than replacing ProseMirror's richer HTML flavor for no gain.
  if (!html) return false;

  event.preventDefault();
  clipboardData.setData("text/plain", text);
  clipboardData.setData("text/html", html);
  if (isCut) {
    view.dispatch(view.state.tr.deleteSelection().scrollIntoView());
  }
  return true;
}
