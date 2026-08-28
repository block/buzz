import * as React from "react";

import { useStartManagedAgentMutation } from "@/features/agents/hooks";
import { normalizeRelayUrl } from "@/features/communities/communityStorage";
import { useCommunities } from "@/features/communities/useCommunities";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { ManagedAgent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import type { ManagedAgentStartInput } from "./useMentionWakePreflight";

/**
 * Scope of an in-flight start: the agent and the tenant it is being started
 * for.
 *
 * `speculative` is deliberately excluded. If a prewake is still in flight when
 * the send path asks for a durable start of the same scope, sharing that flight
 * is correct — the send's own message dispatch is what promotes the harness out
 * of its never-used bound, not the flavour of the call that started it. Keying
 * on the flag instead would spawn a second start for an agent already booting.
 */
export function managedAgentStartFlightKey(input: ManagedAgentStartInput) {
  const pubkey = normalizePubkey(
    typeof input === "string" ? input : input.pubkey,
  );
  return typeof input === "string"
    ? JSON.stringify([pubkey, "", ""])
    : JSON.stringify([
        pubkey,
        input.expectedRelayUrl?.trim() ?? "",
        normalizePubkey(input.expectedSignerPubkey ?? ""),
      ]);
}

export function useSingleFlightManagedAgentStart(
  startManagedAgent: (input: ManagedAgentStartInput) => Promise<ManagedAgent>,
) {
  const promisesRef = React.useRef(new Map<string, Promise<ManagedAgent>>());
  return React.useCallback(
    (input: ManagedAgentStartInput) => {
      const key = managedAgentStartFlightKey(input);
      const existing = promisesRef.current.get(key);
      if (existing) return existing;

      const started = startManagedAgent(input);
      promisesRef.current.set(key, started);
      const clear = () => {
        if (promisesRef.current.get(key) === started) {
          promisesRef.current.delete(key);
        }
      };
      void started.then(clear, clear);
      return started;
    },
    [startManagedAgent],
  );
}

export function useScopedManagedAgentStart() {
  const startAgentMutation = useStartManagedAgentMutation();
  const { activeCommunity } = useCommunities();
  const identityQuery = useIdentityQuery();
  const relayUrl = activeCommunity?.relayUrl;
  const expectedRelayUrl = relayUrl ? normalizeRelayUrl(relayUrl) : undefined;
  const expectedSignerPubkey = identityQuery.data?.pubkey
    ? normalizePubkey(identityQuery.data.pubkey)
    : undefined;
  const startManagedAgentOnce = useSingleFlightManagedAgentStart(
    startAgentMutation.mutateAsync,
  );
  const startManagedAgentForScope = React.useCallback(
    (pubkey: string) =>
      startManagedAgentOnce({ pubkey, expectedRelayUrl, expectedSignerPubkey }),
    [expectedRelayUrl, expectedSignerPubkey, startManagedAgentOnce],
  );
  return {
    expectedRelayUrl,
    expectedSignerPubkey,
    startManagedAgentForScope,
    startManagedAgentOnce,
  };
}
