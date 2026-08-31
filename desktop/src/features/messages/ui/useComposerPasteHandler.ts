import * as React from "react";
import type { Editor } from "@tiptap/react";
import { handleAgentSnapshotPaste } from "@/features/messages/lib/agentSnapshotClipboard";
import type { BlobDescriptor } from "@/shared/api/tauri";
import {
  hasMentionClipboardHtml,
  normalizeMentionClipboardHtml,
} from "@/features/messages/lib/normalizeMentionClipboard";
import { getBuzzCodeBlockClipboardText } from "@/shared/lib/codeBlockClipboard";

export function useComposerPasteHandler(options: {
  editor: Editor | null;
  scrollToBottom: () => void;
  setPendingImeta: (
    update: (current: BlobDescriptor[]) => BlobDescriptor[],
  ) => void;
  uploadFile: (file: File) => Promise<unknown>;
}) {
  const uploadFileRef = React.useRef(options.uploadFile);
  uploadFileRef.current = options.uploadFile;
  React.useEffect(() => {
    const editor = options.editor;
    if (!editor) return;
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
          const html = event.clipboardData?.getData("text/html");
          if (html && hasMentionClipboardHtml(html)) {
            event.preventDefault();
            view.pasteHTML(normalizeMentionClipboardHtml(html));
            return true;
          }
          if ((event.clipboardData?.getData("text/plain") ?? "").includes("\n"))
            options.scrollToBottom();
          return false;
        },
      },
    });
  }, [options.editor, options.scrollToBottom, options.setPendingImeta]);
}
