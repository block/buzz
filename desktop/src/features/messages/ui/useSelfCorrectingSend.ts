import * as React from "react";

import { useCustomEmoji } from "@/features/custom-emoji/hooks";
import { resolveSelfCorrection } from "@/features/messages/lib/selfCorrectionEdit";
import type { TimelineMessage } from "@/features/messages/types";

type EditSaveById = (
  eventId: string,
  content: string,
  mediaTags?: string[][],
  mentionPubkeys?: string[],
) => Promise<void>;

/**
 * Returns a guard for the send path: given the message a user is about to send,
 * if its body is a well-formed `s/old/new/` command it edits the author's most
 * recent message (via the existing kind-40003 edit path) and resolves `true`,
 * telling the caller to skip the literal send. Otherwise resolves `false` and
 * the caller sends normally.
 *
 * Skipped when the draft carries its own attachments (then `s/a/b/` is a
 * caption, not a command) or when `onEditSaveById`/`currentPubkey` are absent.
 */
export function useSelfCorrectingSend(params: {
  messages: readonly TimelineMessage[];
  onEditSaveById?: EditSaveById;
  currentPubkey?: string;
}): (content: string, mediaTags: string[][] | undefined) => Promise<boolean> {
  const customEmoji = useCustomEmoji();
  const customEmojiRef = React.useRef(customEmoji);
  customEmojiRef.current = customEmoji;
  const messagesRef = React.useRef(params.messages);
  messagesRef.current = params.messages;
  const onEditSaveByIdRef = React.useRef(params.onEditSaveById);
  onEditSaveByIdRef.current = params.onEditSaveById;
  const currentPubkeyRef = React.useRef(params.currentPubkey);
  currentPubkeyRef.current = params.currentPubkey;

  return React.useCallback(async (content, mediaTags) => {
    const saveById = onEditSaveByIdRef.current;
    const currentPubkey = currentPubkeyRef.current;
    if (!saveById || !currentPubkey) return false;
    // A draft with its own attachments is an ordinary send with a caption.
    if (mediaTags && mediaTags.length > 0) return false;

    const edit = resolveSelfCorrection(
      content,
      messagesRef.current,
      currentPubkey,
      customEmojiRef.current,
    );
    if (!edit) return false;

    await saveById(edit.eventId, edit.content, edit.tags);
    return true;
  }, []);
}
