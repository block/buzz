import * as React from "react";
import { ChevronDown, ChevronRight, Search } from "lucide-react";

import type { RelayAgent } from "@/shared/api/types";
import { PresenceBadge } from "@/features/presence/ui/PresenceBadge";
import { Card } from "@/shared/ui/card";
import { Input } from "@/shared/ui/input";
import { truncatePubkey } from "./agentUi";

export function RelayDirectorySection({
  error,
  isLoading,
  managedPubkeys,
  relayAgents,
}: {
  error: Error | null;
  isLoading: boolean;
  managedPubkeys: Set<string>;
  relayAgents: RelayAgent[];
}) {
  const [isExpanded, setIsExpanded] = React.useState(false);
  const [searchQuery, setSearchQuery] = React.useState("");

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
        <h3 className="text-sm font-semibold tracking-tight">
          Relay directory
        </h3>
        <span className="text-sm text-muted-foreground">
          ({otherAgents.length} other agent{otherAgents.length !== 1 ? "s" : ""}
          )
        </span>
      </button>

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
                : "No other agents on this relay."}
            </p>
          ) : (
            <Card className="overflow-hidden">
              <div className="overflow-x-auto">
                <table
                  className="w-full border-collapse text-left text-sm"
                  data-testid="relay-directory-table"
                >
                  <thead className="bg-muted/35 text-2xs font-semibold uppercase tracking-[0.16em] text-muted-foreground">
                    <tr>
                      <th className="px-4 py-3">Agent</th>
                      <th className="px-4 py-3">Status</th>
                      <th className="px-4 py-3">Type</th>
                      <th className="px-4 py-3">Channels</th>
                    </tr>
                  </thead>
                  <tbody>
                    {sortedAgents.map((agent) => (
                      <tr
                        className="border-b border-border/60 last:border-b-0"
                        key={agent.pubkey}
                      >
                        <td className="min-w-[16rem] px-4 py-3 align-top">
                          <div className="min-w-0">
                            <p className="truncate font-medium text-foreground">
                              {agent.name}
                            </p>
                            <p className="mt-1 text-xs text-muted-foreground">
                              {truncatePubkey(agent.pubkey)}
                            </p>
                          </div>
                        </td>
                        <td className="px-4 py-3 align-top">
                          <PresenceBadge
                            className="px-2.5 py-0.5 text-2xs"
                            status={agent.status}
                          />
                        </td>
                        <td className="px-4 py-3 align-top text-muted-foreground">
                          {agent.agentType || "Unknown"}
                        </td>
                        <td className="max-w-[20rem] px-4 py-3 align-top text-muted-foreground">
                          <span className="block truncate">
                            {agent.channels.length > 0
                              ? agent.channels.join(", ")
                              : "No visible channel memberships"}
                          </span>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </Card>
          )}
        </>
      ) : null}

      {error ? (
        <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
          {error.message}
        </p>
      ) : null}
    </section>
  );
}
