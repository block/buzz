import * as React from "react";

import { relayAgentAllowlistsUser } from "@/features/agents/lib/agentAutocompleteEligibility";
import {
  useManagedAgentsQuery,
  useRelayAgentsQuery,
} from "@/features/agents/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { ManagedAgent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

export type DevAgentTarget = {
  pubkey: string;
  name: string;
  source: "managed" | "relay";
  /** Present for managed agents; needed to attach + start them. */
  managedAgent?: ManagedAgent;
};

/**
 * The composer target the user cycles through with Tab: plain human chat, or
 * one of the agents that can be tagged into a new session channel.
 */
export type DevComposerMode =
  | { kind: "chat" }
  | { kind: "agent"; target: DevAgentTarget };

export function devComposerModeLabel(mode: DevComposerMode): string {
  return mode.kind === "chat" ? "chat" : mode.target.name;
}

/**
 * Global (channel-independent) list of composer modes. Managed agents are
 * always available — attaching one to a fresh channel is part of the send
 * flow. Relay agents qualify only when their announcement explicitly
 * allowlists the current identity: the shared relay is littered with stale
 * kind:10100 announcements from abandoned identities (welcome-team spawns,
 * wiped data dirs, other users' desktops), and the broader "responds to
 * anyone + shares a channel" rule used by mention autocomplete would surface
 * every one of them here.
 */
export function useDevComposerModes(): DevComposerMode[] {
  const identityQuery = useIdentityQuery();
  const managedAgentsQuery = useManagedAgentsQuery();
  const relayAgentsQuery = useRelayAgentsQuery();

  const currentPubkey = identityQuery.data?.pubkey ?? null;
  const managedAgents = managedAgentsQuery.data;
  const relayAgents = relayAgentsQuery.data;

  return React.useMemo(() => {
    const targets = new Map<string, DevAgentTarget>();

    for (const agent of managedAgents ?? []) {
      const pubkey = normalizePubkey(agent.pubkey);
      targets.set(pubkey, {
        pubkey: agent.pubkey,
        name: agent.name,
        source: "managed",
        managedAgent: agent,
      });
    }

    for (const agent of relayAgents ?? []) {
      const pubkey = normalizePubkey(agent.pubkey);
      if (targets.has(pubkey)) continue;
      if (!relayAgentAllowlistsUser(agent, currentPubkey)) continue;
      targets.set(pubkey, {
        pubkey: agent.pubkey,
        name: agent.name,
        source: "relay",
      });
    }

    const agentModes = [...targets.values()]
      .sort((left, right) => left.name.localeCompare(right.name))
      .map(
        (target): DevComposerMode => ({
          kind: "agent",
          target,
        }),
      );

    return [{ kind: "chat" } satisfies DevComposerMode, ...agentModes];
  }, [currentPubkey, managedAgents, relayAgents]);
}
