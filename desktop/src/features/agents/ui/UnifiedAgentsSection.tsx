import * as React from "react";
import {
  ChevronDown,
  ChevronRight,
  Ellipsis,
  OctagonX,
  Plus,
  Trash2,
} from "lucide-react";

import { isPersonaActive } from "@/features/agents/lib/catalog";
import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import { useFeedbackToasts } from "@/shared/hooks/useToastEffect";
import { useFileImportZone } from "@/shared/hooks/useFileImportZone";
import type {
  AgentPersona,
  ManagedAgent,
  PresenceLookup,
} from "@/shared/api/types";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { Card } from "@/shared/ui/card";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Skeleton } from "@/shared/ui/skeleton";
import { AgentGroupRows } from "./AgentGroupRows";
import { PersonaActionsMenu } from "./PersonaActionsMenu";
import { PersonaIdentity } from "./PersonaIdentity";
import { PersonaLibraryEntryPoints } from "./PersonaLibraryEntryPoints";

type UnifiedAgentsSectionProps = {
  actionErrorMessage: string | null;
  actionNoticeMessage: string | null;
  agents: ManagedAgent[];
  channelIdToName: Record<string, string>;
  channelsByPubkey: Record<string, { id: string; name: string }[]>;
  agentsError: Error | null;
  isActionPending: boolean;
  isAgentsLoading: boolean;
  logContent: string | null;
  logError: Error | null;
  logLoading: boolean;
  personaLabelsById: Record<string, string>;
  presenceLoaded: boolean;
  presenceLookup: PresenceLookup;
  onAddToChannel: (agent: ManagedAgent) => void;
  onBulkRemoveStopped: () => void;
  onBulkStopRunning: () => void;
  onCreateAgent: () => void;
  onDeleteAgent: (pubkey: string) => void;
  onSelectLogAgent: (pubkey: string | null) => void;
  onStartAgent: (pubkey: string) => void;
  onStopAgent: (pubkey: string) => void;
  onToggleStartOnAppLaunch: (pubkey: string, startOnAppLaunch: boolean) => void;
  selectedLogAgentPubkey: string | null;
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
  onExportPersona: (persona: AgentPersona) => void;
  onDeactivatePersona: (persona: AgentPersona) => void;
  onDeletePersona: (persona: AgentPersona) => void;
  onImportPersonaFile: (fileBytes: number[], fileName: string) => void;
};

type PersonaGroup = { persona: AgentPersona; agents: ManagedAgent[] };

function buildUnifiedGroups(personas: AgentPersona[], agents: ManagedAgent[]) {
  const byPersonaId = new Map<string, ManagedAgent[]>();
  const ungrouped: ManagedAgent[] = [];

  for (const agent of agents) {
    if (!agent.personaId) {
      ungrouped.push(agent);
    } else {
      const list = byPersonaId.get(agent.personaId) ?? [];
      list.push(agent);
      byPersonaId.set(agent.personaId, list);
    }
  }

  const matched = new Set<string>();
  const groups: PersonaGroup[] = personas.map((p) => {
    matched.add(p.id);
    return { persona: p, agents: byPersonaId.get(p.id) ?? [] };
  });

  const unknown: ManagedAgent[] = [];
  for (const [id, list] of byPersonaId) {
    if (!matched.has(id)) unknown.push(...list);
  }

  return { groups, ungrouped, unknown };
}

