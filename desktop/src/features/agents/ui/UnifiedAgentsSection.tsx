import * as React from "react";
import {
  ChevronDown,
  ChevronRight,
  Ellipsis,
  OctagonX,
  Trash2,
} from "lucide-react";

import { useActiveAgentTurns } from "@/features/agents/activeAgentTurnsStore";
import { formatAgentModelLabel } from "@/features/agents/lib/formatAgentModelLabel";
import { friendlyAgentLastError } from "@/features/agents/lib/friendlyAgentLastError";
import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import { AgentStatusBadge } from "@/features/agents/ui/AgentStatusBadge";
import { ModelPicker } from "@/features/agents/ui/ModelPicker";
import { useUserProfileQuery } from "@/features/profile/hooks";
import type {
  AgentPersona,
  ManagedAgent,
  PresenceLookup,
} from "@/shared/api/types";
import { useFeedbackToasts } from "@/shared/hooks/useToastEffect";
import { useFileImportZone } from "@/shared/hooks/useFileImportZone";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { IdentityCardSkeleton } from "@/shared/ui/identity-card-skeleton";
import { AgentIdentityCard } from "./AgentIdentityCard";
import { CreateIdentityCard } from "./CreateIdentityCard";
import { PersonaActionsMenu } from "./PersonaActionsMenu";
import { buildUnifiedGroups, pickProfileAgent } from "./unifiedAgentGroups";

type UnifiedAgentsSectionProps = {
  actionErrorMessage: string | null;
  actionNoticeMessage: string | null;
  agents: ManagedAgent[];
  channelIdToName: Record<string, string>;
  channelsByPubkey: Record<string, { id: string; name: string }[]>;
  agentsError: Error | null;
  isActionPending: boolean;
  isAgentsLoading: boolean;
  personaLabelsById: Record<string, string>;
  presenceLoaded: boolean;
  presenceLookup: PresenceLookup;
  onBulkRemoveStopped: () => void;
  onBulkStopRunning: () => void;
  onCreateAgent: () => void;
  onOpenAgentProfile: (pubkey: string) => void;
  onOpenPersonaProfile: (persona: AgentPersona) => void;
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
  onSharePersona: (persona: AgentPersona) => void;
  onDeactivatePersona: (persona: AgentPersona) => void;
  onDeletePersona: (persona: AgentPersona) => void;
  onImportPersonaFile: (fileBytes: number[], fileName: string) => void;
};

const AGENT_CARD_COLUMN_CLASS = "w-full";
const AGENT_CARD_GRID_CLASS = `${AGENT_CARD_COLUMN_CLASS} grid grid-cols-[repeat(auto-fill,minmax(220px,240px))] justify-start gap-3`;

