import * as React from "react";
import {
  ChevronDown,
  ChevronRight,
  Ellipsis,
  OctagonX,
  Trash2,
} from "lucide-react";

import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import { useFeedbackToasts } from "@/shared/hooks/useToastEffect";
import type { ManagedAgent, PresenceLookup } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import { Card } from "@/shared/ui/card";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Skeleton } from "@/shared/ui/skeleton";
import { CreateNewButton } from "./CreateNewButton";
import { ManagedAgentRow } from "./ManagedAgentRow";

type PersonaGroup = {
  key: string;
  label: string;
  agents: ManagedAgent[];
};

function groupAgentsByPersona(
  agents: ManagedAgent[],
  personaLabelsById: Record<string, string>,
): PersonaGroup[] {
  const grouped = new Map<string, ManagedAgent[]>();
  const ungrouped: ManagedAgent[] = [];
  const unknownPersona: ManagedAgent[] = [];

  for (const agent of agents) {
    if (!agent.personaId) {
      ungrouped.push(agent);
    } else if (personaLabelsById[agent.personaId]) {
      const existing = grouped.get(agent.personaId) ?? [];
      existing.push(agent);
      grouped.set(agent.personaId, existing);
    } else {
      unknownPersona.push(agent);
    }
  }

  const groups: PersonaGroup[] = [];

  for (const [personaId, groupAgents] of grouped) {
    groups.push({
      key: personaId,
      label: personaLabelsById[personaId],
      agents: groupAgents,
    });
  }

  if (unknownPersona.length > 0) {
    groups.push({
      key: "__unknown__",
      label: "Unknown Persona",
      agents: unknownPersona,
    });
  }

  if (ungrouped.length > 0) {
    groups.push({
      key: "__ungrouped__",
      label: "Custom Agents",
      agents: ungrouped,
    });
  }

  return groups;
}

