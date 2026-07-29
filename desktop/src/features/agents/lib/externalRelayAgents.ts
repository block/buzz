import type {
  ChannelMember,
  PresenceLookup,
  PublicRelayAgentRegistration,
  RelayAgent,
} from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

import { relayAgentIsSharedWithUser } from "./agentAutocompleteEligibility";

const STATUS_RANK: Record<RelayAgent["status"], number> = {
  online: 0,
  away: 1,
  offline: 2,
};

export function buildChannelAgentFallbacks({
  channels,
  membersByChannelId,
  presence,
  registrations = [],
}: {
  channels: readonly { id: string; name: string }[];
  membersByChannelId: Record<
    string,
    | readonly Pick<
        ChannelMember,
        "displayName" | "isAgent" | "pubkey" | "role"
      >[]
    | undefined
  >;
  presence: PresenceLookup | undefined;
  registrations?: readonly PublicRelayAgentRegistration[];
}): RelayAgent[] {
  const agentsByPubkey = new Map<string, RelayAgent>();
  const registrationsByPubkey = new Map(
    registrations.map((registration) => [
      normalizePubkey(registration.pubkey),
      registration,
    ]),
  );

  for (const channel of channels) {
    for (const member of membersByChannelId[channel.id] ?? []) {
      const pubkey = normalizePubkey(member.pubkey);
      const registration = registrationsByPubkey.get(pubkey);
      const isVisibleRegistration =
        registration?.enabled === true &&
        (registration.state === "active" || registration.state === "failed") &&
        registration.channelIds.includes(channel.id);
      if (member.role !== "bot" && !member.isAgent && !isVisibleRegistration) {
        continue;
      }

      const name = isVisibleRegistration
        ? registration.name.trim()
        : member.displayName?.trim();
      if (!name) continue;

      const existing = agentsByPubkey.get(pubkey);
      if (existing) {
        if (!existing.channelIds.includes(channel.id)) {
          existing.channelIds.push(channel.id);
          existing.channels.push(channel.name);
        }
        continue;
      }

      agentsByPubkey.set(pubkey, {
        pubkey,
        name,
        agentType: "external",
        channels: [channel.name],
        channelIds: [channel.id],
        capabilities: [],
        status:
          isVisibleRegistration && registration.state === "failed"
            ? "offline"
            : (presence?.[pubkey] ?? "offline"),
        respondTo: null,
        respondToAllowlist: [],
        ...(isVisibleRegistration
          ? {
              registryState:
                registration.state === "failed"
                  ? ("failed" as const)
                  : ("active" as const),
            }
          : {}),
      });
    }
  }

  return [...agentsByPubkey.values()];
}

export function selectVisibleExternalRelayAgents({
  channelAgents,
  currentPubkey,
  managedAgentPubkeys,
  relayAgents,
  sharedChannelIds,
}: {
  channelAgents?: readonly RelayAgent[];
  currentPubkey?: string | null;
  managedAgentPubkeys: Iterable<string>;
  relayAgents: readonly RelayAgent[] | undefined;
  sharedChannelIds: ReadonlySet<string>;
}): RelayAgent[] {
  const managedPubkeys = new Set(
    [...managedAgentPubkeys].map((pubkey) => normalizePubkey(pubkey)),
  );
  const agentsByPubkey = new Map<string, RelayAgent>();
  const directoryPubkeys = new Set(
    (relayAgents ?? []).map((agent) => normalizePubkey(agent.pubkey)),
  );

  for (const agent of relayAgents ?? []) {
    const pubkey = normalizePubkey(agent.pubkey);
    if (
      managedPubkeys.has(pubkey) ||
      agentsByPubkey.has(pubkey) ||
      !relayAgentIsSharedWithUser(agent, sharedChannelIds, currentPubkey)
    ) {
      continue;
    }
    agentsByPubkey.set(pubkey, agent);
  }

  for (const agent of channelAgents ?? []) {
    const pubkey = normalizePubkey(agent.pubkey);
    if (
      managedPubkeys.has(pubkey) ||
      directoryPubkeys.has(pubkey) ||
      agentsByPubkey.has(pubkey)
    ) {
      continue;
    }
    agentsByPubkey.set(pubkey, agent);
  }

  return [...agentsByPubkey.values()].sort((left, right) => {
    const statusOrder = STATUS_RANK[left.status] - STATUS_RANK[right.status];
    if (statusOrder !== 0) return statusOrder;

    const nameOrder = left.name.localeCompare(right.name);
    if (nameOrder !== 0) return nameOrder;

    return normalizePubkey(left.pubkey).localeCompare(
      normalizePubkey(right.pubkey),
    );
  });
}
