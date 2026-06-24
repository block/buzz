import * as React from "react";

import {
  ChevronDown,
  ChevronRight,
  Clipboard,
  Crown,
  Ellipsis,
  FileText,
  Pencil,
  Play,
  Power,
  Square,
  Trash2,
  UserPlus,
} from "lucide-react";
import { toast } from "sonner";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { PresenceDot } from "@/features/presence/ui/PresenceBadge";
import { Badge } from "@/shared/ui/badge";
import { AgentStatusBadge } from "@/features/agents/ui/AgentStatusBadge";
import { useActiveAgentTurns } from "@/features/agents/activeAgentTurnsStore";
import { formatElapsed } from "@/features/agents/ui/agentSessionUtils";
import { useNow } from "@/shared/lib/useNow";
import type {
  ManagedAgent,
  PresenceLookup,
  PresenceStatus,
} from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { EditAgentDialog } from "./EditAgentDialog";
import { friendlyAgentLastError } from "@/features/agents/lib/friendlyAgentLastError";
import { ManagedAgentLogPanel } from "./ManagedAgentLogPanel";
import { ModelPicker } from "./ModelPicker";
import { truncateInstanceId, truncatePubkey } from "./agentUi";
import { useAgentLeadership } from "./useObserverEvents";
import {
  type InstanceLeadership,
  filterStaleInstances,
  selectFreshestLeader,
} from "./leadershipHelpers";
import { claimManagedAgentLeadership } from "@/shared/api/agentControl";

