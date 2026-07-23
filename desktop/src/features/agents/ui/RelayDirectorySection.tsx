import * as React from "react";
import { ChevronDown, ChevronRight, Pencil, Search } from "lucide-react";

import { externalAgentPresentationScope } from "@/features/agents/externalAgentPresentation";
import { useCommunities } from "@/features/communities/useCommunities";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { RelayAgent } from "@/shared/api/types";
import { PresenceBadge } from "@/features/presence/ui/PresenceBadge";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { AgentIdentityCard } from "./AgentIdentityCard";
import { ExternalAgentPresentationDialog } from "./ExternalAgentPresentationDialog";

export function RelayDirectorySection({
  error,
  isLoading,
  managedPubkeys,
  onOpenAgentProfile,
  relayAgents,
}: {
  error: Error | null;
  isLoading: boolean;
  managedPubkeys: Set<string>;
  onOpenAgentProfile: (pubkey: string) => void;
  relayAgents: RelayAgent[];
}) {
  const identityQuery = useIdentityQuery();
  const { activeCommunity } = useCommunities();
  const [isExpanded, setIsExpanded] = React.useState(true);
  const [searchQuery, setSearchQuery] = React.useState("");
  const [agentToCustomize, setAgentToCustomize] =
    React.useState<RelayAgent | null>(null);
  const currentPubkey = normalizePubkey(identityQuery.data?.pubkey ?? "");
  const presentationScope = externalAgentPresentationScope({
    identityPubkey: identityQuery.data?.pubkey,
    relayUrl: activeCommunity?.relayUrl,
  });

  // Only show agents that are NOT managed locally — those are already in the
  // managed agents section above.
  const otherAgents = React.useMemo(
    () => relayAgents.filter((agent) => !managedPubkeys.has(agent.pubkey)),
    [relayAgents, managedPubkeys],
  );

  const filteredAgents = React.useMemo(() => {
    if (!searchQuery.trim()) return otherAgents;
    const query = searchQuery.toLowerCase();
    return otherAgents.filter(
      (agent) =>
        agent.name.toLowerCase().includes(query) ||
        agent.agentType.toLowerCase().includes(query) ||
        agent.channels.some((ch) => ch.toLowerCase().includes(query)),
    );
  }, [otherAgents, searchQuery]);

  const sortedAgents = React.useMemo(
    () =>
      [...filteredAgents].sort((left, right) =>
        left.name.localeCompare(right.name),
      ),
    [filteredAgents],
  );

  if (isLoading || otherAgents.length === 0) return null;

  return (
    <section className="space-y-3">
      <div className="space-y-1">
        <button
          className="flex w-full items-center gap-2 text-left"
          onClick={() => setIsExpanded((prev) => !prev)}
          type="button"
        >
          {isExpanded ? (
            <ChevronDown className="h-4 w-4 shrink-0 text-muted-foreground" />
          ) : (
            <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground" />
          )}
          <h2 className="text-lg font-semibold tracking-tight">
            External agents
          </h2>
          <span className="text-sm text-muted-foreground">
            ({otherAgents.length})
          </span>
        </button>
        <p className="pl-6 text-sm text-muted-foreground">
          Agents hosted outside this Desktop. You can mention them, but runtime
          controls stay with their host.
        </p>
      </div>

      {isExpanded ? (
        <>
          <div className="relative">
            <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
            <Input
              className="pl-9"
              onChange={(event) => setSearchQuery(event.target.value)}
              placeholder="Search by name, type, or channel..."
              value={searchQuery}
            />
          </div>

          {sortedAgents.length === 0 ? (
            <p className="px-1 py-3 text-sm text-muted-foreground">
              {searchQuery.trim()
                ? "No agents match your search."
                : "No other agents in this community."}
            </p>
          ) : (
            <div
              className="grid w-full grid-cols-[repeat(auto-fill,minmax(220px,240px))] justify-start gap-3"
              data-testid="relay-directory-cards"
            >
              {sortedAgents.map((agent) => {
                const canCustomize =
                  presentationScope !== null &&
                  normalizePubkey(agent.ownerPubkey ?? "") === currentPubkey;
                return (
                  <AgentIdentityCard
                    actions={
                      canCustomize ? (
                        <Button
                          aria-label={`Customize ${agent.name}`}
                          className="h-8 w-8 rounded-full"
                          data-testid={`customize-external-agent-${agent.pubkey}`}
                          onClick={() => setAgentToCustomize(agent)}
                          size="icon"
                          variant="secondary"
                        >
                          <Pencil className="h-4 w-4" />
                        </Button>
                      ) : null
                    }
                    ariaLabel={`${agent.name} external agent profile`}
                    avatarUrl={agent.avatarUrl}
                    dataTestId={`external-agent-card-${agent.pubkey}`}
                    key={agent.pubkey}
                    label={agent.name}
                    modelLabel={agent.agentType || "External runtime"}
                    onClick={() => onOpenAgentProfile(agent.pubkey)}
                    statusBadge={
                      <span className="mt-1 flex flex-wrap items-center gap-1.5">
                        <Badge variant="info">External</Badge>
                        <PresenceBadge
                          className="border-0 bg-transparent px-0 py-0 text-2xs"
                          status={agent.status}
                        />
                      </span>
                    }
                  />
                );
              })}
            </div>
          )}
        </>
      ) : null}

      {error ? (
        <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
          {error.message}
        </p>
      ) : null}

      <ExternalAgentPresentationDialog
        agent={agentToCustomize}
        onOpenChange={(open) => {
          if (!open) setAgentToCustomize(null);
        }}
        open={agentToCustomize !== null}
        scope={presentationScope}
      />
    </section>
  );
}
