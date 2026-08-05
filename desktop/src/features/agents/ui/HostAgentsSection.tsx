import * as React from "react";

import { useRelayAgentsQuery } from "@/features/agents/hooks";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import type { RelayAgent } from "@/shared/api/types";
import type { ProfilePanelOpenOptions } from "@/shared/context/ProfilePanelContext";
import { Badge } from "@/shared/ui/badge";
import { IdentityCardSkeleton } from "@/shared/ui/identity-card-skeleton";
import { AgentIdentityCard } from "./AgentIdentityCard";
import {
  AGENT_CARD_GRID_COLUMNS_CLASS,
  IDENTITY_CARD_GRID_CLASS,
} from "./UnifiedAgentsSection";

type HostAgentsSectionProps = {
  /** Local managed agent pubkeys — excluded so we never double-list. */
  managedPubkeys: ReadonlySet<string>;
  onOpenAgentProfile: (
    pubkey: string,
    options?: ProfilePanelOpenOptions,
  ) => void;
};

/**
 * Read-only section for host-minted / directory agents that carry kind:10100
 * `host` lineage and are not in the Local managed store.
 *
 * Reuses list_relay_agents + users-batch profile hydration (same as DMs).
 * No Local Start — process lives on the host.
 */
export function HostAgentsSection({
  managedPubkeys,
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

  const hostPubkeys = React.useMemo(
    () => [...new Set(hostAgents.map((agent) => agent.hostPubkey))],
    [hostAgents],
  );
  const hostProfilesQuery = useUsersBatchQuery(hostPubkeys, {
    enabled: hostPubkeys.length > 0,
  });
  const hostProfiles = hostProfilesQuery.data?.profiles;

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
            const hostLabel = resolveUserLabel({
              pubkey: agent.hostPubkey,
              profiles: hostProfiles,
            });
            return (
              <AgentIdentityCard
                key={agent.pubkey}
                ariaLabel={`${agent.name}, on ${hostLabel}`}
                dataTestId={`host-agent-${agent.pubkey}`}
                label={agent.name}
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
