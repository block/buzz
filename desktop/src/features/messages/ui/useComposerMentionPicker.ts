import * as React from "react";

import type { UseMentionsResult } from "@/features/messages/lib/useMentions";
import type { UseRichTextEditorResult } from "@/features/messages/lib/useRichTextEditor";

export function useComposerMentionPicker({
  mentions,
  onTurnOffAutoPinConfirmation,
  richText,
  setIsEmojiPickerOpen,
}: {
  mentions: UseMentionsResult;
  onTurnOffAutoPinConfirmation: () => void;
  richText: UseRichTextEditorResult;
  setIsEmojiPickerOpen: (open: boolean) => void;
}) {
  const {
    cancelMentionAutocomplete,
    isMentionOpen,
    openMentionPicker: setMentionPickerOpen,
    updateMentionQuery,
  } = mentions;
  const { editor, focus, getPlainTextAndCursor } = richText;
  const openMentionPicker = React.useCallback(
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
        setMentionPickerOpen(cursor, "first-agent");
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
      setMentionPickerOpen,
      setIsEmojiPickerOpen,
      updateMentionQuery,
    ],
  );
  const openMentionSettings = React.useCallback(
    () => openMentionPicker(false),
    [openMentionPicker],
  );
  const revealMentionSettings = React.useCallback(() => {
    if (!editor) return;
    const { cursor } = getPlainTextAndCursor();
    setMentionPickerOpen(cursor, "first-agent");
    setIsEmojiPickerOpen(false);
    focus();
  }, [
    editor,
    focus,
    getPlainTextAndCursor,
    setIsEmojiPickerOpen,
    setMentionPickerOpen,
  ]);
  const turnOffAutoPinFromConfirmation = React.useCallback(() => {
    revealMentionSettings();
    onTurnOffAutoPinConfirmation();
  }, [onTurnOffAutoPinConfirmation, revealMentionSettings]);

  return {
    openMentionPicker,
    openMentionSettings,
    turnOff: turnOffAutoPinFromConfirmation,
  };
}
