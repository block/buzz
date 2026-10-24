import * as React from "react";

type RestoreAddressedAgentMentions = (
  pubkeys?: readonly string[],
  allowedUnpinnedPubkeys?: readonly string[],
) => string;

export function useAddressedAgentMentionRestore({
  audiencePubkeys,
  channelId,
  enabled,
}: {
  audiencePubkeys: readonly string[];
  channelId: string | null;
  enabled: boolean;
}) {
  const restoreAddressedAgentMentionsRef =
    React.useRef<RestoreAddressedAgentMentions>(() => "");
  const restoreFrameRef = React.useRef<number | null>(null);
  const channelIdRef = React.useRef(channelId);
  channelIdRef.current = channelId;

  React.useEffect(
    () => () => {
      if (restoreFrameRef.current !== null) {
        cancelAnimationFrame(restoreFrameRef.current);
      }
    },
    [],
  );

  const onAddressedAgentsComposerCleared = React.useCallback(
    (pubkeys: readonly string[]) =>
      restoreAddressedAgentMentionsRef.current(pubkeys),
    [],
  );
  const onAddressedAgentsSendSucceeded = React.useCallback(
    (pubkeys: readonly string[], newlyPinnedPubkeys: readonly string[]) => {
      const currentAudience = new Set(audiencePubkeys);
      const confirmedPinnedPubkeys = newlyPinnedPubkeys.filter((pubkey) =>
        currentAudience.has(pubkey),
      );
      if (!enabled || confirmedPinnedPubkeys.length === 0) return;

      const sentChannelId = channelId;
      if (restoreFrameRef.current !== null) {
        cancelAnimationFrame(restoreFrameRef.current);
      }
      restoreFrameRef.current = requestAnimationFrame(() => {
        restoreFrameRef.current = null;
        if (channelIdRef.current !== sentChannelId) return;
        restoreAddressedAgentMentionsRef.current(
          pubkeys,
          confirmedPinnedPubkeys,
        );
      });
    },
    [audiencePubkeys, channelId, enabled],
  );

  return {
    onAddressedAgentsComposerCleared,
    onAddressedAgentsSendSucceeded,
    restoreAddressedAgentMentionsRef,
  };
}