export function ManagedAgentsSection({
  actionErrorMessage,
  actionNoticeMessage,
  agents,
  channelsByPubkey,
  error,
  isActionPending,
  isLoading,
  logContent,
  logError,
  logLoading,
  personaLabelsById,
  presenceLoaded,
  presenceLookup,
  onAddToChannel,
  onBulkRemoveStopped,
  onBulkStopRunning,
  onCreate,
  onDelete,
  onSelectLogAgent,
  onStart,
  onStop,
  onToggleStartOnAppLaunch,
  selectedLogAgentPubkey,
}: {
  actionErrorMessage: string | null;
  actionNoticeMessage: string | null;
  agents: ManagedAgent[];
  channelsByPubkey: Record<string, string[]>;
  error: Error | null;
  isActionPending: boolean;
  isLoading: boolean;
  logContent: string | null;
  logError: Error | null;
  logLoading: boolean;
  personaLabelsById: Record<string, string>;
  presenceLoaded: boolean;
  presenceLookup: PresenceLookup;
  onAddToChannel: (agent: ManagedAgent) => void;
  onBulkRemoveStopped: () => void;
  onBulkStopRunning: () => void;
  onCreate: () => void;
  onDelete: (pubkey: string) => void;
  onSelectLogAgent: (pubkey: string | null) => void;
  onStart: (pubkey: string) => void;
  onStop: (pubkey: string) => void;
  onToggleStartOnAppLaunch: (pubkey: string, startOnAppLaunch: boolean) => void;
  selectedLogAgentPubkey: string | null;
}) {
  const runningCount = agents.filter((a) => isManagedAgentActive(a)).length;
  const stoppedCount = agents.filter(
    (a) => a.status === "stopped" || a.status === "not_deployed",
  ).length;

  const groups = React.useMemo(
    () => groupAgentsByPersona(agents, personaLabelsById),
    [agents, personaLabelsById],
  );

  const [collapsedGroups, setCollapsedGroups] = React.useState<Set<string>>(
    new Set(),
  );

  function toggleGroup(key: string) {
    setCollapsedGroups((prev) => {
      const next = new Set(prev);
      if (next.has(key)) {
        next.delete(key);
      } else {
        next.add(key);
      }
      return next;
    });
  }

  useFeedbackToasts(actionNoticeMessage, actionErrorMessage);

  return (
    <section className="space-y-4">
      <div className="flex items-center justify-between gap-3">
        <div>
          <h3 className="text-sm font-semibold tracking-tight">
            Managed agents
          </h3>
          <p className="text-sm text-muted-foreground">
            Agent profiles and process state — local and remote.
          </p>
        </div>
        <div className="flex items-center gap-2">
          {agents.length > 0 ? (
            <DropdownMenu modal={false}>
              <DropdownMenuTrigger asChild>
                <Button
                  aria-label="Bulk actions"
                  className="h-7 w-7"
                  size="icon"
                  variant="ghost"
                >
                  <Ellipsis className="h-4 w-4" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent
                align="end"
                onCloseAutoFocus={(e) => e.preventDefault()}
              >
                <DropdownMenuItem
                  disabled={isActionPending || runningCount === 0}
                  onClick={onBulkStopRunning}
                >
                  <OctagonX className="h-4 w-4" />
                  Stop all running ({runningCount})
                </DropdownMenuItem>
                <DropdownMenuItem
                  className="text-destructive focus:text-destructive"
                  disabled={isActionPending || stoppedCount === 0}
                  onClick={onBulkRemoveStopped}
                >
                  <Trash2 className="h-4 w-4" />
                  Remove all stopped ({stoppedCount})
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          ) : null}
          <CreateNewButton
            ariaLabel="Create agent"
            label="Agent"
            onClick={onCreate}
          />
        </div>
      </div>

      {isLoading ? (
        <Card className="overflow-hidden">
          {["first", "second"].map((key) => (
            <div
              className="flex items-center gap-4 border-b border-border/60 px-4 py-3 last:border-b-0"
              key={key}
            >
              <Skeleton className="h-4 w-28" />
              <Skeleton className="h-5 w-16 rounded-full" />
              <Skeleton className="h-4 w-24" />
              <Skeleton className="h-4 w-20" />
            </div>
          ))}
        </Card>
      ) : null}

      {!isLoading && agents.length === 0 ? (
        <div className="rounded-xl border border-dashed border-border/80 bg-card/70 px-6 py-10 text-center">
          <p className="text-sm font-semibold tracking-tight">
            No local agents yet
          </p>
          <p className="mt-2 text-sm text-muted-foreground">
            Create one to generate a keypair, mint a token, and launch the ACP
            harness from the desktop app.
          </p>
        </div>
      ) : null}

      {!isLoading && agents.length > 0 ? (
        <div className="space-y-3" data-testid="managed-agents-table">
          {groups.map((group) => {
            const isCollapsed = collapsedGroups.has(group.key);
            return (
              <div key={group.key} className="space-y-2">
                <button
                  className="flex w-full items-center gap-2 rounded-lg px-1 py-1 text-left transition-colors hover:bg-muted/40"
                  onClick={() => toggleGroup(group.key)}
                  type="button"
                >
                  {isCollapsed ? (
                    <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground" />
                  ) : (
                    <ChevronDown className="h-4 w-4 shrink-0 text-muted-foreground" />
                  )}
                  <span className="text-sm font-medium">{group.label}</span>
                  <span className="text-xs text-muted-foreground">
                    ({group.agents.length})
                  </span>
                </button>
                {!isCollapsed ? (
                  <div className="space-y-2">
                    {group.agents.map((agent) => (
                      <ManagedAgentRow
                        agent={agent}
                        channelNames={channelsByPubkey[agent.pubkey] ?? []}
                        isActionPending={isActionPending}
                        isLogSelected={selectedLogAgentPubkey === agent.pubkey}
                        key={agent.pubkey}
                        logContent={
                          selectedLogAgentPubkey === agent.pubkey
                            ? logContent
                            : null
                        }
                        logError={
                          selectedLogAgentPubkey === agent.pubkey
                            ? logError
                            : null
                        }
                        logLoading={
                          selectedLogAgentPubkey === agent.pubkey && logLoading
                        }
                        personaLabelsById={personaLabelsById}
                        presenceLoaded={presenceLoaded}
                        presenceLookup={presenceLookup}
                        onAddToChannel={onAddToChannel}
                        onDelete={onDelete}
                        onSelectLogAgent={onSelectLogAgent}
                        onStart={onStart}
                        onStop={onStop}
                        onToggleStartOnAppLaunch={onToggleStartOnAppLaunch}
                      />
                    ))}
                  </div>
                ) : null}
              </div>
            );
          })}
        </div>
      ) : null}

      {!isLoading && stoppedCount > 0 ? (
        <div className="flex items-center justify-between rounded-xl border border-border/60 bg-muted/30 px-4 py-2.5">
          <p className="text-sm text-muted-foreground">
            {stoppedCount} stopped {stoppedCount === 1 ? "agent" : "agents"}
          </p>
          <Button
            className="text-destructive"
            disabled={isActionPending}
            onClick={onBulkRemoveStopped}
            size="sm"
            variant="ghost"
          >
            <Trash2 className="mr-1.5 h-3.5 w-3.5" />
            Remove stopped
          </Button>
        </div>
      ) : null}

      {error ? (
        <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
          {error.message}
        </p>
      ) : null}
    </section>
  );
}
