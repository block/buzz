import * as React from "react";
import { ChevronDown, ChevronRight, RefreshCw } from "lucide-react";

import { useChannelsQuery } from "@/features/channels/hooks";
import { isChannelOpenable } from "@/features/agents/useOpenAgentActivity";
import {
  useAgentWorking,
  type AgentWorkingChannel,
} from "@/features/agents/agentWorkingSignal";
import { useLastCompletedTurn } from "@/features/agents/activeAgentTurnsStore";
import { useOpenAgentActivity } from "@/features/agents/useOpenAgentActivity";
import {
  type AgentCardStatus,
  deriveAgentCardStatus,
  formatAgentCardActivityChannel,
} from "@/features/agents/lib/agentCardStatus";
import { formatAgentModelLabel } from "@/features/agents/lib/formatAgentModelLabel";
import { friendlyAgentLastError } from "@/features/agents/lib/friendlyAgentLastError";
import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import { formatElapsed } from "@/features/agents/ui/agentSessionUtils";
import { useUserProfileQuery } from "@/features/profile/hooks";
import type { AgentPersona, ManagedAgent } from "@/shared/api/types";
import type { ProfilePanelOpenOptions } from "@/shared/context/ProfilePanelContext";
import { useFeedbackToasts } from "@/shared/hooks/useToastEffect";
import { useFileImportZone } from "@/shared/hooks/useFileImportZone";
import { useNow } from "@/shared/lib/useNow";
import { Badge } from "@/shared/ui/badge";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { IdentityCardSkeleton } from "@/shared/ui/identity-card-skeleton";
import { AgentIdentityCard } from "./AgentIdentityCard";
import { AgentRuntimeAvatarControl } from "./AgentRuntimeAvatarControl";
import { CreateIdentityCard } from "./CreateIdentityCard";
import { PersonaActionsMenu } from "./PersonaActionsMenu";
import { buildUnifiedGroups, pickProfileAgent } from "./unifiedAgentGroups";

type UnifiedAgentsSectionProps = {
  defaultModel: string;
  actionErrorMessage: string | null;
  actionNoticeMessage: string | null;
  agents: ManagedAgent[];
  agentsError: Error | null;
  isActionPending: boolean;
  isAgentsLoading: boolean;
  startingAgentPubkey: string | null;
  startingPersonaIds: ReadonlySet<string>;
  onOpenAgentProfile: (
    pubkey: string,
    options?: ProfilePanelOpenOptions,
  ) => void;
  onOpenPersonaProfile: (persona: AgentPersona) => void;
  onStartAgent: (pubkey: string) => void;
  onStartPersona: (persona: AgentPersona) => void;
  canChooseCatalog: boolean;
  personas: AgentPersona[];
  personasError: Error | null;
  personaFeedbackErrorMessage: string | null;
  personaFeedbackNoticeMessage: string | null;
  isPersonasLoading: boolean;
  isPersonasPending: boolean;
  onCreatePersona: () => void;
  onChooseCatalog: () => void;
  onDuplicatePersona: (persona: AgentPersona) => void;
  onEditPersona: (persona: AgentPersona) => void;
  onSharePersona: (
    persona: AgentPersona,
    linkedAgent: ManagedAgent | undefined,
  ) => void;
  onDeactivatePersona: (persona: AgentPersona) => void;
  onDeletePersona: (persona: AgentPersona) => void;
  onImportSnapshotFile: (fileBytes: number[], fileName: string) => void;
};

const AGENT_CARD_COLUMN_CLASS = "w-full";
const AGENT_CARD_GRID_CLASS = `${AGENT_CARD_COLUMN_CLASS} grid grid-cols-[repeat(auto-fill,minmax(220px,240px))] justify-start gap-3`;

