import * as React from "react";
import { ChevronDown, ChevronRight } from "lucide-react";

import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import { useUserProfileQuery } from "@/features/profile/hooks";
import { useFeedbackToasts } from "@/shared/hooks/useToastEffect";
import { useFileImportZone } from "@/shared/hooks/useFileImportZone";
import type { AgentPersona, ManagedAgent } from "@/shared/api/types";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Skeleton } from "@/shared/ui/skeleton";
import { AgentIdentityCard } from "./AgentIdentityCard";
import { CreateIdentityCard } from "./CreateIdentityCard";

type UnifiedAgentsSectionProps = {
  actionErrorMessage: string | null;
  actionNoticeMessage: string | null;
  agents: ManagedAgent[];
  agentsError: Error | null;
  isAgentsLoading: boolean;
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
  onImportPersonaFile: (fileBytes: number[], fileName: string) => void;
};

type PersonaGroup = { persona: AgentPersona; agents: ManagedAgent[] };

const AGENT_CARD_COLUMN_CLASS = "w-full";
const AGENT_CARD_GRID_CLASS = `${AGENT_CARD_COLUMN_CLASS} grid grid-cols-[repeat(auto-fill,minmax(220px,240px))] justify-start gap-3`;

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
    agentsError,
    isAgentsLoading,
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
    onImportPersonaFile,
  } = props;

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
            Drop .md, .persona.json, .persona.png, or .zip to import
          </p>
        </div>
      ) : null}

      <SectionHeader
        fileInputRef={fileInputRef}
        handleFileChange={handleFileChange}
      />

      {isLoading ? <LoadingSkeleton /> : null}

      {!isLoading ? (
        <div className="space-y-3" data-testid="unified-agents-groups">
          <div className={AGENT_CARD_GRID_CLASS}>
            {groups.map((g) => {
              const profileAgent = pickProfileAgent(g.agents);
              return (
                <AgentPersonaCard
                  agent={profileAgent}
                  key={g.persona.id}
                  persona={g.persona}
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
              onToggle={toggle}
              onOpenAgentProfile={onOpenAgentProfile}
            />
          ) : null}
          {ungrouped.length > 0 ? (
            <CollapsibleAgentGroup
              agents={ungrouped}
              collapsed={collapsed}
              groupKey="__ungrouped__"
              label="Custom Agents"
              onToggle={toggle}
              onOpenAgentProfile={onOpenAgentProfile}
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

function pickProfileAgent(agents: ManagedAgent[]) {
  return [...agents].sort((left, right) => {
    const activeDiff =
      Number(isManagedAgentActive(right)) - Number(isManagedAgentActive(left));
    if (activeDiff !== 0) return activeDiff;
    return left.name.localeCompare(right.name);
  })[0];
}

function AgentPersonaCard({
  agent,
  persona,
  onOpenAgentProfile,
  onOpenPersonaProfile,
}: {
  agent: ManagedAgent | undefined;
  persona: AgentPersona;
  onOpenAgentProfile: (pubkey: string) => void;
  onOpenPersonaProfile: (persona: AgentPersona) => void;
}) {
  const title = persona.displayName;
  const modelLabel = formatAgentModelLabel(agent?.model ?? persona.model);
  const profileQuery = useUserProfileQuery(agent?.pubkey);
  const avatarUrl = agent
    ? (profileQuery.data?.avatarUrl ?? null)
    : persona.avatarUrl;

  return (
    <AgentIdentityCard
      ariaLabel={`${title} agent profile`}
      avatarUrl={avatarUrl}
      dataTestId={`persona-agent-row-${persona.id}`}
      label={title}
      modelLabel={modelLabel}
      onClick={() => {
        if (agent) {
          onOpenAgentProfile(agent.pubkey);
          return;
        }
        onOpenPersonaProfile(persona);
      }}
    />
  );
}

function StandaloneAgentCard({
  agent,
  onOpenAgentProfile,
}: {
  agent: ManagedAgent;
  onOpenAgentProfile: (pubkey: string) => void;
}) {
  const title = agent.name;
  const profileQuery = useUserProfileQuery(agent.pubkey);

  return (
    <AgentIdentityCard
      ariaLabel={`${title} agent profile`}
      avatarUrl={profileQuery.data?.avatarUrl ?? null}
      dataTestId={`managed-agent-${agent.pubkey}`}
      label={title}
      modelLabel={formatAgentModelLabel(agent.model)}
      onClick={() => onOpenAgentProfile(agent.pubkey)}
    />
  );
}

function formatAgentModelLabel(model: string | null | undefined) {
  const trimmed = model?.trim();
  return trimmed && trimmed.length > 0 ? trimmed : "Auto";
}

function SectionHeader({
  fileInputRef,
  handleFileChange,
}: {
  fileInputRef: React.RefObject<HTMLInputElement | null>;
  handleFileChange: (e: React.ChangeEvent<HTMLInputElement>) => void;
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
  );
}

function LoadingSkeleton() {
  return (
    <div className={AGENT_CARD_GRID_CLASS}>
      {["a", "b", "c"].map((k) => (
        <div
          className="flex aspect-[5/7] w-full max-w-[240px] flex-col items-center justify-center rounded-lg border border-border/70 bg-card p-5"
          key={k}
        >
          <div className="relative">
            <Skeleton className="h-32 w-32 rounded-full" />
            <Skeleton className="-translate-x-1/2 absolute bottom-0 left-1/2 h-8 w-24 rounded-full" />
          </div>
          <Skeleton className="mt-10 h-4 w-40 max-w-full" />
          <Skeleton className="mt-2 h-4 w-32 max-w-full" />
        </div>
      ))}
    </div>
  );
}

function CollapsibleAgentGroup({
  groupKey,
  label,
  agents,
  collapsed,
  onToggle,
  onOpenAgentProfile,
}: {
  groupKey: string;
  label: string;
  agents: ManagedAgent[];
  collapsed: ReadonlySet<string>;
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
              onOpenAgentProfile={onOpenAgentProfile}
            />
          ))}
        </div>
      ) : null}
    </div>
  );
}
