import * as React from "react";
import {
  getAgentTranscript,
  subscribeAgentObserverStore,
} from "@/features/agents/observerRelayStore";
import { partitionComposerWorkingAgents } from "@/features/channels/ui/composerLiveActivity";
import type { TypingIndicatorEntry } from "@/features/messages/useChannelTyping";

/** Partitions thread-scoped bot typers without leaking them into channel state. */
export function useThreadComposerActivity({
  botTypingEntries,
  channelId,
  threadHeadId,
  typingPubkeys,
}: {
  botTypingEntries: readonly TypingIndicatorEntry[];
  channelId: string | null;
  threadHeadId: string | null;
  typingPubkeys: readonly string[];
}): {
  combinedTypingPubkeys: string[];
  hasActivity: boolean;
  pillBotPubkeys: string[];
} {
  const botPubkeys = React.useMemo(() => {
    if (!threadHeadId) return [];
    return botTypingEntries
      .filter((entry) => entry.threadHeadId === threadHeadId)
      .map((entry) => entry.pubkey)
      .filter(
        (pubkey, index, all) =>
          all.findIndex(
            (candidate) => candidate.toLowerCase() === pubkey.toLowerCase(),
          ) === index,
      );
  }, [botTypingEntries, threadHeadId]);
  const subscribe = React.useCallback(
    (onChange: () => void) => subscribeAgentObserverStore(onChange),
    [],
  );
  const getSnapshot = React.useCallback(() => {
    const partition = partitionComposerWorkingAgents({
      channelId,
      getTranscript: getAgentTranscript,
      // Thread typing stays out of the channel-wide working registry. A
      // channel-scoped transcript alone decides whether a real preview exists.
      getWorkingSource: () => "typing",
      pubkeys: botPubkeys,
    });
    return `${partition.pillPubkeys.join(",")}\n${partition.typingGroupPubkeys.join(",")}`;
  }, [botPubkeys, channelId]);
  const partitionKey = React.useSyncExternalStore(subscribe, getSnapshot);
  const [pillKey = "", typingKey = ""] = partitionKey.split("\n");
  const pillBotPubkeys = React.useMemo(
    () => (pillKey === "" ? [] : pillKey.split(",")),
    [pillKey],
  );
  const typingBotPubkeys = React.useMemo(
    () => (typingKey === "" ? [] : typingKey.split(",")),
    [typingKey],
  );
  const combinedTypingPubkeys = React.useMemo(
    () => [...typingPubkeys, ...typingBotPubkeys],
    [typingBotPubkeys, typingPubkeys],
  );
  return {
    combinedTypingPubkeys,
    hasActivity: pillBotPubkeys.length > 0 || combinedTypingPubkeys.length > 0,
    pillBotPubkeys,
  };
}