export function UnifiedAgentsSection(props: UnifiedAgentsSectionProps) {
  const {
    actionErrorMessage,
    actionNoticeMessage,
    defaultModel,
    agents,
    agentsError,
    isActionPending,
    isAgentsLoading,
    startingAgentPubkey,
    startingPersonaIds,
    onOpenAgentProfile,
    onOpenPersonaProfile,
    onStartAgent,
    onStartPersona,
    canChooseCatalog,
    personas,
    personasError,
    personaFeedbackErrorMessage,
    personaFeedbackNoticeMessage,
    isPersonasLoading,
    isPersonasPending,
    onCreatePersona,
    onChooseCatalog,
    onDuplicatePersona,
    onEditPersona,
    onSharePersona,
    onDeactivatePersona,
    onDeletePersona,
    onImportSnapshotFile,
  } = props;

  const { groups, ungrouped, unknown } = React.useMemo(
    () => buildUnifiedGroups(personas, agents),
    [personas, agents],
  );
  const [collapsed, setCollapsed] = React.useState<Set<string>>(new Set());
  const {
    fileInputRef,
    isDragOver,
    dropHandlers,
    handleFileChange,
    openFilePicker,
  } = useFileImportZone({ onImportFile: onImportSnapshotFile });

  function toggle(key: string) {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }

  useFeedbackToasts(actionNoticeMessage, actionErrorMessage);
  useFeedbackToasts(personaFeedbackNoticeMessage, personaFeedbackErrorMessage);
  const isLoading = isAgentsLoading || isPersonasLoading;

  return (
    <section
      className="relative space-y-4"
      data-testid="agents-library-personas"
      {...dropHandlers}
    >
      {isDragOver ? (
        <div className="pointer-events-none absolute -inset-1 z-10 flex items-center justify-center rounded-2xl border-2 border-dashed border-primary/50 bg-background/80 backdrop-blur-sm">
          <p className="text-sm font-medium text-primary">
            Drop .agent.json or .agent.png to import
          </p>
        </div>
      ) : null}

      <input
        accept=".agent.json,.agent.png"
        className="hidden"
        onChange={handleFileChange}
        ref={fileInputRef}
        type="file"
      />

      {isLoading ? <LoadingSkeleton /> : null}

      {!isLoading ? (
        <div className="space-y-3" data-testid="unified-agents-groups">
          <div className={AGENT_CARD_GRID_CLASS}>
            {groups.map((group) => {
              const profileAgent = pickProfileAgent(group.agents);
              return (
                <AgentPersonaCard
                  actions={
                    <PersonaActionsMenu
                      isActionPending={isActionPending}
                      isPending={isPersonasPending}
                      persona={group.persona}
                      linkedAgent={profileAgent}
                      onDeactivate={onDeactivatePersona}
                      onDelete={onDeletePersona}
                      onDuplicate={onDuplicatePersona}
                      onEdit={onEditPersona}
                      onShare={onSharePersona}
                    />
                  }
                  agent={profileAgent}
                  defaultModel={defaultModel}
                  key={group.persona.id}
                  persona={group.persona}
                  startingAgentPubkey={startingAgentPubkey}
                  startingPersonaIds={startingPersonaIds}
                  onOpenAgentProfile={onOpenAgentProfile}
                  onOpenPersonaProfile={onOpenPersonaProfile}
                  onStartAgent={onStartAgent}
                  onStartPersona={onStartPersona}
                />
              );
            })}
            <NewAgentCard
              canChooseCatalog={canChooseCatalog}
              isPersonasPending={isPersonasPending}
              openFilePicker={openFilePicker}
              onChooseCatalog={onChooseCatalog}
              onCreatePersona={onCreatePersona}
            />
          </div>

          {unknown.length > 0 ? (
            <CollapsibleAgentGroup
              agents={unknown}
              collapsed={collapsed}
              defaultModel={defaultModel}
              groupKey="__unknown__"
              label="Unknown agents"
              startingAgentPubkey={startingAgentPubkey}
              onToggle={toggle}
              onOpenAgentProfile={onOpenAgentProfile}
              onStartAgent={onStartAgent}
            />
          ) : null}
          {ungrouped.length > 0 ? (
            <CollapsibleAgentGroup
              agents={ungrouped}
              collapsed={collapsed}
              defaultModel={defaultModel}
              groupKey="__ungrouped__"
              label="Custom agents"
              startingAgentPubkey={startingAgentPubkey}
              onToggle={toggle}
              onOpenAgentProfile={onOpenAgentProfile}
              onStartAgent={onStartAgent}
            />
          ) : null}
        </div>
      ) : null}

      {agentsError ? (
        <p
          className={`${AGENT_CARD_COLUMN_CLASS} rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive`}
        >
          {agentsError.message}
        </p>
      ) : null}
      {personasError ? (
        <p
          className={`${AGENT_CARD_COLUMN_CLASS} rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive`}
        >
          {personasError.message}
        </p>
      ) : null}
    </section>
  );
}