export function UnifiedAgentsSection(props: UnifiedAgentsSectionProps) {
  const {
    actionErrorMessage,
    actionNoticeMessage,
    agents,
    agentsError,
    isActionPending,
    isAgentsLoading,
    presenceLoaded,
    presenceLookup,
    onBulkRemoveStopped,
    onBulkStopRunning,
    onCreateAgent,
    onOpenAgentProfile,
    onOpenPersonaProfile,
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
    onImportPersonaFile,
  } = props;

  const runningCount = agents.filter((agent) =>
    isManagedAgentActive(agent),
  ).length;
  const stoppedCount = agents.filter(
    (agent) => agent.status === "stopped" || agent.status === "not_deployed",
  ).length;
  const { groups, ungrouped, unknown } = React.useMemo(
    () => buildUnifiedGroups(personas, agents),
    [personas, agents],
  );
  const additionalPersonaAgents = React.useMemo(() => {
    const additional: ManagedAgent[] = [];
    for (const group of groups) {
      const primary = pickProfileAgent(group.agents);
      for (const agent of group.agents) {
        if (primary?.pubkey !== agent.pubkey) {
          additional.push(agent);
        }
      }
    }
    return additional;
  }, [groups]);
  const [collapsed, setCollapsed] = React.useState<Set<string>>(new Set());
  const {
    fileInputRef,
    isDragOver,
    dropHandlers,
    handleFileChange,
    openFilePicker,
  } = useFileImportZone({ onImportFile: onImportPersonaFile });

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
            Drop .persona.md, .persona.json, .persona.png, or .zip to import
          </p>
        </div>
      ) : null}

      <SectionHeader
        agentCount={agents.length}
        fileInputRef={fileInputRef}
        handleFileChange={handleFileChange}
        isActionPending={isActionPending}
        runningCount={runningCount}
        stoppedCount={stoppedCount}
        onBulkRemoveStopped={onBulkRemoveStopped}
        onBulkStopRunning={onBulkStopRunning}
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
                      onDeactivate={onDeactivatePersona}
                      onDelete={onDeletePersona}
                      onDuplicate={onDuplicatePersona}
                      onEdit={onEditPersona}
                      onShare={onSharePersona}
                    />
                  }
                  agent={profileAgent}
                  key={group.persona.id}
                  persona={group.persona}
                  presenceLoaded={presenceLoaded}
                  presenceLookup={presenceLookup}
                  onOpenAgentProfile={onOpenAgentProfile}
                  onOpenPersonaProfile={onOpenPersonaProfile}
                />
              );
            })}
            <NewAgentCard
              canChooseCatalog={canChooseCatalog}
              isPersonasPending={isPersonasPending}
              openFilePicker={openFilePicker}
              onChooseCatalog={onChooseCatalog}
              onCreateAgent={onCreateAgent}
              onCreatePersona={onCreatePersona}
            />
          </div>

          {additionalPersonaAgents.length > 0 ? (
            <CollapsibleAgentGroup
              agents={additionalPersonaAgents}
              collapsed={collapsed}
              groupKey="__additional_persona_agents__"
              label="Additional agent instances"
              presenceLoaded={presenceLoaded}
              presenceLookup={presenceLookup}
              onToggle={toggle}
              onOpenAgentProfile={onOpenAgentProfile}
            />
          ) : null}
          {unknown.length > 0 ? (
            <CollapsibleAgentGroup
              agents={unknown}
              collapsed={collapsed}
              groupKey="__unknown__"
              label="Unknown Persona"
              presenceLoaded={presenceLoaded}
              presenceLookup={presenceLookup}
              onToggle={toggle}
              onOpenAgentProfile={onOpenAgentProfile}
            />
          ) : null}
          {ungrouped.length > 0 ? (
            <CollapsibleAgentGroup
              agents={ungrouped}
              collapsed={collapsed}
              groupKey="__ungrouped__"
              label="Custom agents"
              presenceLoaded={presenceLoaded}
              presenceLookup={presenceLookup}
              onToggle={toggle}
              onOpenAgentProfile={onOpenAgentProfile}
            />
          ) : null}
        </div>
      ) : null}

      {!isLoading && stoppedCount > 0 ? (
        <div
          className={`${AGENT_CARD_COLUMN_CLASS} flex items-center justify-between rounded-xl border border-border/60 bg-muted/30 px-4 py-2.5`}
        >
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
            <Trash2 className="mr-1.5 h-4 w-4" />
            Remove stopped
          </Button>
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
  persona,
  presenceLoaded,
  presenceLookup,
  onOpenAgentProfile,
  onOpenPersonaProfile,
}: {
  actions?: React.ReactNode;
  agent: ManagedAgent | undefined;
  persona: AgentPersona;
  presenceLoaded: boolean;
  presenceLookup: PresenceLookup;
  onOpenAgentProfile: (pubkey: string) => void;
  onOpenPersonaProfile: (persona: AgentPersona) => void;
}) {
  const title = persona.displayName;
  const modelLabel = formatAgentModelLabel(agent?.model ?? persona.model);
  const profileQuery = useUserProfileQuery(agent?.pubkey);
  const avatarUrl = agent
    ? firstAvatarUrl(profileQuery.data?.avatarUrl, persona.avatarUrl)
    : persona.avatarUrl;
  const friendlyError = agent
    ? friendlyAgentLastError(agent.lastError)?.copy
    : null;

  return (
    <AgentIdentityCard
      actions={actions}
      ariaLabel={`${title} agent profile`}
      avatarUrl={avatarUrl}
      dataTestId={`persona-agent-row-${persona.id}`}
      errorLabel={friendlyError}
      label={title}
      modelControl={agent ? <ModelPicker agent={agent} /> : undefined}
      modelLabel={modelLabel}
      onClick={() => {
        if (agent) {
          onOpenAgentProfile(agent.pubkey);
          return;
        }
        onOpenPersonaProfile(persona);
      }}
      status={
        agent ? (
          <AgentCardStatus
            agent={agent}
            presenceLoaded={presenceLoaded}
            presenceLookup={presenceLookup}
          />
        ) : null
      }
    />
  );
}

