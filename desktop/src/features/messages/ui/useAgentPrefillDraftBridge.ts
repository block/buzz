import * as React from "react";

import type { DraftEntryKind } from "@/features/messages/lib/useDrafts";
import { useAddressedAgentMentionRestore } from "./useAddressedAgentMentionRestore";

type RestoreAddressedAgentMentions = (
  pubkeys?: readonly string[],
  allowedUnpinnedPubkeys?: readonly string[],
) => string;

export function useAgentPrefillDraftBridge({
  audiencePubkeys,
  channelId,
  contentRef,
  getEntryKind,
  keepMentionedAgentsPinned,
  pendingImetaRef,
  persistAgentPrefill,
  queuedAttachmentsRef,
  resetToAgentPrefill,
  setComposerContent,
}: {
  audiencePubkeys: readonly string[];
  channelId: string | null;
  contentRef: React.MutableRefObject<string>;
  getEntryKind: () => DraftEntryKind;
  keepMentionedAgentsPinned: boolean;
  pendingImetaRef: React.MutableRefObject<readonly unknown[]>;
  persistAgentPrefill: (content: string) => void;
  queuedAttachmentsRef: React.MutableRefObject<readonly unknown[]>;
  resetToAgentPrefill: () => void;
  setComposerContent: (content: string) => void;
}) {
  const restoreAddressedAgentMentionsRef =
    React.useRef<RestoreAddressedAgentMentions>(() => "");

  /**
   * Automatic restoration inserts its mentions with `preventUpdate`, so the
   * editor's authored-update observer never sees them. Mirror the restored
   * text into composer state here, and keep a composer that was only ever
   * filled automatically classified as a prefill rather than a real draft.
   */
  const applyRestoredAgentMentions = React.useCallback(
    (
      restoreAddressedAgentMentions: RestoreAddressedAgentMentions,
      pubkeys?: readonly string[],
      allowedUnpinnedPubkeys?: readonly string[],
    ) => {
      const shouldPersistPrefill = getEntryKind() === "agent-prefill";
      const content = restoreAddressedAgentMentions(
        pubkeys,
        allowedUnpinnedPubkeys,
      );
      contentRef.current = content;
      setComposerContent(content);
      if (shouldPersistPrefill) persistAgentPrefill(content);
      return content;
    },
    [contentRef, getEntryKind, persistAgentPrefill, setComposerContent],
  );

  // The deferred post-send restore — its frame fence, channel check, and
  // audience confirmation — is owned by `useAddressedAgentMentionRestore`.
  // This bridge only layers the draft classification on top of it.
  const addressedMentionRestore = useAddressedAgentMentionRestore({
    audiencePubkeys,
    channelId,
    enabled: keepMentionedAgentsPinned,
  });
  addressedMentionRestore.restoreAddressedAgentMentionsRef.current = (
    pubkeys,
    allowedUnpinnedPubkeys,
  ) =>
    applyRestoredAgentMentions(
      restoreAddressedAgentMentionsRef.current,
      pubkeys,
      allowedUnpinnedPubkeys,
    );

  const onAddressedAgentsComposerCleared = React.useCallback(
    (pubkeys: readonly string[]) => {
      const content = restoreAddressedAgentMentionsRef.current(pubkeys);
      setComposerContent(content);
      resetToAgentPrefill();
      return content;
    },
    [resetToAgentPrefill, setComposerContent],
  );

  const onAddressedAgentsSendSucceeded = React.useCallback(
    (pubkeys: readonly string[], newlyPinnedPubkeys: readonly string[]) => {
      // The send emptied the composer; capture the prefill it will be
      // restored to now, before the deferred restore can race authored text.
      if (
        getEntryKind() === "agent-prefill" &&
        contentRef.current.trim().length > 0
      ) {
        persistAgentPrefill(contentRef.current);
      }
      addressedMentionRestore.onAddressedAgentsSendSucceeded(
        pubkeys,
        newlyPinnedPubkeys,
      );
    },
    [
      addressedMentionRestore.onAddressedAgentsSendSucceeded,
      contentRef,
      getEntryKind,
      persistAgentPrefill,
    ],
  );

  const onAgentPrefillChanged = React.useCallback(
    (content: string) => {
      contentRef.current = content;
      setComposerContent(content);
      persistAgentPrefill(content);
    },
    [contentRef, persistAgentPrefill, setComposerContent],
  );

  const shouldKeepAgentPrefill = React.useCallback(
    (currentContent: string) =>
      getEntryKind() === "agent-prefill" ||
      (currentContent.trim().length === 0 &&
        pendingImetaRef.current.length === 0 &&
        queuedAttachmentsRef.current.length === 0),
    [getEntryKind, pendingImetaRef, queuedAttachmentsRef],
  );

  return {
    applyRestoredAgentMentions,
    onAddressedAgentsComposerCleared,
    onAddressedAgentsSendSucceeded,
    onAgentPrefillChanged,
    restoreAddressedAgentMentionsRef,
    shouldKeepAgentPrefill,
  };
}
