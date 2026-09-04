import * as React from "react";
import type { Editor } from "@tiptap/react";
import { toast } from "sonner";
import { handleAgentSnapshotPaste } from "@/features/messages/lib/agentSnapshotClipboard";
import {
  clipboardPasteErrorMessage,
  firstClipboardFile,
  shouldReadNativeClipboardImage,
} from "@/features/messages/lib/clipboardFile";
import type { BlobDescriptor } from "@/shared/api/tauri";
import { readClipboardImage } from "@/shared/api/tauriMedia";
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
  const uploadNativeClipboardImage = React.useCallback(async () => {
    try {
      const bytes = await readClipboardImage();
      // Copy into a browser-owned ArrayBuffer: Tauri's Uint8Array is typed as
      // ArrayBufferLike, while File accepts ArrayBuffer.
      const imageBytes = new Uint8Array(bytes.byteLength);
      imageBytes.set(bytes);
      await uploadFileRef.current(
        new File([imageBytes.buffer], "clipboard-image.png", {
          type: "image/png",
        }),
      );
    } catch (error) {
      toast.error(clipboardPasteErrorMessage(error));
    }
  }, []);

  React.useEffect(() => {
    const editor = options.editor;
    if (!editor) return;
    editor.setOptions({
      editorProps: {
        ...editor.options.editorProps,
        handlePaste: (view, event) => {
          const mediaFile = firstClipboardFile(event.clipboardData);
          if (mediaFile) {
            void uploadFileRef.current(mediaFile);
            return true;
          }
          if (shouldReadNativeClipboardImage(event.clipboardData)) {
            event.preventDefault();
            void uploadNativeClipboardImage();
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
  }, [
    options.editor,
    options.scrollToBottom,
    options.setPendingImeta,
    uploadNativeClipboardImage,
  ]);

  // WebKitGTK can consume image clipboard events before ProseMirror reaches
  // editorProps.handlePaste. Intercept native-image candidates at the editor
  // DOM capture phase.
  React.useEffect(() => {
    const editor = options.editor;
    if (!editor) return;
    const handleNativeImagePaste = (event: ClipboardEvent) => {
      if (!shouldReadNativeClipboardImage(event.clipboardData)) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      void uploadNativeClipboardImage();
    };
    const dom = editor.view.dom;
    dom.addEventListener("paste", handleNativeImagePaste, true);
    return () => dom.removeEventListener("paste", handleNativeImagePaste, true);
  }, [options.editor, uploadNativeClipboardImage]);
}