export function ManagedAgentRow({
  agent,
  channelIdToName,
  channelNames,
  isActionPending,
  isLogSelected,
  logContent,
  logError,
  logLoading,
  personaLabelsById,
  presenceLoaded,
  presenceLookup,
  onAddToChannel,
  onDelete,
  onSelectLogAgent,
  onStart,
  onStop,
  onToggleStartOnAppLaunch,
}: {
  agent: ManagedAgent;
  channelIdToName: Record<string, string>;
  channelNames: { id: string; name: string }[];
  isActionPending: boolean;
  isLogSelected: boolean;
  logContent: string | null;
  logError: Error | null;
  logLoading: boolean;
  personaLabelsById: Record<string, string>;
  presenceLoaded: boolean;
  presenceLookup: PresenceLookup;
  onAddToChannel: (agent: ManagedAgent) => void;
  onDelete: (pubkey: string) => void;
  onSelectLogAgent: (pubkey: string | null) => void;
  onStart: (pubkey: string) => void;
  onStop: (pubkey: string) => void;
  onToggleStartOnAppLaunch: (pubkey: string, startOnAppLaunch: boolean) => void;
}) {
  const isActive = agent.status === "running" || agent.status === "deployed";
  const isLocal = agent.backend.type === "local";
  const runtimeSource =
    agent.backend.type === "provider" ? `Remote (${agent.backend.id})` : null;
  const personaLabel = agent.personaId
    ? (personaLabelsById[agent.personaId] ?? null)
    : null;
  const presenceStatus = presenceLookup[agent.pubkey.trim().toLowerCase()];
  const activeTurns = useActiveAgentTurns(agent.pubkey);
  const activeWorkingChannels = React.useMemo(
    () =>
      activeTurns
        .map(({ channelId, anchorAt }) => ({
          id: channelId,
          name: channelIdToName[channelId] ?? channelId,
          anchorAt,
        }))
        .slice(0, 3),
    [activeTurns, channelIdToName],
  );
  const isWorking = activeWorkingChannels.length > 0;
  const processDetail =
    agent.pid !== null
      ? `PID ${agent.pid}`
      : agent.lastExitCode !== null
        ? `Exit ${agent.lastExitCode}`
        : isLocal
          ? "Ready to launch"
          : "Managed remotely";
  // When the harness recovered a meaningful error string from the agent's
  // log tail (Max's seam in `managed_agents/storage.rs`), promote it to
  // user-visible copy below the process detail. Specifically renders the
  // friendly "Relay mesh denied this agent — check your relay membership."
  // for auth failures so the user knows it's a membership thing, not a
  // crash. Generic exits stay verbatim so we don't lie about other failures.
  const friendlyError = friendlyAgentLastError(agent.lastError);

  // Leadership frames flow into the owner-wide observer store regardless of
  // session-panel state, so this is enabled on row visibility (gated only on a
  // pubkey). The 5s clock drives stale eviction without a new frame arriving —
  // a crashed leader's last frame ages out and the badge drops within 15s.
  const leadership = useAgentLeadership(true, agent.pubkey);
  const leadershipNow = useNow(5000);
  const liveInstances = React.useMemo(
    () => filterStaleInstances(leadership, leadershipNow),
    [leadership, leadershipNow],
  );
  const leaderInstanceId = React.useMemo(
    () => selectFreshestLeader(liveInstances)?.instanceId ?? null,
    [liveInstances],
  );

  return (
    <div
      className={cn(
        "overflow-hidden transition-colors",
        isLogSelected ? "bg-primary/5" : "hover:bg-muted/20",
      )}
      data-testid={`managed-agent-${agent.pubkey}`}
    >
      <div className="flex items-start gap-3 px-4 py-3">
        {isLocal ? (
          <button
            aria-expanded={isLogSelected}
            className="-m-1 min-w-0 flex-1 rounded-lg p-1 text-left transition-colors hover:bg-background/40"
            onClick={() =>
              onSelectLogAgent(isLogSelected ? null : agent.pubkey)
            }
            type="button"
          >
            <div className="grid gap-3 lg:grid-cols-[minmax(0,1.8fr)_minmax(120px,0.8fr)_minmax(0,1.1fr)] lg:gap-4">
              <AgentSummary
                activeWorkingChannels={activeWorkingChannels}
                agent={agent}
                channelNames={channelNames}
                isExpandable
                isLogSelected={isLogSelected}
                personaLabel={personaLabel}
                presenceStatus={presenceStatus}
              />
              <StatusBlock
                friendlyError={friendlyError}
                isWorking={isWorking}
                leaderInstanceId={leaderInstanceId}
                presenceLoaded={presenceLoaded}
                presenceStatus={presenceStatus}
                processDetail={processDetail}
                status={agent.status}
              />
              <RuntimeBlock agent={agent} runtimeSource={runtimeSource} />
            </div>
          </button>
        ) : (
          <div className="min-w-0 flex-1">
            <div className="grid gap-3 lg:grid-cols-[minmax(0,1.8fr)_minmax(120px,0.8fr)_minmax(0,1.1fr)] lg:gap-4">
              <AgentSummary
                activeWorkingChannels={activeWorkingChannels}
                agent={agent}
                channelNames={channelNames}
                isExpandable={false}
                isLogSelected={false}
                personaLabel={personaLabel}
                presenceStatus={presenceStatus}
              />
              <StatusBlock
                friendlyError={friendlyError}
                isWorking={isWorking}
                leaderInstanceId={leaderInstanceId}
                presenceLoaded={presenceLoaded}
                presenceStatus={presenceStatus}
                processDetail={processDetail}
                status={agent.status}
              />
              <RuntimeBlock agent={agent} runtimeSource={runtimeSource} />
            </div>
          </div>
        )}

        <div className="flex shrink-0 items-start gap-2 lg:pt-0.5">
          <ModelPicker agent={agent} />
          <AgentActionsMenu
            agent={agent}
            instances={liveInstances}
            isActionPending={isActionPending}
            isActive={isActive}
            leaderInstanceId={leaderInstanceId}
            onAddToChannel={onAddToChannel}
            onDelete={onDelete}
            onOpenLogs={(pubkey) => onSelectLogAgent(pubkey)}
            onStart={onStart}
            onStop={onStop}
            onToggleStartOnAppLaunch={onToggleStartOnAppLaunch}
          />
        </div>
      </div>

      {isLocal && isLogSelected ? (
        <div
          className="border-t border-border/60 bg-background/60 px-4 py-4"
          data-testid="managed-agent-log-row"
        >
          <ManagedAgentLogPanel
            error={logError}
            isLoading={logLoading}
            logContent={logContent}
            selectedAgent={agent}
            variant="inline"
          />
        </div>
      ) : null}
    </div>
  );
}

