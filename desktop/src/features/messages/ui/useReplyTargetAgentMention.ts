import * as React from "react";

import type { ReplyTargetAgent } from "@/features/messages/lib/replyTargetAgentMention";
import type { UseMentionsResult } from "@/features/messages/lib/useMentions";
import type { UseRichTextEditorResult } from "@/features/messages/lib/useRichTextEditor";
import { normalizePubkey } from "@/shared/lib/pubkey";

/**
 * Auto-insert a visible `@AgentName` mention when the composer's reply target
 * is an agent-authored message, so plain thread replies carry the agent p-tag
 * the ACP harness wakes on. The chip is real composer text: the user can
 * delete it to opt out (tracked per target so it is not re-inserted), and it
 * flows through the normal send pipeline like a hand-typed mention.
 *
 * Insertion never touches user-authored text — it only writes into an empty
 * composer, or replaces a previous auto-inserted chip that is still the
 * composer's entire content.
 */
export function useReplyTargetAgentMention({
  isEditing,
  mentions,
  richText,
  target,
}: {
  isEditing: boolean;
  mentions: UseMentionsResult;
  richText: UseRichTextEditorResult;
  target: ReplyTargetAgent | null;
}) {
  const targetRef = React.useRef(target);
  targetRef.current = target;
  const isEditingRef = React.useRef(isEditing);
  isEditingRef.current = isEditing;
  const isRestoringRef = React.useRef(false);
  const isSubmittingRef = React.useRef(false);
  // Exact text of the auto-inserted chip while it is still the composer's
  // entire content; null once the user types around it or removes it.
  const lastInsertedTextRef = React.useRef<string | null>(null);
  const lastInsertedTargetIdRef = React.useRef<string | null>(null);
  const dismissedTargetIdRef = React.useRef<string | null>(null);

  const insert = React.useCallback(() => {
    const capturedTarget = targetRef.current;
    const current = richText.getPlainTextAndCursor().text;
    const untouchedAutoMention =
      lastInsertedTextRef.current !== null &&
      current === lastInsertedTextRef.current;

    if (!capturedTarget) {
      // The reply target moved off an agent. Clear the chip only while it is
      // still the composer's entire, untouched content, and always drop the
      // bookkeeping so stale chip text from a previous target cannot leak
      // into later comparisons.
      if (untouchedAutoMention) {
        isRestoringRef.current = true;
        richText.replacePlainTextRange(0, current.length, "");
        isRestoringRef.current = false;
        mentions.cancelMentionAutocomplete();
      }
      lastInsertedTextRef.current = null;
      lastInsertedTargetIdRef.current = null;
      return;
    }

    if (
      isEditingRef.current ||
      dismissedTargetIdRef.current === capturedTarget.targetId
    ) {
      return;
    }

    const present = new Set(
      mentions.extractMentionPubkeys(current).map(normalizePubkey),
    );
    if (present.has(capturedTarget.pubkey)) {
      lastInsertedTargetIdRef.current = capturedTarget.targetId;
      return;
    }

    if (current.length > 0 && !untouchedAutoMention) {
      return;
    }

    const displayName =
      mentions.getMentionDisplayName(capturedTarget.pubkey) ??
      capturedTarget.displayName;

    isRestoringRef.current = true;
    const edit = mentions.insertResolvedMention({
      displayName,
      pubkey: capturedTarget.pubkey,
      isAgent: true,
      replaceFromOffset: 0,
      replaceToOffset: current.length,
    });
    richText.replacePlainTextRange(
      edit.replaceFromOffset,
      edit.replaceToOffset,
      edit.insertText,
    );
    isRestoringRef.current = false;
    lastInsertedTextRef.current = edit.insertText;
    lastInsertedTargetIdRef.current = capturedTarget.targetId;
    // Programmatic transition, not an authored query — drop any autocomplete
    // work the editor update scheduled.
    mentions.cancelMentionAutocomplete();
  }, [mentions, richText]);

  const reconcile = React.useCallback(
    (text: string) => {
      if (
        isRestoringRef.current ||
        isSubmittingRef.current ||
        isEditingRef.current
      ) {
        return;
      }
      const capturedTarget = targetRef.current;
      if (
        !capturedTarget ||
        lastInsertedTargetIdRef.current !== capturedTarget.targetId
      ) {
        return;
      }
      // The user typed around the chip — the content is no longer an
      // untouched auto-mention, so target switches must not rewrite it.
      if (
        lastInsertedTextRef.current !== null &&
        text !== lastInsertedTextRef.current
      ) {
        lastInsertedTextRef.current = null;
      }
      // The user removed the chip — honor the opt-out for this target.
      const present = new Set(
        mentions.extractMentionPubkeys(text).map(normalizePubkey),
      );
      if (!present.has(capturedTarget.pubkey)) {
        dismissedTargetIdRef.current = capturedTarget.targetId;
      }
    },
    [mentions.extractMentionPubkeys],
  );

  const insertRef = React.useRef(insert);
  insertRef.current = insert;
  const scheduleInsert = React.useCallback(
    () => requestAnimationFrame(() => insertRef.current()),
    [],
  );

  const targetId = target?.targetId ?? null;
  React.useEffect(() => {
    if (
      dismissedTargetIdRef.current !== null &&
      dismissedTargetIdRef.current !== targetId
    ) {
      dismissedTargetIdRef.current = null;
    }
    const frame = scheduleInsert();
    return () => cancelAnimationFrame(frame);
  }, [targetId, scheduleInsert]);

  return {
    beginSubmit: () => {
      isSubmittingRef.current = true;
    },
    /**
     * True while the composer's entire content is still the auto-inserted
     * chip. Draft persistence uses this to avoid stranding a phantom
     * "@Agent " draft when the user opens an agent thread and leaves
     * without typing.
     */
    isUntouchedAutoMention: () =>
      lastInsertedTextRef.current !== null &&
      richText.getPlainTextAndCursor().text === lastInsertedTextRef.current,
    endSubmit: () => {
      isSubmittingRef.current = false;
      // The composer was cleared by the send — re-arm so the next reply in
      // the same thread gets the chip again.
      lastInsertedTextRef.current = null;
      lastInsertedTargetIdRef.current = null;
      dismissedTargetIdRef.current = null;
      scheduleInsert();
    },
    reconcile,
    scheduleInsert,
  };
}
