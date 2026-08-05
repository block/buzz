/**
 * A headless TipTap editor used purely to answer "does this markdown survive
 * the editor?".
 *
 * This is the production side of the round-trip guard. It must use the *same*
 * extension set the real editor does, or the guard measures the wrong thing —
 * so both take their extensions from `vaultEditorExtensions()`.
 *
 * The instance is created lazily and reused: constructing a ProseMirror editor
 * costs real time, and opening a vault checks one note per tab.
 */
import { Editor } from "@tiptap/core";

import { vaultEditorExtensions } from "@/features/documents/lib/editor/vaultEditorExtensions";
import { toDiskMarkdown } from "@/features/documents/lib/markdownEscapes";

let probeEditor: Editor | null = null;

function getProbeEditor(): Editor {
  if (!probeEditor) {
    probeEditor = new Editor({
      content: "",
      extensions: vaultEditorExtensions(),
    });
  }
  return probeEditor;
}

/**
 * Parses `body` and serializes it straight back, applying the same
 * disk-normalization the save path uses so the comparison reflects what would
 * actually be written.
 */
export function reserializeMarkdown(body: string): string {
  const editor = getProbeEditor();
  editor.commands.setContent(body, { emitUpdate: false });
  const storage = editor.storage as {
    markdown?: { getMarkdown?: () => string };
  };
  const output = storage.markdown?.getMarkdown?.();
  if (typeof output !== "string") {
    throw new Error("tiptap-markdown storage is unavailable");
  }
  return toDiskMarkdown(output);
}

/**
 * Releases the probe editor. Called from the Documents view on unmount so a
 * ProseMirror instance and its DOM do not outlive the feature.
 */
export function destroyMarkdownProbe(): void {
  probeEditor?.destroy();
  probeEditor = null;
}