function AgentPersonaCard({
  actions,
  agent,
  defaultModel,
  persona,
  startingAgentPubkey,
  startingPersonaIds,
  onOpenAgentProfile,
  onOpenPersonaProfile,
  onStartAgent,
  onStartPersona,
}: {
  actions?: React.ReactNode;
  agent: ManagedAgent | undefined;
  defaultModel: string;
  persona: AgentPersona;
  startingAgentPubkey: string | null;
  startingPersonaIds: ReadonlySet<string>;
  onOpenAgentProfile: (
    pubkey: string,
    options?: ProfilePanelOpenOptions,
  ) => void;
  onOpenPersonaProfile: (persona: AgentPersona) => void;
  onStartAgent: (pubkey: string) => void;
  onStartPersona: (persona: AgentPersona) => void;
}) {
  const title = persona.displayName;
  const explicitModel = agent?.model ?? persona.model;
  const modelLabel = explicitModel?.trim()
    ? formatAgentModelLabel(explicitModel)
    : formatDefaultModelLabel(defaultModel);
  const isActive = agent ? isManagedAgentActive(agent) : false;
  const profileQuery = useUserProfileQuery(agent?.pubkey);
  const avatarUrl = agent
    ? firstAvatarUrl(persona.avatarUrl, profileQuery.data?.avatarUrl)
    : persona.avatarUrl;
  const friendlyError = agent
    ? friendlyAgentLastError(agent.lastError, agent.lastErrorCode)?.copy
    : null;
  const opensRuntimeTab = Boolean(agent && friendlyError && !isActive);
  const cardRuntime = useAgentCardRuntime(agent, Boolean(friendlyError));

  return (
    <AgentIdentityCard
      actions={actions}
      activity={cardRuntime.activity}
      ariaLabel={`${title} agent profile`}
      avatar={
        agent ? (
          <AgentRuntimeAvatarControl
            activeTestId={`agent-runtime-active-${agent.pubkey}`}
            avatarUrl={avatarUrl}
            errorLabel={friendlyError}
            errorTestId={`agent-runtime-error-${agent.pubkey}`}
            isActive={isActive}
            isStarting={startingAgentPubkey === agent.pubkey}
            label={title}
            startTestId={`agent-runtime-start-${agent.pubkey}`}
            onOpenError={() => {
              onOpenAgentProfile(agent.pubkey, { tab: "runtime" });
            }}
            onStart={() => onStartAgent(agent.pubkey)}
          />
        ) : (
          <AgentRuntimeAvatarControl
            activeTestId={`persona-runtime-active-${persona.id}`}
            avatarUrl={avatarUrl}
            isActive={false}
            isStarting={startingPersonaIds.has(persona.id)}
            label={title}
            startTestId={`persona-runtime-start-${persona.id}`}
            onStart={() => onStartPersona(persona)}
          />
        )
      }
      avatarUrl={avatarUrl}
      dataTestId={`persona-agent-row-${persona.id}`}
      label={title}
      modelLabel={modelLabel}
      onClick={() => {
        if (agent) {
          onOpenAgentProfile(
            agent.pubkey,
            opensRuntimeTab ? { tab: "runtime" } : undefined,
          );
          return;
        }
        onOpenPersonaProfile(persona);
      }}
      statusBadge={
        <AgentCardStatusBadges
          needsRestart={agent?.needsRestart ?? false}
          status={cardRuntime.status}
        />
      }
    />
  );
}

function StandaloneAgentCard({
  agent,
  defaultModel,
  startingAgentPubkey,
  onOpenAgentProfile,
  onStartAgent,
}: {
  agent: ManagedAgent;
  defaultModel: string;
  startingAgentPubkey: string | null;
  onOpenAgentProfile: (
    pubkey: string,
    options?: ProfilePanelOpenOptions,
  ) => void;
  onStartAgent: (pubkey: string) => void;
}) {
  const title = agent.name;
  const profileQuery = useUserProfileQuery(agent.pubkey);
  const friendlyError = friendlyAgentLastError(
    agent.lastError,
    agent.lastErrorCode,
  )?.copy;
  const isActive = isManagedAgentActive(agent);
  const opensRuntimeTab = Boolean(friendlyError && !isActive);
  const cardRuntime = useAgentCardRuntime(agent, Boolean(friendlyError));

  return (
    <AgentIdentityCard
      activity={cardRuntime.activity}
      ariaLabel={`${title} agent profile`}
      avatar={
        <AgentRuntimeAvatarControl
          activeTestId={`agent-runtime-active-${agent.pubkey}`}
          avatarUrl={profileQuery.data?.avatarUrl}
          errorLabel={friendlyError}
          errorTestId={`agent-runtime-error-${agent.pubkey}`}
          isActive={isActive}
          isStarting={startingAgentPubkey === agent.pubkey}
          label={title}
          startTestId={`agent-runtime-start-${agent.pubkey}`}
          onOpenError={() => {
            onOpenAgentProfile(agent.pubkey, { tab: "runtime" });
          }}
          onStart={() => onStartAgent(agent.pubkey)}
        />
      }
      avatarUrl={profileQuery.data?.avatarUrl}
      dataTestId={`managed-agent-${agent.pubkey}`}
      label={title}
      modelLabel={
        agent.model?.trim()
          ? formatAgentModelLabel(agent.model)
          : formatDefaultModelLabel(defaultModel)
      }
      onClick={() => {
        onOpenAgentProfile(
          agent.pubkey,
          opensRuntimeTab ? { tab: "runtime" } : undefined,
        );
      }}
      statusBadge={
        <AgentCardStatusBadges
          needsRestart={agent.needsRestart}
          status={cardRuntime.status}
        />
      }
    />
  );
}

