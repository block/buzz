import * as React from "react";

import type { MentionRecord } from "@/features/dev-mode/lib/mentionRecords";
import { useEditMessageMutation } from "@/features/messages/hooks";
import {
  buildOutgoingMessage,
  findSpoileredImetaMediaUrls,
  type ImetaMedia,
  imetaMediaFromTags,
  restoreImetaMediaDisplayLabels,
  stripImetaMediaLines,
} from "@/features/messages/lib/imetaMediaMarkdown";
import type { Channel, RelayEvent } from "@/shared/api/types";

/**
 * `e` on a selected own prompt card: the composer edits that message in
 * place of sending (a kind:40003 edit event — the same path as the standard
 * UI). The pre-edit draft is stashed on entry and restored on exit; save and
 * cancel both land in `stopEditing`.
 */
export function useMessageEditing({
  channel,
  roots,
  myPubkey,
  setInput,
  setBusy,
  setError,
}: {
  channel: Channel | null;
  /** Prompt roots with edits already applied (selectRootEvents). */
  roots: RelayEvent[];
  myPubkey: string | null;
  setInput: (value: string) => void;
  setBusy: (busy: boolean) => void;
  setError: (error: string | null) => void;
}) {
  const editMessageMutation = useEditMessageMutation(channel);
  const [editingRootId, setEditingRootId] = React.useState<string | null>(null);
  const preEditInputRef = React.useRef<string | null>(null);

  // The channel-switch draft stash reads this so the scope keeps its
  // pre-edit draft (not the abandoned edit buffer) when a switch cancels
  // an in-flight edit.
  const peekPreEditInput = React.useCallback(() => preEditInputRef.current, []);

  // Exit edit mode and put the stashed pre-edit draft back in the box.
  const stopEditing = React.useCallback(() => {
    setEditingRootId(null);
    if (preEditInputRef.current !== null) {
      setInput(preEditInputRef.current);
      preEditInputRef.current = null;
    }
  }, [setInput]);

  // Someone else's message (or one still pending) is not editable — silent
  // no-op. Returns whether edit mode started.
  const startEditing = React.useCallback(
    (root: RelayEvent | null, currentInput: string): boolean => {
      if (!root || !myPubkey || root.pubkey !== myPubkey || root.pending) {
        return false;
      }
      // The trailing attachment lines stay out of the editable text — they
      // are re-appended (with the original attachments) on save.
      const media = restoreImetaMediaDisplayLabels(
        root.content,
        imetaMediaFromTags(root.tags),
      );
      preEditInputRef.current = currentInput;
      setInput(stripImetaMediaLines(root.content, media));
      setEditingRootId(root.id);
      return true;
    },
    [myPubkey, setInput],
  );

  // The editing counterpart of the composer's Enter.
  const submitEdit = React.useCallback(
    (prompt: string, mentions: MentionRecord[], media: ImetaMedia[]) => {
      if (!editingRootId) return;
      const target = roots.find((root) => root.id === editingRootId);
      if (!target || !channel) {
        // The message vanished under us (deleted / channel gone).
        stopEditing();
        return;
      }
      // An emptied edit is a no-op — esc is the way out. (The standard UI
      // routes this to a delete confirmation; dev mode has no delete.)
      if (!prompt) return;
      setBusy(true);
      setError(null);
      void (async () => {
        try {
          // The edit's imeta set fully replaces the original's, so the
          // original attachments (plus any newly pasted ones) ride along.
          const originalMedia = restoreImetaMediaDisplayLabels(
            target.content,
            imetaMediaFromTags(target.tags),
          );
          const { content, mediaTags } = buildOutgoingMessage(
            prompt,
            [...originalMedia, ...media],
            findSpoileredImetaMediaUrls(target.content, originalMedia),
          );
          // Only mentions newly added by this edit get p tags — a typo
          // fix re-wakes nobody.
          const alreadyMentioned = new Set(
            target.tags
              .filter((tag) => tag[0] === "p" && tag[1])
              .map((tag) => tag[1]),
          );
          const mentionPubkeys = [
            ...new Set(mentions.map((mention) => mention.pubkey)),
          ].filter((pubkey) => !alreadyMentioned.has(pubkey));
          await editMessageMutation.mutateAsync({
            eventId: target.id,
            content,
            mediaTags,
            mentionPubkeys,
          });
          stopEditing();
        } catch (submitError) {
          // The edited text stays in the box for a retry.
          setError(
            submitError instanceof Error
              ? submitError.message
              : "Failed to edit message.",
          );
        } finally {
          setBusy(false);
        }
      })();
    },
    [
      channel,
      editMessageMutation,
      editingRootId,
      roots,
      setBusy,
      setError,
      stopEditing,
    ],
  );

  return {
    editingRootId,
    peekPreEditInput,
    startEditing,
    stopEditing,
    submitEdit,
  };
}
