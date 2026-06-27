import { getBuzzCodeBlockClipboardText } from "@/shared/lib/codeBlockClipboard";
import {
  hasMentionClipboardHtml,
  normalizeMentionClipboardHtml,
} from "@/features/messages/lib/normalizeMentionClipboard";
import { buildTaskLinkPasteContent } from "@/features/messages/lib/taskLinkPasteContent";
import type { MediaUploadController } from "@/features/messages/lib/useMediaUpload";
import type { UseRichTextEditorResult } from "@/features/messages/lib/useRichTextEditor";

type PasteView = {
  pasteHTML: (html: string) => void;
};

type ComposerPasteHandlerOptions = {
  agentConversationTitleForHref?: (href: string) => string | undefined;
  enableAgentConversationLinks?: boolean;
  editor: NonNullable<UseRichTextEditorResult["editor"]>;
  scrollComposerToBottom: () => void;
  uploadFile: MediaUploadController["uploadFile"];
};

export function createMessageComposerPasteHandler({
  agentConversationTitleForHref,
  enableAgentConversationLinks = true,
  editor,
  scrollComposerToBottom,
  uploadFile,
}: ComposerPasteHandlerOptions) {
  return (view: PasteView, event: ClipboardEvent) => {
    const items = Array.from(event.clipboardData?.items ?? []);
    const mediaItem = items.find((item) => item.kind === "file");
    if (mediaItem) {
      const file = mediaItem.getAsFile();
      if (file) {
        void uploadFile(file);
      }
      return true;
    }

    const codeBlockText = getBuzzCodeBlockClipboardText(event.clipboardData);
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
      scrollComposerToBottom();
      return true;
    }

    const html = event.clipboardData?.getData("text/html");
    if (html && hasMentionClipboardHtml(html)) {
      const cleanHtml = normalizeMentionClipboardHtml(html);
      event.preventDefault();
      view.pasteHTML(cleanHtml);
      return true;
    }

    const plainText = event.clipboardData?.getData("text/plain") ?? "";
    const taskLinkPasteContent =
      !enableAgentConversationLinks ||
      plainText.includes("\n") ||
      plainText.trim().length === 0
        ? null
        : buildTaskLinkPasteContent(plainText, agentConversationTitleForHref);
    if (taskLinkPasteContent) {
      event.preventDefault();
      editor.chain().focus().insertContent(taskLinkPasteContent).run();
      return true;
    }

    if (plainText.includes("\n")) {
      scrollComposerToBottom();
    }

    return false;
  };
}
