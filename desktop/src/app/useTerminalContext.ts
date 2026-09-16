import * as React from "react";

import type { Channel } from "@/shared/api/types";
import { safeNpub } from "@/shared/lib/nostrUtils";

type TerminalContext = {
  channelId: string | null;
  channelName: string | null;
  threadId: string | null;
  npub: string | null;
  relayUrl: string | null;
};

type TerminalContextResult = {
  activeChannel: Channel | null;
  terminalContext: TerminalContext;
};

export function useTerminalContext({
  channelId,
  channels,
  locationSearch,
  pubkey,
  relayUrl,
}: {
  channelId: string | null;
  channels: Channel[];
  locationSearch: unknown;
  pubkey?: string;
  relayUrl?: string;
}): TerminalContextResult {
  return React.useMemo(() => {
    const search = locationSearch as {
      thread?: unknown;
      threadRootId?: unknown;
    };
    const threadId = search.threadRootId ?? search.thread;
    const activeChannel = channelId
      ? (channels.find((candidate) => candidate.id === channelId) ?? null)
      : null;

    return {
      activeChannel,
      terminalContext: {
        channelId,
        channelName: activeChannel?.name ?? null,
        threadId:
          channelId && typeof threadId === "string" && threadId.length > 0
            ? threadId
            : null,
        npub: pubkey ? safeNpub(pubkey) : null,
        relayUrl: relayUrl ?? null,
      },
    };
  }, [channelId, channels, locationSearch, pubkey, relayUrl]);
}
