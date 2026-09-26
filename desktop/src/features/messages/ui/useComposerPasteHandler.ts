import * as React from "react";
import type { Editor } from "@tiptap/react";
import { handleAgentSnapshotPaste } from "@/features/messages/lib/agentSnapshotClipboard";
import type { BlobDescriptor } from "@/shared/api/tauri";
import { hasMentionClipboardHtml } from "@/features/messages/lib/normalizeMentionClipboard";
import { handleMentionClipboardPaste } from "@/features/messages/lib/mentionClipboardPaste";
import type { BindPastedMentionIdentities } from "@/features/messages/lib/mentionPasteBinding";
import { getBuzzCodeBlockClipboardText } from "@/shared/lib/codeBlockClipboard";
import {
  clipboardImageFile,
  isPasteShortcut,
  shouldReadNativeClipboardImage,
} from "@/features/messages/lib/nativeClipboardImage";
import { readImageFromSystemClipboard } from "@/shared/api/tauriMedia";

/** How long after Ctrl/Cmd+V a paste event still counts as keyboard-initiated. */
const PASTE_SHORTCUT_WINDOW_MS = 1000;

async function readClipboardImageFile(): Promise<File | null> {
  const png = await readImageFromSystemClipboard();
  return png ? clipboardImageFile(png) : null;
}

export function useComposerPasteHandler(options: {
  editor: Editor | null;
  /**
   * Teaches the composer each `name → pubkey` pair a Buzz copy carried, once
   * trusted state vouches for it. Without it, a paste binds nothing.
   */
  bindMentionIdentities?: BindPastedMentionIdentities;
  scrollToBottom: () => void;
  setPendingImeta: (
    update: (current: BlobDescriptor[]) => BlobDescriptor[],
  ) => void;
  uploadDeferredFile: (
    readFile: () => Promise<File | null>,
  ) => Promise<unknown>;
  uploadFile: (file: File) => Promise<unknown>;
}) {
  const uploadFileRef = React.useRef(options.uploadFile);
  uploadFileRef.current = options.uploadFile;
  const uploadDeferredFileRef = React.useRef(options.uploadDeferredFile);
  uploadDeferredFileRef.current = options.uploadDeferredFile;
  const pasteShortcutAtRef = React.useRef(0);
  const bindMentionIdentitiesRef = React.useRef(options.bindMentionIdentities);
  bindMentionIdentitiesRef.current = options.bindMentionIdentities;
  React.useEffect(() => {
    const editor = options.editor;
    if (!editor) return;
    const dom = editor.view.dom;
    const onKeyDown = (event: KeyboardEvent) => {
      if (isPasteShortcut(event)) pasteShortcutAtRef.current = Date.now();
    };
    dom.addEventListener("keydown", onKeyDown, true);
    editor.setOptions({
      editorProps: {
        ...editor.options.editorProps,
        handlePaste: (view, event) => {
          const mediaItem = Array.from(event.clipboardData?.items ?? []).find(
            (item) => item.kind === "file",
          );
          if (mediaItem) {
            const file = mediaItem.getAsFile();
            if (file) void uploadFileRef.current(file);
            return true;
          }
          const codeBlockText = getBuzzCodeBlockClipboardText(
            event.clipboardData,
          );
          if (codeBlockText !== null) {
            event.preventDefault();
            editor
              .chain()
              .focus()
              .insertContent([
                {
                  type: "codeBlock",
                  content:
                    codeBlockText.length > 0
                      ? [{ type: "text", text: codeBlockText }]
                      : [],
                },
                { type: "paragraph" },
              ])
              .run();
            options.scrollToBottom();
            return true;
          }
          if (handleAgentSnapshotPaste(event, options.setPendingImeta))
            return true;
          const clipboardData = event.clipboardData;
          const html = clipboardData?.getData("text/html");
          if (clipboardData && html && hasMentionClipboardHtml(html)) {
            if (clipboardData.getData("text/plain").includes("\n")) {
              options.scrollToBottom();
            }
            return handleMentionClipboardPaste({
              bindMentionIdentities: bindMentionIdentitiesRef.current,
              clipboardData,
              preventDefault: () => event.preventDefault(),
              view,
            });
          }
          const keyboardInitiated =
            Date.now() - pasteShortcutAtRef.current < PASTE_SHORTCUT_WINDOW_MS;
          if (
            shouldReadNativeClipboardImage(clipboardData, keyboardInitiated)
          ) {
            event.preventDefault();
            void uploadDeferredFileRef.current(readClipboardImageFile);
            return true;
          }
          if ((clipboardData?.getData("text/plain") ?? "").includes("\n"))
            options.scrollToBottom();
          return false;
        },
      },
    });
    return () => dom.removeEventListener("keydown", onKeyDown, true);
  }, [options.editor, options.scrollToBottom, options.setPendingImeta]);
}