function AgentSummary({
  activeWorkingChannels,
  agent,
  channelNames,
  isExpandable,
  isLogSelected,
  personaLabel,
  presenceStatus,
}: {
  activeWorkingChannels: { id: string; name: string; anchorAt: number }[];
  agent: ManagedAgent;
  channelNames: { id: string; name: string }[];
  isExpandable: boolean;
  isLogSelected: boolean;
  personaLabel: string | null;
  presenceStatus: PresenceStatus | undefined;
}) {
  const { goChannel } = useAppNavigation();

  return (
    <div className="min-w-0">
      <div className="flex items-start gap-3">
        {isExpandable ? (
          isLogSelected ? (
            <ChevronDown className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
          ) : (
            <ChevronRight className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
          )
        ) : (
          <span className="mt-0.5 h-4 w-4 shrink-0" />
        )}
        {presenceStatus ? (
          <PresenceDot className="mt-1 shrink-0" status={presenceStatus} />
        ) : (
          <span className="mt-1 h-2 w-2 shrink-0" />
        )}
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <p className="truncate font-medium text-foreground">{agent.name}</p>
            {personaLabel ? (
              <Badge variant="secondary">{personaLabel}</Badge>
            ) : null}
            <AgentOriginBadge agent={agent} />
          </div>
          <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
            <span className="font-mono">{truncatePubkey(agent.pubkey)}</span>
            {agent.backend.type === "local" ? (
              <span>
                {agent.startOnAppLaunch ? "Auto-start" : "Manual start"}
              </span>
            ) : (
              <span>Remote deployment</span>
            )}
          </div>
          {channelNames.length > 0 ? (
            <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
              {channelNames.map((channel) => (
                <Badge
                  className="cursor-pointer normal-case tracking-normal hover:opacity-80"
                  key={channel.id}
                  variant="outline"
                  onClick={(e) => {
                    e.stopPropagation();
                    void goChannel(channel.id);
                  }}
                >
                  # {channel.name}
                </Badge>
              ))}
            </div>
          ) : null}
          {activeWorkingChannels.length > 0 ? (
            <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
              {activeWorkingChannels.map((channel) => (
                <WorkingBadge
                  key={`working-${channel.id}`}
                  channelId={channel.id}
                  name={channel.name}
                  anchorAt={channel.anchorAt}
                  onNavigate={goChannel}
                />
              ))}
            </div>
          ) : null}
        </div>
      </div>
    </div>
  );
}

function WorkingBadge({
  channelId,
  name,
  anchorAt,
  onNavigate,
}: {
  channelId: string;
  name: string;
  anchorAt: number;
  onNavigate: (channelId: string) => void;
}) {
  // The 1s tick lives here, at the leaf, so only visible working badges
  // re-render each second — idle rows never mount this hook.
  const now = useNow(1000);

  return (
    <Badge
      className="cursor-pointer motion-safe:animate-pulse normal-case tracking-normal hover:opacity-80"
      variant="default"
      onClick={(e) => {
        e.stopPropagation();
        onNavigate(channelId);
      }}
    >
      Working in #{name} · {formatElapsed(now - anchorAt)}
    </Badge>
  );
}

