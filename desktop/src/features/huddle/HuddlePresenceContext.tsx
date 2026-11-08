import * as React from "react";

import { useRelaySelfQuery } from "@/features/moderation/hooks";
import { useChannelsQuery } from "@/features/channels/hooks";
import { startHuddlePresenceRuntime } from "@/features/huddle/lib/huddlePresenceRuntime";
import { relayClient } from "@/shared/api/relayClient";
import { normalizePubkey } from "@/shared/lib/pubkey";

const EMPTY_HUDDLE_PRESENCE = new Set<string>();
const HuddlePresenceContext = React.createContext<ReadonlySet<string>>(
  EMPTY_HUDDLE_PRESENCE,
);

export function memberChannelIdsKey(
  channels: readonly { id: string; isMember: boolean }[],
): string {
  return channels
    .filter((channel) => channel.isMember)
    .map((channel) => channel.id)
    .sort()
    .join("\u0000");
}

/** Keeps one community-wide lifecycle subscription for name indicators. */
export function HuddlePresenceProvider({
  children,
}: {
  children: React.ReactNode;
}) {
  const relaySelfQuery = useRelaySelfQuery();
  const channelsQuery = useChannelsQuery();
  const relaySelfPubkey = relaySelfQuery.data;
  const [participantPubkeys, setParticipantPubkeys] = React.useState<
    ReadonlySet<string>
  >(EMPTY_HUDDLE_PRESENCE);
  const channelIdsKey = memberChannelIdsKey(channelsQuery.data ?? []);
  const channelIds = React.useMemo(
    () => (channelIdsKey ? channelIdsKey.split("\u0000") : []),
    [channelIdsKey],
  );
  React.useEffect(() => {
    if (
      !relaySelfPubkey ||
      relaySelfQuery.isError ||
      !channelsQuery.isSuccess
    ) {
      setParticipantPubkeys(EMPTY_HUDDLE_PRESENCE);
      return;
    }

    const dispose = startHuddlePresenceRuntime({
      relaySelfPubkey,
      channelIds,
      subscribeLive: (filter, onEvent) =>
        relayClient.subscribeLive(filter, onEvent),
      fetchEvents: (filter) => relayClient.fetchEvents(filter),
      subscribeToReconnects: (listener) =>
        relayClient.subscribeToReconnects(listener),
      onPresence: setParticipantPubkeys,
      onError: (message, error) => console.error(`[huddle] ${message}:`, error),
    });

    return () => {
      dispose();
      setParticipantPubkeys(EMPTY_HUDDLE_PRESENCE);
    };
  }, [
    relaySelfPubkey,
    relaySelfQuery.isError,
    channelsQuery.isSuccess,
    channelIds,
  ]);

  return (
    <HuddlePresenceContext.Provider value={participantPubkeys}>
      {children}
    </HuddlePresenceContext.Provider>
  );
}

export function useIsUserInHuddle(pubkey: string | undefined): boolean {
  const participantPubkeys = React.useContext(HuddlePresenceContext);
  const normalized = normalizePubkey(pubkey ?? "");
  return normalized ? participantPubkeys.has(normalized) : false;
}
