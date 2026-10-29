import type {
  LiveSubscriptionReadiness,
  RelaySubscriptionFilter,
} from "@/shared/api/relayClientShared";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_PRESENCE_UPDATE } from "@/shared/constants/kinds";

export type OpenLiveSubscription = (
  filter: RelaySubscriptionFilter,
  onEvent: (event: RelayEvent) => void,
  onReady: (readiness: LiveSubscriptionReadiness) => void,
  readinessTimeoutMs: number,
) => Promise<() => Promise<void>>;

/** Open an author-scoped presence subscription and require relay EOSE. */
export async function openPresenceSubscription(
  pubkeys: string[],
  onEvent: (event: RelayEvent) => void,
  openLive: OpenLiveSubscription,
) {
  const authors = [...new Set(pubkeys.map((pubkey) => pubkey.toLowerCase()))]
    .filter(Boolean)
    .sort();
  if (authors.length === 0) {
    throw new Error("Presence subscriptions require at least one author.");
  }

  const readiness: { value: LiveSubscriptionReadiness } = { value: "timeout" };
  const unsubscribe = await openLive(
    { kinds: [KIND_PRESENCE_UPDATE], authors, limit: 0 },
    onEvent,
    (nextReadiness) => {
      readiness.value = nextReadiness;
    },
    5_000,
  );
  if (readiness.value === "eose") return unsubscribe;

  await unsubscribe();
  throw new Error(
    readiness.value === "closed"
      ? "Relay rejected the presence subscription."
      : "Timed out confirming the presence subscription.",
  );
}