function StatusBlock({
  friendlyError,
  isWorking,
  leaderInstanceId,
  presenceLoaded,
  presenceStatus,
  processDetail,
  status,
}: {
  friendlyError: ReturnType<typeof friendlyAgentLastError>;
  isWorking: boolean;
  leaderInstanceId: string | null;
  presenceLoaded: boolean;
  presenceStatus: PresenceStatus | undefined;
  processDetail: string;
  status: ManagedAgent["status"];
}) {
  return (
    <div className="space-y-1 lg:pt-0.5">
      <p className="text-2xs font-semibold uppercase tracking-[0.16em] text-muted-foreground lg:hidden">
        Status
      </p>
      <div className="flex flex-wrap items-center gap-1.5">
        <AgentStatusBadge
          isWorking={isWorking}
          presenceLoaded={presenceLoaded}
          presenceStatus={presenceStatus}
          status={status}
        />
        {leaderInstanceId ? (
          <Badge className="gap-1" variant="outline">
            <Crown className="h-3 w-3" />
            Leader
          </Badge>
        ) : null}
      </div>
      <p className="text-xs text-muted-foreground">{processDetail}</p>
      {friendlyError ? (
        <p
          className={cn(
            "text-xs",
            friendlyError.severity === "denied"
              ? "text-destructive"
              : "text-muted-foreground",
          )}
          data-testid="managed-agent-last-error"
        >
          {friendlyError.copy}
        </p>
      ) : null}
    </div>
  );
}

function RuntimeBlock({
  agent,
  runtimeSource,
}: {
  agent: ManagedAgent;
  runtimeSource: string | null;
}) {
  return (
    <div className="space-y-1 lg:pt-0.5">
      <p className="text-2xs font-semibold uppercase tracking-[0.16em] text-muted-foreground lg:hidden">
        Runtime
      </p>
      <p className="truncate font-mono text-xs text-foreground">
        {agent.agentCommand}
      </p>
      {runtimeSource || agent.model ? (
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
          {runtimeSource ? <span>{runtimeSource}</span> : null}
          {agent.model ? <span>{agent.model}</span> : null}
        </div>
      ) : null}
    </div>
  );
}

