import * as React from "react";

import { useCustomEmoji } from "@/features/custom-emoji/hooks";
import {
  buildSelfCorrectionEdit,
  parseSelfCorrection,
} from "@/features/messages/lib/selfCorrection";
import type { TimelineMessage } from "@/features/messages/types";

type EditSave = (
  content: string,
  mediaTags?: string[][],
  mentionPubkeys?: string[],
  eventId?: string,
) => Promise<void>;

/**
 * Send-path guard for IRC-style `s/old/new/` self-correction. Given the text a
 * user is about to send: if it is a well-formed substitution command it edits
 * the author's most recent message — via the existing edit path (`onEditSave`
 * with an explicit id) — and resolves `true`, telling the caller to skip the
 * literal send. Otherwise it resolves `false` and the caller sends normally.
 *
 * The command grammar and the media/emoji-preserving edit assembly live in the
 * pure, tested `selfCorrection` lib; this hook only wires React state. The
 * target lookup is the caller's own `findLastOwnEditable`, so "what counts as
 * editable" has a single definition shared with ArrowUp-to-edit.
 *
 * Everything is read through a ref so the returned callback is reference-stable
 * and never re-renders MessageComposer (see the React.memo note in AGENTS.md).
 */
export function useSelfCorrectingSend(params: {
  messages: TimelineMessage[];
  findLastOwnEditable: (
    candidates: TimelineMessage[],
  ) => TimelineMessage | null;
  onEditSave?: EditSave;
}): (content: string, mediaTags?: string[][]) => Promise<boolean> {
  const customEmoji = useCustomEmoji();
  const ref = React.useRef({ ...params, customEmoji });
  ref.current = { ...params, customEmoji };

  return React.useCallback(async (content, mediaTags) => {
    const { messages, findLastOwnEditable, onEditSave, customEmoji } =
      ref.current;
    // A draft with its own attachments is an ordinary send with a caption;
    // archived (edit-disabled) channels pass no `onEditSave`.
    if (!onEditSave || (mediaTags && mediaTags.length > 0)) return false;
    const command = parseSelfCorrection(content.trim());
    if (!command) return false;
    const target = findLastOwnEditable(messages);
    if (!target) return false;
    const edit = buildSelfCorrectionEdit(target, command, customEmoji);
    if (!edit) return false;
    // No new mention `p` tags — a typo-fix re-wakes nobody.
    await onEditSave(edit.content, edit.tags, undefined, target.id);
    return true;
  }, []);
}
