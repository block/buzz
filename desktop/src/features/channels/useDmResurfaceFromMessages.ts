import * as React from "react";

import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";
import {
  getChannelMembers,
  type OpenDmInput,
} from "@/shared/api/tauriChannels";
import { CHANNEL_MESSAGE_EVENT_KINDS } from "@/shared/constants/kinds";
import { relayEventChannelId } from "./dmResurface";
import { resurfaceHiddenDmMessage } from "./hiddenDmResurfaceAction";
import { fetchHiddenDmIds } from "./useHiddenDmIds";

type UseDmResurfaceFromMessagesOptions = {
  pubkey: string | undefined;
  relayUrl: string | undefined;
  reopen: (input: OpenDmInput) => Promise<{ id: string }>;
};

export function useDmResurfaceFromMessages({
  pubkey,
  relayUrl,
  reopen,
}: UseDmResurfaceFromMessagesOptions) {
  const handledEventIdsRef = React.useRef(new Set<string>());
  const pendingChannelIdsRef = React.useRef(new Set<string>());
  const generationRef = React.useRef(0);
  const reopenLatest = React.useEffectEvent(reopen);

  React.useEffect(() => {
    const expectedSignerPubkey = pubkey?.trim().toLowerCase() ?? "";
    const expectedRelayUrl = relayUrl?.trim() ?? "";
    const generation = ++generationRef.current;
    handledEventIdsRef.current.clear();
    pendingChannelIdsRef.current.clear();
    if (!expectedSignerPubkey || !expectedRelayUrl) return;

    let disposed = false;
    let unsubscribe: (() => Promise<void>) | undefined;
    const isCurrent = () => !disposed && generationRef.current === generation;
    const handleEvent = async (event: RelayEvent) => {
      if (!isCurrent() || !handledEventIdsRef.current.add(event.id)) return;
      const channelId = relayEventChannelId(event);
      if (!channelId || pendingChannelIdsRef.current.has(channelId)) return;
      pendingChannelIdsRef.current.add(channelId);

      try {
        await resurfaceHiddenDmMessage({
          event,
          expectedRelayUrl,
          expectedSignerPubkey,
          fetchHiddenDmIds: () => fetchHiddenDmIds(expectedSignerPubkey),
          fetchMembers: getChannelMembers,
          isCurrent,
          reopen: reopenLatest,
        });
      } catch (error) {
        handledEventIdsRef.current.delete(event.id);
        if (isCurrent()) {
          console.error("Failed to resurface hidden DM", channelId, error);
        }
      } finally {
        pendingChannelIdsRef.current.delete(channelId);
      }
    };

    void relayClient
      .subscribeLive(
        {
          kinds: [...CHANNEL_MESSAGE_EVENT_KINDS],
          "#p": [expectedSignerPubkey],
          since: Math.floor(Date.now() / 1_000),
          limit: 100,
        },
        (event) => void handleEvent(event),
      )
      .then((dispose) => {
        if (!isCurrent()) {
          void dispose().catch(() => {});
          return;
        }
        unsubscribe = dispose;
      })
      .catch((error) => {
        if (isCurrent()) {
          console.error("Failed to subscribe to hidden DM activity", error);
        }
      });

    return () => {
      disposed = true;
      generationRef.current += 1;
      void unsubscribe?.().catch(() => {});
    };
  }, [pubkey, relayUrl]);
}