export function UnifiedAgentsSection(props: UnifiedAgentsSectionProps) {
  const {
    actionErrorMessage,
    actionNoticeMessage,
    agents,
    channelIdToName,
    channelsByPubkey,
    agentsError,
    isActionPending,
    isAgentsLoading,
    logContent,
    logError,
    logLoading,
    personaLabelsById,
    presenceLoaded,
    presenceLookup,
    onAddToChannel,
    onBulkRemoveStopped,
    onBulkStopRunning,
    onCreateAgent,
    onDeleteAgent,
    onSelectLogAgent,
    onStartAgent,
    onStopAgent,
    onToggleStartOnAppLaunch,
    selectedLogAgentPubkey,
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
    onExportPersona,
    onDeactivatePersona,
    onDeletePersona,
    onImportPersonaFile,
  } = props;

  const runningCount = agents.filter((a) => isManagedAgentActive(a)).length;
  const stoppedCount = agents.filter(
    (a) => a.status === "stopped" || a.status === "not_deployed",
  ).length;
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

  const rowProps = {
    channelIdToName,
    channelsByPubkey,
    isActionPending,
    logContent,
    logError,
    logLoading,
    personaLabelsById,
    presenceLoaded,
    presenceLookup,
    selectedLogAgentPubkey,
    onAddToChannel,
    onDelete: onDeleteAgent,
    onSelectLogAgent,
    onStart: onStartAgent,
    onStop: onStopAgent,
    onToggleStartOnAppLaunch,
  } as const;

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
        canChooseCatalog={canChooseCatalog}
        fileInputRef={fileInputRef}
        handleFileChange={handleFileChange}
        isActionPending={isActionPending}
        isPersonasPending={isPersonasPending}
        openFilePicker={openFilePicker}
        runningCount={runningCount}
        stoppedCount={stoppedCount}
        onBulkRemoveStopped={onBulkRemoveStopped}
        onBulkStopRunning={onBulkStopRunning}
        onChooseCatalog={onChooseCatalog}
        onCreateAgent={onCreateAgent}
        onCreatePersona={onCreatePersona}
      />

      {isLoading ? <LoadingSkeleton /> : null}

      {!isLoading && personas.length === 0 && agents.length === 0 ? (
        <EmptyState
          canChooseCatalog={canChooseCatalog}
          isPersonasPending={isPersonasPending}
          openFilePicker={openFilePicker}
          onChooseCatalog={onChooseCatalog}
          onCreatePersona={onCreatePersona}
        />
      ) : null}

      {!isLoading && (personas.length > 0 || agents.length > 0) ? (
        <div className="space-y-3" data-testid="unified-agents-groups">
          {groups.map((g) => {
            const isCollapsed = collapsed.has(g.persona.id);
            const hasAgents = g.agents.length > 0;
            const isDeactivated = !isPersonaActive(g.persona);
            return (
              <div
                key={g.persona.id}
                className={`overflow-hidden rounded-xl border border-border/70 bg-card/40${isDeactivated ? " opacity-60" : ""}`}
              >
                <div className="flex items-center gap-2 px-3 py-2 transition-colors hover:bg-muted/40">
                  <button
                    className="flex min-w-0 flex-1 items-center gap-2 py-1 text-left"
                    onClick={() => toggle(g.persona.id)}
                    type="button"
                  >
                    {isCollapsed ? (
                      <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground" />
                    ) : (
                      <ChevronDown className="h-4 w-4 shrink-0 text-muted-foreground" />
                    )}
                    <PersonaIdentity
                      persona={g.persona}
                      showPromptTooltip={false}
                    />
                    <span className="shrink-0 text-xs text-muted-foreground">
                      {hasAgents
                        ? `${g.agents.length} instance${g.agents.length === 1 ? "" : "s"}`
                        : "Not deployed"}
                    </span>
                  </button>
                  <div className="flex shrink-0 items-center gap-1">
                    {isDeactivated ? (
                      <Badge variant="outline">Deactivated</Badge>
                    ) : !hasAgents ? (
                      <Badge variant="outline">Inactive</Badge>
                    ) : null}
                    <PersonaActionsMenu
                      isActionPending={isActionPending}
                      isPending={isPersonasPending}
                      persona={g.persona}
                      onDuplicate={onDuplicatePersona}
                      onEdit={onEditPersona}
                      onExport={onExportPersona}
                      onDeactivate={onDeactivatePersona}
                      onDelete={onDeletePersona}
                    />
                  </div>
                </div>
                {!isCollapsed && hasAgents ? (
                  <AgentGroupRows agents={g.agents} {...rowProps} />
                ) : null}
              </div>
            );
          })}

          {unknown.length > 0 ? (
            <CollapsibleAgentGroup
              agents={unknown}
              collapsed={collapsed}
              groupKey="__unknown__"
              label="Unknown Persona"
              rowProps={rowProps}
              onToggle={toggle}
            />
          ) : null}
          {ungrouped.length > 0 ? (
            <CollapsibleAgentGroup
              agents={ungrouped}
              collapsed={collapsed}
              groupKey="__ungrouped__"
              label="Custom Agents"
              rowProps={rowProps}
              onToggle={toggle}
            />
          ) : null}
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

      {agentsError ? (
        <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
          {agentsError.message}
        </p>
      ) : null}
      {personasError ? (
        <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
          {personasError.message}
        </p>
      ) : null}
    </section>
  );
}

function SectionHeader({
  agentCount,
  canChooseCatalog,
  fileInputRef,
  handleFileChange,
  isActionPending,
  isPersonasPending,
  openFilePicker,
  runningCount,
  stoppedCount,
  onBulkRemoveStopped,
  onBulkStopRunning,
  onChooseCatalog,
  onCreateAgent,
  onCreatePersona,
}: {
  agentCount: number;
  canChooseCatalog: boolean;
  fileInputRef: React.RefObject<HTMLInputElement | null>;
  handleFileChange: (e: React.ChangeEvent<HTMLInputElement>) => void;
  isActionPending: boolean;
  isPersonasPending: boolean;
  openFilePicker: () => void;
  runningCount: number;
  stoppedCount: number;
  onBulkRemoveStopped: () => void;
  onBulkStopRunning: () => void;
  onChooseCatalog: () => void;
  onCreateAgent: () => void;
  onCreatePersona: () => void;
}) {
  return (
    <div className="flex items-center justify-between gap-3">
      <div>
        <h3 className="text-sm font-semibold tracking-tight">Your Agents</h3>
        <p className="text-sm text-muted-foreground">
          Personas and their deployed agent instances.
        </p>
      </div>
      <input
        accept=".md,.json,.png,.zip"
        className="hidden"
        onChange={handleFileChange}
        ref={fileInputRef}
        type="file"
      />
      <div className="flex items-center gap-2">
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
        <DropdownMenu modal={false}>
          <DropdownMenuTrigger asChild>
            <Button size="sm" type="button" variant="default">
              <Plus className="h-4 w-4" />
              New
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent
            align="end"
            onCloseAutoFocus={(e) => e.preventDefault()}
          >
            <DropdownMenuItem
              disabled={isPersonasPending}
              onClick={onCreatePersona}
            >
              Persona
            </DropdownMenuItem>
            {canChooseCatalog ? (
              <DropdownMenuItem
                disabled={isPersonasPending}
                onClick={onChooseCatalog}
              >
                Choose from Catalog...
              </DropdownMenuItem>
            ) : null}
            <DropdownMenuSeparator />
            <DropdownMenuItem onClick={onCreateAgent}>
              Custom Agent
            </DropdownMenuItem>
            <DropdownMenuItem onClick={openFilePicker}>
              Import persona file
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </div>
  );
}

function LoadingSkeleton() {
  return (
    <Card className="overflow-hidden">
      {["a", "b", "c"].map((k) => (
        <div
          className="flex items-center gap-4 border-b border-border/60 px-4 py-3 last:border-b-0"
          key={k}
        >
          <Skeleton className="h-8 w-8 rounded-lg" />
          <Skeleton className="h-4 w-28" />
          <Skeleton className="h-5 w-16 rounded-full" />
        </div>
      ))}
    </Card>
  );
}

function EmptyState({
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
    <div className="rounded-xl border border-dashed border-primary/40 px-6 py-10 text-center">
      <p className="text-sm font-semibold tracking-tight">No agents yet</p>
      <p className="mt-2 text-sm text-muted-foreground">
        Create a persona or choose one from the catalog, then deploy it to a
        channel.
      </p>
      <div className="mt-4 flex flex-wrap items-center justify-center gap-2">
        <PersonaLibraryEntryPoints
          canChooseCatalog={canChooseCatalog}
          isPending={isPersonasPending}
          layout="empty"
          onCreate={onCreatePersona}
          onChooseCatalog={onChooseCatalog}
          onImport={openFilePicker}
        />
      </div>
    </div>
  );
}

function CollapsibleAgentGroup({
  groupKey,
  label,
  agents,
  collapsed,
  onToggle,
  rowProps,
}: {
  groupKey: string;
  label: string;
  agents: ManagedAgent[];
  collapsed: ReadonlySet<string>;
  onToggle: (key: string) => void;
  rowProps: Omit<React.ComponentProps<typeof AgentGroupRows>, "agents">;
}) {
  const isCollapsed = collapsed.has(groupKey);
  return (
    <div className="overflow-hidden rounded-xl border border-border/70 bg-card/40">
      <div className="px-3 py-2 transition-colors hover:bg-muted/40">
        <button
          className="flex w-full items-center gap-2 py-1 text-left"
          onClick={() => onToggle(groupKey)}
          type="button"
        >
          {isCollapsed ? (
            <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground" />
          ) : (
            <ChevronDown className="h-4 w-4 shrink-0 text-muted-foreground" />
          )}
          <span className="text-sm font-medium">{label}</span>
          <span className="text-xs text-muted-foreground">
            ({agents.length})
          </span>
        </button>
      </div>
      {!isCollapsed ? <AgentGroupRows agents={agents} {...rowProps} /> : null}
    </div>
  );
}