function useAgentCardRuntime(
  agent: ManagedAgent | undefined,
  hasError: boolean,
) {
  const working = useAgentWorking(agent?.pubkey);
  const lastCompleted = useLastCompletedTurn(agent?.pubkey);
  const channelsQuery = useChannelsQuery();
  const { openAgentActivity } = useOpenAgentActivity();
  const visibleChannelNames = React.useMemo(
    () =>
      new Map(
        (channelsQuery.data ?? [])
          .filter((channel) => isChannelOpenable(channel))
          .map((channel) => [channel.id, channel.name]),
      ),
    [channelsQuery.data],
  );
  const status = deriveAgentCardStatus({
    hasError,
    isWorking: working.working,
    status: agent?.status ?? null,
  });

  const activeChannel =
    working.channels.find((channel) =>
      visibleChannelNames.has(channel.channelId),
    ) ?? working.channels[0];
  const activity =
    agent && status === "working" && activeChannel ? (
      <AgentCardCurrentActivity
        agentPubkey={agent.pubkey}
        channel={activeChannel}
        channelName={visibleChannelNames.get(activeChannel.channelId)}
        onOpen={openAgentActivity}
      />
    ) : agent && lastCompleted ? (
      <AgentCardLastActivity
        agentPubkey={agent.pubkey}
        channelId={lastCompleted.channelId}
        channelName={
          lastCompleted.channelId
            ? visibleChannelNames.get(lastCompleted.channelId)
            : undefined
        }
        completedAt={lastCompleted.completedAt}
        onOpen={openAgentActivity}
      />
    ) : null;

  return { activity, status };
}

function AgentCardStatusBadges({
  needsRestart,
  status,
}: {
  needsRestart: boolean;
  status: AgentCardStatus;
}) {
  const statusPresentation = {
    working: { label: "Bezig", variant: "default" as const },
    available: { label: "Beschikbaar", variant: "success" as const },
    error: { label: "Fout", variant: "destructive" as const },
    off: { label: "Uit", variant: "secondary" as const },
  }[status];

  return (
    <div className="mt-1 flex flex-wrap gap-1">
      <Badge
        className={
          status === "working" ? "motion-safe:animate-pulse" : undefined
        }
        data-testid={`agent-card-status-${status}`}
        variant={statusPresentation.variant}
      >
        {statusPresentation.label}
      </Badge>
      {needsRestart ? (
        <Badge className="gap-1" variant="warning">
          <RefreshCw className="h-3 w-3" />
          Restart required
        </Badge>
      ) : null}
    </div>
  );
}

function AgentCardCurrentActivity({
  agentPubkey,
  channel,
  channelName,
  onOpen,
}: {
  agentPubkey: string;
  channel: AgentWorkingChannel;
  channelName: string | undefined;
  onOpen: (pubkey: string, options?: { channelId?: string | null }) => boolean;
}) {
  const now = useNow(1000);
  return (
    <AgentCardActivityButton
      channelId={channel.channelId}
      label={`Nu: ${formatAgentCardActivityChannel(channelName)} · ${formatElapsed(now - channel.anchorAt)}`}
      testId="agent-card-current-activity"
      onOpen={() =>
        onOpen(agentPubkey, {
          channelId: channel.channelId,
        })
      }
    />
  );
}

function AgentCardLastActivity({
  agentPubkey,
  channelId,
  channelName,
  completedAt,
  onOpen,
}: {
  agentPubkey: string;
  channelId: string | null;
  channelName: string | undefined;
  completedAt: number;
  onOpen: (pubkey: string, options?: { channelId?: string | null }) => boolean;
}) {
  return (
    <AgentCardActivityButton
      channelId={channelId}
      label={`Laatst: ${formatAgentCardActivityChannel(channelName)} · ${new Intl.DateTimeFormat(
        undefined,
        { hour: "2-digit", minute: "2-digit" },
      ).format(completedAt)}`}
      testId="agent-card-last-activity"
      onOpen={() =>
        onOpen(agentPubkey, {
          channelId,
        })
      }
    />
  );
}

