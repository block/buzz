import * as React from "react";

import { useKnownAgentPubkeys } from "@/features/agents/useKnownAgentPubkeys";
import type { Channel, RelayEvent } from "@/shared/api/types";
import {
  DEV_MENTION_TICKER_DURATION_MS,
  type DevMentionTickerItem,
  toDevMentionTickerItem,
} from "./mentionTicker";

export function useDevMentionTicker({
  channels,
  currentPubkey,
  onMention,
}: {
  channels: readonly Pick<Channel, "id" | "name">[];
  currentPubkey: string;
  onMention: () => void;
}) {
  const knownAgentPubkeys = useKnownAgentPubkeys();
  const [item, setItem] = React.useState<DevMentionTickerItem | null>(null);

  const handleMention = React.useEffectEvent((event: RelayEvent) => {
    onMention();
    const next = toDevMentionTickerItem(
      event,
      currentPubkey,
      channels,
      knownAgentPubkeys,
    );
    if (next) setItem(next);
  });

  React.useEffect(() => {
    if (!item) return;
    const timeout = window.setTimeout(
      () => setItem(null),
      DEV_MENTION_TICKER_DURATION_MS,
    );
    return () => window.clearTimeout(timeout);
  }, [item]);

  return {
    dismissMentionTicker: () => setItem(null),
    handleMention,
    mentionTicker: item,
  };
}
