import { useCallback, useState } from "react";
import { resolveMessagesDestination } from "./resolveMessagesDestination";
import type {
  MessagesDestination,
  NavigableChannel,
  NavigationDestination,
} from "./types";

export function useNavigation() {
  const [destination, setDestination] =
    useState<NavigationDestination>("channels");
  const [messagesDestination, setMessagesDestination] =
    useState<MessagesDestination>({ type: "empty" });

  const resolveMessages = useCallback(
    (
      channels: readonly NavigableChannel[],
      sessionChannelIds: ReadonlySet<string>,
    ) => {
      setMessagesDestination((current) =>
        resolveMessagesDestination(current, channels, sessionChannelIds),
      );
    },
    [],
  );

  const openChannel = useCallback((channelId: string) => {
    setDestination("channels");
    setMessagesDestination({ type: "channel", channelId });
  }, []);

  const openSession = useCallback((channelId: string) => {
    setDestination("channels");
    setMessagesDestination({ type: "session", channelId });
  }, []);

  const startSession = useCallback((originChannelId: string) => {
    setDestination("channels");
    setMessagesDestination({ type: "new", originChannelId });
  }, []);

  const openChannels = useCallback(() => setDestination("channels"), []);
  const openAgents = useCallback(() => setDestination("agents"), []);

  return {
    destination,
    messagesDestination,
    resolveMessages,
    openChannel,
    openSession,
    startSession,
    openChannels,
    openAgents,
  };
}