function AgentActionsMenu({
  agent,
  instances,
  isActionPending,
  isActive,
  leaderInstanceId,
  onAddToChannel,
  onDelete,
  onOpenLogs,
  onStart,
  onStop,
  onToggleStartOnAppLaunch,
}: {
  agent: ManagedAgent;
  instances: InstanceLeadership[];
  isActionPending: boolean;
  isActive: boolean;
  leaderInstanceId: string | null;
  onAddToChannel: (agent: ManagedAgent) => void;
  onDelete: (pubkey: string) => void;
  onOpenLogs: (pubkey: string) => void;
  onStart: (pubkey: string) => void;
  onStop: (pubkey: string) => void;
  onToggleStartOnAppLaunch: (pubkey: string, startOnAppLaunch: boolean) => void;
}) {
  const [editOpen, setEditOpen] = React.useState(false);
  // Nothing to steal unless at least two instances are racing.
  const showLeadershipSubmenu = instances.length > 1;

  return (
    <>
      <DropdownMenu modal={false}>
        <DropdownMenuTrigger asChild>
          <button
            aria-label={`Agent actions for ${agent.name}`}
            className="flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
            data-testid={`managed-agent-actions-${agent.pubkey}`}
            type="button"
          >
            <Ellipsis className="h-4 w-4" />
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent
          align="end"
          onCloseAutoFocus={(event) => event.preventDefault()}
        >
          {agent.backend.type === "provider" ? (
            <>
              <DropdownMenuItem
                disabled={isActionPending}
                onClick={() => onStart(agent.pubkey)}
              >
                <Play className="h-4 w-4" />
                {isActive ? "Redeploy" : "Deploy"}
              </DropdownMenuItem>
              <DropdownMenuItem
                disabled={isActionPending}
                onClick={() => onStop(agent.pubkey)}
              >
                <Square className="h-4 w-4" />
                Shutdown
              </DropdownMenuItem>
            </>
          ) : isActive ? (
            <DropdownMenuItem
              disabled={isActionPending}
              onClick={() => onStop(agent.pubkey)}
            >
              <Square className="h-4 w-4" />
              Stop
            </DropdownMenuItem>
          ) : (
            <DropdownMenuItem
              disabled={isActionPending}
              onClick={() => onStart(agent.pubkey)}
            >
              <Play className="h-4 w-4" />
              Spawn
            </DropdownMenuItem>
          )}

          {agent.backend.type !== "provider" ? (
            <DropdownMenuItem onClick={() => setEditOpen(true)}>
              <Pencil className="h-4 w-4" />
              Edit
            </DropdownMenuItem>
          ) : null}

          <DropdownMenuItem
            disabled={isActionPending}
            onClick={() => onAddToChannel(agent)}
          >
            <UserPlus className="h-4 w-4" />
            Add to channel
          </DropdownMenuItem>

          <DropdownMenuItem
            onClick={async () => {
              await navigator.clipboard.writeText(agent.pubkey);
              toast.success("Copied pubkey to clipboard");
            }}
          >
            <Clipboard className="h-4 w-4" />
            Copy pubkey
          </DropdownMenuItem>

          {agent.backend.type === "local" ? (
            <DropdownMenuItem onClick={() => onOpenLogs(agent.pubkey)}>
              <FileText className="h-4 w-4" />
              View logs
            </DropdownMenuItem>
          ) : null}

          {agent.backend.type === "local" ? (
            <DropdownMenuItem
              disabled={isActionPending}
              onClick={() =>
                onToggleStartOnAppLaunch(agent.pubkey, !agent.startOnAppLaunch)
              }
            >
              <Power className="h-4 w-4" />
              {agent.startOnAppLaunch
                ? "Disable auto-start"
                : "Enable auto-start"}
            </DropdownMenuItem>
          ) : null}

          {showLeadershipSubmenu ? (
            <DropdownMenuSub>
              <DropdownMenuSubTrigger>
                <Crown className="h-4 w-4" />
                Leadership
              </DropdownMenuSubTrigger>
              <DropdownMenuSubContent>
                {instances.map((instance) => {
                  const isLeader = instance.instanceId === leaderInstanceId;
                  return (
                    <DropdownMenuItem
                      disabled={isLeader}
                      key={instance.instanceId}
                      onClick={async () => {
                        try {
                          await claimManagedAgentLeadership(
                            agent.pubkey,
                            instance.instanceId,
                          );
                          toast.success(
                            `Leadership request sent to ${agent.name}.`,
                          );
                        } catch (error) {
                          toast.error(
                            error instanceof Error
                              ? error.message
                              : `Failed to send leadership request to ${agent.name}.`,
                          );
                        }
                      }}
                    >
                      {isLeader ? (
                        <Crown className="h-4 w-4" />
                      ) : (
                        <span className="h-4 w-4" />
                      )}
                      <span className="font-mono">
                        {truncateInstanceId(instance.instanceId)}
                      </span>
                      <span className="ml-auto text-xs text-muted-foreground">
                        {isLeader
                          ? "Leader"
                          : `${formatElapsed(Date.now() - instance.lastSeen)} ago`}
                      </span>
                    </DropdownMenuItem>
                  );
                })}
              </DropdownMenuSubContent>
            </DropdownMenuSub>
          ) : null}

          <DropdownMenuSeparator />

          <DropdownMenuItem
            className="text-destructive focus:text-destructive"
            disabled={isActionPending}
            onClick={() => onDelete(agent.pubkey)}
          >
            <Trash2 className="h-4 w-4" />
            Delete
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>

      {editOpen ? (
        <EditAgentDialog
          agent={agent}
          onOpenChange={setEditOpen}
          open={editOpen}
        />
      ) : null}
    </>
  );
}

function AgentOriginBadge({ agent }: { agent: ManagedAgent }) {
  return (
    <Badge variant="outline">
      {agent.backend.type === "local" ? "Local" : "Remote"}
    </Badge>
  );
}
