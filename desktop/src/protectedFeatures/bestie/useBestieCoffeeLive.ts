import * as React from "react";

import { useAgentWorking } from "@/features/agents/agentWorkingSignal";
import { useChannelMessagesQuery } from "@/features/messages/hooks";
import { normalizePubkey } from "@/shared/lib/pubkey";
import type { Channel } from "@/shared/api/types";

import {
  isBestieCoffeeLive,
  isBestieCoffeePendingStale,
  messageLooksLikeBestieCoffeeTrigger,
  messageLooksLikeBestieCompetingSystemTrigger,
  shouldDisableBestieCoffeeBrew,
} from "./bestieCoffeeLive";
import {
  abandonBestieCoffeePendingForScope,
  useBestieCoffee,
} from "./bestieCoffeeStore";
import type { BestieCoffeeScope } from "./bestieCoffeeTypes";

/**
 * Brewing UI for Coffee RHS: ties disable + 🤔… to the real in-flight coffee
 * turn (ACP agent working on Assistant DM + pending/grace), not a fake spinner.
 */
export function useBestieCoffeeLive(
  scope: BestieCoffeeScope | null,
  bestieChannel: Channel | null | undefined,
): {
  brewDisabled: boolean;
  coffeeLive: boolean;
} {
  const coffeeState = useBestieCoffee(scope);
  const agentPubkey = scope?.agentPubkey ?? null;
  const channelId = bestieChannel?.id ?? null;
  const working = useAgentWorking(agentPubkey, channelId);
  const messagesQuery = useChannelMessagesQuery(bestieChannel ?? null);

  const pendingRun = coffeeState.pendingRun;
  const tickWhilePending = pendingRun != null;
  const [nowSeconds, setNowSeconds] = React.useState(() =>
    Math.floor(Date.now() / 1000),
  );
  React.useEffect(() => {
    if (!tickWhilePending) return;
    setNowSeconds(Math.floor(Date.now() / 1000));
    const timer = window.setInterval(() => {
      setNowSeconds(Math.floor(Date.now() / 1000));
    }, 1000);
    return () => window.clearInterval(timer);
  }, [tickWhilePending, pendingRun?.startedAt]);

  const { latestCompetingTriggerAt, openCoffeeTriggerAt } = React.useMemo(() => {
    if (!scope || !agentPubkey) {
      return { latestCompetingTriggerAt: null, openCoffeeTriggerAt: null };
    }
    const owner = normalizePubkey(scope.ownerPubkey);
    const events = messagesQuery.data ?? [];
    const matchedTriggerIds = new Set(
      coffeeState.entries
        .map((entry) => entry.triggerMessageId)
        .filter((id): id is string => typeof id === "string" && id.length > 0),
    );
    const lastEntryRanAt = coffeeState.entries[0]?.ranAt ?? 0;
    let latestCoffee: number | null = null;
    let latestCompeting: number | null = null;
    for (const event of events) {
      if (typeof event.content !== "string") continue;
      if (normalizePubkey(event.pubkey) !== owner) continue;
      const createdAt =
        typeof event.created_at === "number"
          ? event.created_at
          : Math.floor(Date.now() / 1000);
      if (messageLooksLikeBestieCompetingSystemTrigger(event.content)) {
        if (latestCompeting == null || createdAt > latestCompeting) {
          latestCompeting = createdAt;
        }
        continue;
      }
      if (!messageLooksLikeBestieCoffeeTrigger(event.content)) continue;
      // Prefer id match; fall back to time vs last entry for legacy rows.
      if (matchedTriggerIds.has(event.id)) continue;
      if (createdAt + 5 < lastEntryRanAt) continue;
      if (latestCoffee == null || createdAt > latestCoffee) {
        latestCoffee = createdAt;
      }
    }
    return {
      latestCompetingTriggerAt: latestCompeting,
      openCoffeeTriggerAt: latestCoffee,
    };
  }, [agentPubkey, coffeeState.entries, messagesQuery.data, scope]);

  const agentWorkingOnBestieDm = working.working;

  // Drop / finalize abandoned pending so Brew + 👀 cannot stick forever.
  React.useEffect(() => {
    if (!scope) return;
    if (
      !isBestieCoffeePendingStale({
        agentWorkingOnBestieDm,
        nowSeconds,
        pendingRun,
        latestCompetingTriggerAt,
      })
    ) {
      return;
    }
    abandonBestieCoffeePendingForScope(scope);
  }, [
    agentWorkingOnBestieDm,
    latestCompetingTriggerAt,
    nowSeconds,
    pendingRun,
    scope,
  ]);

  const coffeeLive = isBestieCoffeeLive({
    agentWorkingOnBestieDm,
    nowSeconds,
    openCoffeeTriggerAt,
    latestCompetingTriggerAt,
    pendingRun,
  });
  const brewDisabled = shouldDisableBestieCoffeeBrew({
    agentWorkingOnBestieDm,
    nowSeconds,
    openCoffeeTriggerAt,
    latestCompetingTriggerAt,
    pendingRun,
  });

  return { brewDisabled, coffeeLive };
}