function StandaloneAgentCard({
  agent,
  presenceLoaded,
  presenceLookup,
  onOpenAgentProfile,
}: {
  agent: ManagedAgent;
  presenceLoaded: boolean;
  presenceLookup: PresenceLookup;
  onOpenAgentProfile: (pubkey: string) => void;
}) {
  const title = agent.name;
  const profileQuery = useUserProfileQuery(agent.pubkey);
  const friendlyError = friendlyAgentLastError(agent.lastError)?.copy;

  return (
    <AgentIdentityCard
      ariaLabel={`${title} agent profile`}
      avatarUrl={profileQuery.data?.avatarUrl}
      dataTestId={`managed-agent-${agent.pubkey}`}
      errorLabel={friendlyError}
      label={title}
      modelControl={<ModelPicker agent={agent} />}
      modelLabel={formatAgentModelLabel(agent.model)}
      onClick={() => {
        onOpenAgentProfile(agent.pubkey);
      }}
      status={
        <AgentCardStatus
          agent={agent}
          presenceLoaded={presenceLoaded}
          presenceLookup={presenceLookup}
        />
      }
    />
  );
}

function AgentCardStatus({
  agent,
  presenceLoaded,
  presenceLookup,
}: {
  agent: ManagedAgent;
  presenceLoaded: boolean;
  presenceLookup: PresenceLookup;
}) {
  const activeTurns = useActiveAgentTurns(agent.pubkey);
  const presenceStatus = presenceLookup[normalizePubkey(agent.pubkey)];

  return (
    <AgentStatusBadge
      isWorking={activeTurns.length > 0}
      presenceLoaded={presenceLoaded}
      presenceStatus={presenceStatus}
      status={agent.status}
    />
  );
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

function SectionHeader({
  agentCount,
  fileInputRef,
  handleFileChange,
  isActionPending,
  runningCount,
  stoppedCount,
  onBulkRemoveStopped,
  onBulkStopRunning,
}: {
  agentCount: number;
  fileInputRef: React.RefObject<HTMLInputElement | null>;
  handleFileChange: (e: React.ChangeEvent<HTMLInputElement>) => void;
  isActionPending: boolean;
  runningCount: number;
  stoppedCount: number;
  onBulkRemoveStopped: () => void;
  onBulkStopRunning: () => void;
}) {
  return (
    <div
      className={`${AGENT_CARD_COLUMN_CLASS} flex items-center justify-between gap-3`}
    >
      <div>
        <h3 className="text-sm font-semibold tracking-tight">Your Agents</h3>
        <p className="text-sm text-secondary-foreground/75">
          Agents in this workspace.
        </p>
      </div>
      <input
        accept=".md,.json,.png,.zip"
        className="hidden"
        onChange={handleFileChange}
        ref={fileInputRef}
        type="file"
      />
      {agentCount > 0 ? (
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
            onCloseAutoFocus={(event) => event.preventDefault()}
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
    </div>
  );
}

function NewAgentCard({
  canChooseCatalog,
  isPersonasPending,
  openFilePicker,
  onChooseCatalog,
  onCreateAgent,
  onCreatePersona,
}: {
  canChooseCatalog: boolean;
  isPersonasPending: boolean;
  openFilePicker: () => void;
  onChooseCatalog: () => void;
  onCreateAgent: () => void;
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
          New agent
        </DropdownMenuItem>
        {canChooseCatalog ? (
          <DropdownMenuItem
            disabled={isPersonasPending}
            onClick={onChooseCatalog}
          >
            Choose from catalog
          </DropdownMenuItem>
        ) : null}
        <DropdownMenuSeparator />
        <DropdownMenuItem onClick={onCreateAgent}>
          Custom agent
        </DropdownMenuItem>
        <DropdownMenuItem onClick={openFilePicker}>
          Import persona file
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
  presenceLoaded,
  presenceLookup,
  onToggle,
  onOpenAgentProfile,
}: {
  groupKey: string;
  label: string;
  agents: ManagedAgent[];
  collapsed: ReadonlySet<string>;
  presenceLoaded: boolean;
  presenceLookup: PresenceLookup;
  onToggle: (key: string) => void;
  onOpenAgentProfile: (pubkey: string) => void;
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
              key={agent.pubkey}
              presenceLoaded={presenceLoaded}
              presenceLookup={presenceLookup}
              onOpenAgentProfile={onOpenAgentProfile}
            />
          ))}
        </div>
      ) : null}
    </div>
  );
}