function AgentCardActivityButton({
  channelId,
  label,
  testId,
  onOpen,
}: {
  channelId: string | null;
  label: string;
  testId: string;
  onOpen: () => void;
}) {
  return (
    <button
      className="block w-full truncate rounded-sm text-left text-xs text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
      data-testid={testId}
      disabled={!channelId}
      onClick={(event) => {
        event.stopPropagation();
        onOpen();
      }}
      title={label}
      type="button"
    >
      {label}
    </button>
  );
}

function formatDefaultModelLabel(defaultModel: string) {
  const model = defaultModel.trim();
  return model ? `Default model (${model})` : "Default model";
}

function firstAvatarUrl(
  ...candidates: Array<string | null | undefined>
): string | null {
  for (const candidate of candidates) {
    const trimmed = candidate?.trim();
    if (trimmed) return trimmed;
  }
  return null;
}

function NewAgentCard({
  canChooseCatalog,
  isPersonasPending,
  openFilePicker,
  onChooseCatalog,
  onCreatePersona,
}: {
  canChooseCatalog: boolean;
  isPersonasPending: boolean;
  openFilePicker: () => void;
  onChooseCatalog: () => void;
  onCreatePersona: () => void;
}) {
  return (
    <DropdownMenu modal={false}>
      <DropdownMenuTrigger asChild>
        <CreateIdentityCard
          ariaLabel="New agent"
          dataTestId="new-agent-card"
          label="New agent"
        />
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="start"
        onCloseAutoFocus={(event) => event.preventDefault()}
      >
        <DropdownMenuItem
          disabled={isPersonasPending}
          onClick={onCreatePersona}
        >
          Create from scratch
        </DropdownMenuItem>
        {canChooseCatalog ? (
          <DropdownMenuItem
            disabled={isPersonasPending}
            onClick={onChooseCatalog}
          >
            Choose from catalog
          </DropdownMenuItem>
        ) : null}
        <DropdownMenuItem
          data-testid="import-agent-snapshot-menu-item"
          onClick={openFilePicker}
        >
          Import agent snapshot
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function LoadingSkeleton() {
  return (
    <div className={AGENT_CARD_GRID_CLASS}>
      <IdentityCardSkeleton
        footerSubtitleWidthClass="w-14"
        footerTitleWidthClass="w-24"
      />
      <IdentityCardSkeleton
        footerSubtitleWidthClass="w-20"
        footerTitleWidthClass="w-32"
      />
      <IdentityCardSkeleton
        footerSubtitleWidthClass="w-16"
        footerTitleWidthClass="w-28"
      />
    </div>
  );
}

function CollapsibleAgentGroup({
  groupKey,
  label,
  agents,
  collapsed,
  defaultModel,
  startingAgentPubkey,
  onToggle,
  onOpenAgentProfile,
  onStartAgent,
}: {
  groupKey: string;
  label: string;
  agents: ManagedAgent[];
  collapsed: ReadonlySet<string>;
  defaultModel: string;
  startingAgentPubkey: string | null;
  onToggle: (key: string) => void;
  onOpenAgentProfile: (
    pubkey: string,
    options?: ProfilePanelOpenOptions,
  ) => void;
  onStartAgent: (pubkey: string) => void;
}) {
  const isCollapsed = collapsed.has(groupKey);
  return (
    <div className={`${AGENT_CARD_COLUMN_CLASS} space-y-2`}>
      <button
        className="group flex items-center gap-2 rounded-md px-1 py-1 text-left transition-colors hover:bg-muted/50"
        onClick={() => onToggle(groupKey)}
        type="button"
      >
        {isCollapsed ? (
          <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground transition-transform group-hover:translate-x-0.5" />
        ) : (
          <ChevronDown className="h-4 w-4 shrink-0 text-muted-foreground" />
        )}
        <span className="text-sm font-medium">{label}</span>
        <span className="text-xs text-muted-foreground">({agents.length})</span>
      </button>
      {!isCollapsed ? (
        <div className={AGENT_CARD_GRID_CLASS}>
          {agents.map((agent) => (
            <StandaloneAgentCard
              agent={agent}
              defaultModel={defaultModel}
              key={agent.pubkey}
              startingAgentPubkey={startingAgentPubkey}
              onOpenAgentProfile={onOpenAgentProfile}
              onStartAgent={onStartAgent}
            />
          ))}
        </div>
      ) : null}
    </div>
  );
}
