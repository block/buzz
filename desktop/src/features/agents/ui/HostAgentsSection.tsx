import * as React from "react";

import { useRelayAgentsQuery } from "@/features/agents/hooks";
import { resolveHostLabel } from "@/features/agents/lib/resolveHostLabel";
import { usePresenceQuery } from "@/features/presence/hooks";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import {
  DEFAULT_HOVER_PROFILE_STATUS_GEOMETRY,
  ProfileAvatarWithStatus,
  scaleProfileAvatarStatusGeometry,
} from "@/features/profile/ui/ProfileAvatarWithStatus";
import { normalizePubkey } from "@/shared/lib/pubkey";
import type { RelayAgent } from "@/shared/api/types";
import type { ProfilePanelOpenOptions } from "@/shared/context/ProfilePanelContext";
import { Badge } from "@/shared/ui/badge";
import { IdentityCardSkeleton } from "@/shared/ui/identity-card-skeleton";
import { AgentIdentityCard } from "./AgentIdentityCard";
import {
  AGENT_CARD_GRID_COLUMNS_CLASS,
  IDENTITY_CARD_GRID_CLASS,
} from "./UnifiedAgentsSection";

/** Card avatar size matches AgentIdentityCard / Local agent cards (h-24 w-24). */
const HOST_AGENT_AVATAR_SIZE = 96;

const HOST_AGENT_STATUS_GEOMETRY = scaleProfileAvatarStatusGeometry(
  DEFAULT_HOVER_PROFILE_STATUS_GEOMETRY,
  HOST_AGENT_AVATAR_SIZE,
);

type HostAgentsSectionProps = {
  /** Local managed agent pubkeys — excluded so we never double-list. */
  managedPubkeys: ReadonlySet<string>;
  /**
   * Optional hostPubkey → human environment label (e.g. agentbox).
   * Environment names are not kind:0 person profiles.
   */
  knownHosts?: Readonly<Record<string, string>>;
  onOpenAgentProfile: (
    pubkey: string,
    options?: ProfilePanelOpenOptions,
  ) => void;
};

/**
 * Read-only section for host-minted / directory agents that carry kind:10100
 * `host` lineage and are not in the Local managed store.
 *
 * Hydration reuses native Buzz lanes (same as DMs / profile click-through):
 * - directory list: list_relay_agents (10100)
 * - face + person name: users-batch (kind:0)
 * - live online: presence (kind:20001)
 * - environment badge: knownHosts / truncated host pubkey — not kind:0
 *
 * No Local Start — process lives on the host.
 */
export function HostAgentsSection({
  managedPubkeys,
  knownHosts,
  onOpenAgentProfile,
}: HostAgentsSectionProps) {
  const relayAgentsQuery = useRelayAgentsQuery();
  const hostAgents = React.useMemo(
    () =>
      (relayAgentsQuery.data ?? []).filter(
        (agent): agent is RelayAgent & { hostPubkey: string } =>
          typeof agent.hostPubkey === "string" &&
          agent.hostPubkey.length === 64 &&
          !managedPubkeys.has(agent.pubkey.toLowerCase()),
      ),
    [managedPubkeys, relayAgentsQuery.data],
  );

  const agentPubkeys = React.useMemo(
    () => hostAgents.map((agent) => agent.pubkey),
    [hostAgents],
  );

  const agentProfilesQuery = useUsersBatchQuery(agentPubkeys, {
    enabled: agentPubkeys.length > 0,
  });
  const agentProfiles = agentProfilesQuery.data?.profiles;

  const presenceQuery = usePresenceQuery(agentPubkeys, {
    enabled: agentPubkeys.length > 0,
  });
  const presenceLookup = presenceQuery.data;

  // Nothing to show and not loading → omit section (no empty chrome).
  if (
    !relayAgentsQuery.isLoading &&
    !relayAgentsQuery.isError &&
    hostAgents.length === 0
  ) {
    return null;
  }

  return (
    <section
      aria-label="Host agents"
      className="space-y-3"
      data-testid="host-agents-section"
    >
      <div className="flex items-baseline justify-between gap-3">
        <div className="min-w-0 space-y-0.5">
          <h2 className="text-sm font-semibold tracking-tight text-foreground">
            On host
          </h2>
          <p className="text-xs text-secondary-foreground/80">
            Agents discovered from the relay with a host environment. Open a
            profile to message them — they do not Start on this device.
          </p>
        </div>
      </div>

      {relayAgentsQuery.isError ? (
        <p className="text-sm text-destructive" role="alert">
          {relayAgentsQuery.error instanceof Error
            ? relayAgentsQuery.error.message
            : "Could not load relay agents."}
        </p>
      ) : null}

      {relayAgentsQuery.isLoading ? (
        <div className={IDENTITY_CARD_GRID_CLASS}>
          <IdentityCardSkeleton className="w-full" />
          <IdentityCardSkeleton className="w-full" />
        </div>
      ) : (
        <div
          className={`${AGENT_CARD_GRID_COLUMNS_CLASS} grid justify-start gap-3 [@container(max-width:40rem)]:justify-center`}
        >
          {hostAgents.map((agent) => {
            const pubkeyLower = normalizePubkey(agent.pubkey);
            const profile = agentProfiles?.[pubkeyLower];
            const title =
              profile?.displayName?.trim() || agent.name.trim() || pubkeyLower;
            const avatarUrl = profile?.avatarUrl ?? null;
            const presenceStatus = presenceLookup?.[pubkeyLower];
            const hostLabel = resolveHostLabel({
              hostPubkey: agent.hostPubkey,
              knownHosts,
            });

            return (
              <AgentIdentityCard
                key={agent.pubkey}
                ariaLabel={`${title}, on ${hostLabel}`}
                avatar={
                  <ProfileAvatarWithStatus
                    avatarClassName="border-[3px] border-background bg-muted shadow-none"
                    avatarUrl={avatarUrl}
                    geometry={HOST_AGENT_STATUS_GEOMETRY}
                    iconClassName="h-8 w-8"
                    label={title}
                    size={HOST_AGENT_AVATAR_SIZE}
                    status={presenceStatus}
                    statusTestId={`host-agent-presence-${agent.pubkey}`}
                    testId={`host-agent-avatar-${agent.pubkey}`}
                  />
                }
                avatarUrl={avatarUrl}
                dataTestId={`host-agent-${agent.pubkey}`}
                label={title}
                modelLabel={null}
                onClick={() => {
                  onOpenAgentProfile(agent.pubkey);
                }}
                statusBadge={
                  <Badge
                    className="pointer-events-auto max-w-full truncate font-normal"
                    data-testid={`host-agent-badge-${agent.pubkey}`}
                    title={`Host ${agent.hostPubkey}`}
                    variant="secondary"
                  >
                    On {hostLabel}
                  </Badge>
                }
              />
            );
          })}
        </div>
      )}
    </section>
  );
}
