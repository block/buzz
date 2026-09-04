import * as React from "react";

type RestoreAddressedAgentMentions = (
  pubkeys?: readonly string[],
  allowedUnpinnedPubkeys?: readonly string[],
) => string;

export function useAddressedAgentMentionRestore({
  audiencePubkeys,
  channelId,
  enabled,
  getComposerRevision,
  runComposerUpdate,
}: {
  audiencePubkeys: readonly string[];
  channelId: string | null;
  enabled: boolean;
  getComposerRevision: () => number;
  runComposerUpdate: (update: () => void) => void;
}) {
  const restoreAddressedAgentMentionsRef =
    React.useRef<RestoreAddressedAgentMentions>(() => "");
  const restoreFrameRef = React.useRef<number | null>(null);

  // biome-ignore lint/correctness/useExhaustiveDependencies: revoke pending writes on owner/setting transitions
  React.useLayoutEffect(
    () => () => {
      if (restoreFrameRef.current !== null) {
        cancelAnimationFrame(restoreFrameRef.current);
        restoreFrameRef.current = null;
      }
    },
    // Accessor identity owns a draft visit, not just a channel (including A→B→A).
    [channelId, enabled, getComposerRevision],
  );

  const onAddressedAgentsComposerCleared = React.useCallback(
    (
      pubkeys?: readonly string[],
      allowedUnpinnedPubkeys?: readonly string[],
    ) => {
      let content = "";
      // All automatic restorations share the draft owner's programmatic boundary.
      // They must not manufacture authored intent, especially authored emptiness.
      runComposerUpdate(() => {
        content = restoreAddressedAgentMentionsRef.current(
          pubkeys,
          allowedUnpinnedPubkeys,
        );
      });
      return content;
    },
    [runComposerUpdate],
  );
  const onAddressedAgentsSendSucceeded = React.useCallback(
    (pubkeys: readonly string[], newlyPinnedPubkeys: readonly string[]) => {
      const currentAudience = new Set(audiencePubkeys);
      const confirmedPinnedPubkeys = newlyPinnedPubkeys.filter((pubkey) =>
        currentAudience.has(pubkey),
      );
      if (!enabled || confirmedPinnedPubkeys.length === 0) return;

      const revision = getComposerRevision();
      if (restoreFrameRef.current !== null) {
        cancelAnimationFrame(restoreFrameRef.current);
      }
      const frame = requestAnimationFrame(() => {
        if (restoreFrameRef.current !== frame) return;
        restoreFrameRef.current = null;
        // Recheck shared authority at execution, not only at send settlement.
        // Authoring (even empty), deletion, reset and newer sends revoke it.
        if (getComposerRevision() !== revision) return;
        onAddressedAgentsComposerCleared(pubkeys, confirmedPinnedPubkeys);
      });
      restoreFrameRef.current = frame;
    },
    [
      audiencePubkeys,
      enabled,
      getComposerRevision,
      onAddressedAgentsComposerCleared,
    ],
  );

  return {
    onAddressedAgentsComposerCleared,
    onAddressedAgentsSendSucceeded,
    restoreAddressedAgentMentionsRef,
  };
}
