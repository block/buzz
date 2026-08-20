import * as React from "react";

import type { UseMentionsResult } from "@/features/messages/lib/useMentions";
import type { UseRichTextEditorResult } from "@/features/messages/lib/useRichTextEditor";

export function useComposerMentionPicker({
  mentions,
  richText,
  setIsEmojiPickerOpen,
}: {
  mentions: UseMentionsResult;
  richText: UseRichTextEditorResult;
  setIsEmojiPickerOpen: (open: boolean) => void;
}) {
  const {
    cancelMentionAutocomplete,
    isMentionOpen,
    openMentionPicker,
    updateMentionQuery,
  } = mentions;
  const { editor, focus, getPlainTextAndCursor } = richText;
  return React.useCallback(
    (insertTrigger = true) => {
      if (!editor) return;
      const { text, cursor } = getPlainTextAndCursor();
      if (!insertTrigger) {
        if (isMentionOpen) {
          cancelMentionAutocomplete();
          setIsEmojiPickerOpen(false);
          focus();
          return;
        }
        openMentionPicker(cursor, "first-agent");
        setIsEmojiPickerOpen(false);
        focus();
        return;
      }
      const beforeCursor = text.slice(0, cursor);
      if (/(?:^|[\s])@[^\s]*$/.test(beforeCursor)) {
        updateMentionQuery(text, cursor);
        focus();
        return;
      }
      const previousChar = text.slice(0, cursor).slice(-1);
      const prefix =
        cursor > 0 && previousChar && !/\s/.test(previousChar) ? " @" : "@";
      editor.chain().focus().insertContent(prefix).run();
      setIsEmojiPickerOpen(false);
      const updated = getPlainTextAndCursor();
      updateMentionQuery(updated.text, updated.cursor);
    },
    [
      cancelMentionAutocomplete,
      editor,
      focus,
      getPlainTextAndCursor,
      isMentionOpen,
      openMentionPicker,
      setIsEmojiPickerOpen,
      updateMentionQuery,
    ],
  );
}
