import * as React from "react";

type RestoreAddressedAgentMentions = (
  pubkeys?: readonly string[],
  allowedUnpinnedPubkeys?: readonly string[],
) => string;

export function useAddressedAgentMentionRestore({
  audiencePubkeys,
  audienceScope,
  channelId,
  enabled,
  getComposerRevision,
}: {
  audiencePubkeys: readonly string[];
  audienceScope: string | null;
  getComposerRevision: () => number;
  channelId: string | null;
  enabled: boolean;
}) {
  const restoreAddressedAgentMentionsRef =
    React.useRef<RestoreAddressedAgentMentions>(() => "");
  const restoreFrameRef = React.useRef<number | null>(null);
  const ownerRef = React.useRef({
    audiencePubkeys,
    audienceScope,
    channelId,
    enabled,
    getComposerRevision,
    generation: 0,
  });
  const previous = ownerRef.current;
  // Audience removal and scope/preference changes retire even an otherwise
  // identical later state. An old send may not undo a user's unpin or turn-off.
  const retired =
    previous.channelId !== channelId ||
    previous.audienceScope !== audienceScope ||
    previous.enabled !== enabled ||
    previous.audiencePubkeys.some(
      (pubkey) => !audiencePubkeys.includes(pubkey),
    );
  ownerRef.current = {
    audiencePubkeys,
    audienceScope,
    channelId,
    enabled,
    getComposerRevision,
    generation: previous.generation + Number(retired),
  };

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
      const owner = ownerRef.current;
      const currentAudience = new Set(owner.audiencePubkeys);
      const confirmedPinnedPubkeys = newlyPinnedPubkeys.filter((pubkey) =>
        currentAudience.has(pubkey),
      );
      if (!owner.enabled || confirmedPinnedPubkeys.length === 0) return;

      const revision = owner.getComposerRevision();
      if (restoreFrameRef.current !== null) {
        cancelAnimationFrame(restoreFrameRef.current);
      }
      restoreFrameRef.current = requestAnimationFrame(() => {
        restoreFrameRef.current = null;
        // The send flow checks before scheduling, but a native edit can occur
        // before this frame (including edit -> clear back to the same text).
        if (
          ownerRef.current.generation !== owner.generation ||
          ownerRef.current.getComposerRevision() !== revision
        )
          return;
        restoreAddressedAgentMentionsRef.current(
          pubkeys,
          confirmedPinnedPubkeys,
        );
      });
    },
    [],
  );

  return {
    onAddressedAgentsComposerCleared,
    onAddressedAgentsSendSucceeded,
    restoreAddressedAgentMentionsRef,
  };
}
