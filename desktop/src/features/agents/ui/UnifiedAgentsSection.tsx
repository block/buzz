import * as React from "react";
import { AlertTriangle, ChevronDown, ChevronRight } from "lucide-react";

import {
  isAgentCardAvatarLoading,
  resolveAgentCardAvatarUrl,
} from "@/features/agents/lib/agentCardAvatar";
import { resolveAgentCardModelLabel } from "@/features/agents/lib/agentCardModelLabel";
import { effectiveAgentDescription } from "@/features/agents/lib/agentDescription";
import { friendlyAgentLastError } from "@/features/agents/lib/friendlyAgentLastError";
import type { AgentAvailabilityReader } from "@/features/agents/lib/useAgentAvailability";
import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import { pickProfileAgent } from "@/features/agents/lib/pickProfileAgent";
import { useIsArchivedPredicate } from "@/features/identity-archive/hooks";
import { useUserProfileQuery } from "@/features/profile/hooks";
import type { AgentPersona, ManagedAgent } from "@/shared/api/types";
import type { ProfilePanelOpenOptions } from "@/shared/context/ProfilePanelContext";
import { useFeedbackToasts } from "@/shared/hooks/useToastEffect";
import { Badge } from "@/shared/ui/badge";
import {
  ProtectedBestieCardBadge,
  useProtectedBestiePubkey,
} from "@protected-feature-components";
import { IdentityCardSkeleton } from "@/shared/ui/identity-card-skeleton";
import { AgentIdentityCard } from "./AgentIdentityCard";
import { AgentRuntimeAvatarControl } from "./AgentRuntimeAvatarControl";
import { CreateIdentityCard } from "./CreateIdentityCard";
import { PersonaActionsMenu } from "./PersonaActionsMenu";
import { buildUnifiedGroups } from "./unifiedAgentGroups";

type UnifiedAgentsSectionProps = {
  defaultModel: string;
  getAvailability: AgentAvailabilityReader;
  actionErrorMessage: string | null;
  actionNoticeMessage: string | null;
  agents: ManagedAgent[];
  agentsError: Error | null;
  isActionPending: boolean;
  isAgentsLoading: boolean;
  restartingAgentPubkey: string | null;
  startingAgentPubkey: string | null;
  startingPersonaIds: ReadonlySet<string>;
  onOpenAgentProfile: (
    pubkey: string,
    options?: ProfilePanelOpenOptions,
  ) => void;
  onOpenPersonaProfile: (persona: AgentPersona) => void;
  onRestartAgent: (pubkey: string) => void;
  onStartAgent: (pubkey: string) => void;
  onStartPersona: (persona: AgentPersona) => void;
  personas: AgentPersona[];
  personasError: Error | null;
  personaFeedbackErrorMessage: string | null;
  personaFeedbackNoticeMessage: string | null;
  isPersonasLoading: boolean;
  isPersonasPending: boolean;
  onOpenCatalog: () => void;
  onDuplicatePersona: (persona: AgentPersona) => void;
  onEditPersona: (persona: AgentPersona) => void;
  onSharePersona: (
    persona: AgentPersona,
    linkedAgent: ManagedAgent | undefined,
    effectiveAvatarUrl: string | null,
  ) => void;
  onDeactivatePersona: (persona: AgentPersona) => void;
  onDeletePersona: (persona: AgentPersona) => void;
};

const AGENT_CARD_COLUMN_CLASS = "w-full";
export const AGENT_CARD_GRID_COLUMNS_CLASS =
  "grid-cols-1 [@container(min-width:21rem)]:grid-cols-2 [@container(min-width:32rem)]:grid-cols-3 [@container(min-width:43rem)]:grid-cols-4 [@container(min-width:54rem)]:grid-cols-5";
export const IDENTITY_CARD_GRID_CLASS = `${AGENT_CARD_COLUMN_CLASS} ${AGENT_CARD_GRID_COLUMNS_CLASS} grid gap-3`;

export function UnifiedAgentsSection(props: UnifiedAgentsSectionProps) {
  const {
    actionErrorMessage,
    actionNoticeMessage,
    defaultModel,
    getAvailability,
    agents,
    agentsError,
    isActionPending,
    isAgentsLoading,
    restartingAgentPubkey,
    startingAgentPubkey,
    startingPersonaIds,
    onOpenAgentProfile,
    onOpenPersonaProfile,
    onRestartAgent,
    onStartAgent,
    onStartPersona,
    personas,
    personasError,
    personaFeedbackErrorMessage,
    personaFeedbackNoticeMessage,
    isPersonasLoading,
    isPersonasPending,
    onOpenCatalog,
    onDuplicatePersona,
    onEditPersona,
    onSharePersona,
    onDeactivatePersona,
    onDeletePersona,
  } = props;

  const isArchived = useIsArchivedPredicate();
  const bestiePubkey = useProtectedBestiePubkey(agents)?.toLowerCase() ?? null;
  const { groups, ungrouped, unknown } = React.useMemo(
    () => buildUnifiedGroups(personas, agents, isArchived),
    [personas, agents, isArchived],
  );
  const [collapsed, setCollapsed] = React.useState<Set<string>>(new Set());
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
    >
      {isLoading ? <LoadingSkeleton /> : null}

      {!isLoading ? (
        <div className="space-y-3" data-testid="unified-agents-groups">
          <div className={IDENTITY_CARD_GRID_CLASS}>
            <CreateIdentityCard
              ariaLabel="New agent"
              dataTestId="new-agent-card"
              disabled={isPersonasPending}
              onClick={onOpenCatalog}
            />
            {groups.map((group) => {
              const profileAgent = pickProfileAgent(group.agents, isArchived);
              return (
                <AgentPersonaCard
                  actions={(effectiveAvatarUrl, isEffectiveAvatarLoading) => (
                    <PersonaActionsMenu
                      isActionPending={
                        isActionPending || isEffectiveAvatarLoading
                      }
                      isPending={isPersonasPending}
                      persona={group.persona}
                      linkedAgent={profileAgent}
                      onDeactivate={onDeactivatePersona}
                      onDelete={onDeletePersona}
                      onDuplicate={onDuplicatePersona}
                      onEdit={onEditPersona}
                      onShare={(persona, linkedAgent) =>
                        onSharePersona(persona, linkedAgent, effectiveAvatarUrl)
                      }
                    />
                  )}
                  agent={profileAgent}
                  getAvailability={getAvailability}
                  defaultModel={defaultModel}
                  isBestie={profileAgent?.pubkey.toLowerCase() === bestiePubkey}
                  key={group.persona.id}
                  persona={group.persona}
                  restartingAgentPubkey={restartingAgentPubkey}
                  startingAgentPubkey={startingAgentPubkey}
                  startingPersonaIds={startingPersonaIds}
                  onOpenAgentProfile={onOpenAgentProfile}
                  onOpenPersonaProfile={onOpenPersonaProfile}
                  onRestartAgent={onRestartAgent}
                  onStartAgent={onStartAgent}
                  onStartPersona={onStartPersona}
                />
              );
            })}
          </div>

          {unknown.length > 0 ? (
            <CollapsibleAgentGroup
              agents={unknown}
              collapsed={collapsed}
              getAvailability={getAvailability}
              defaultModel={defaultModel}
              groupKey="__unknown__"
              bestiePubkey={bestiePubkey}
              label="Unknown agents"
              restartingAgentPubkey={restartingAgentPubkey}
              startingAgentPubkey={startingAgentPubkey}
              onToggle={toggle}
              onOpenAgentProfile={onOpenAgentProfile}
              onRestartAgent={onRestartAgent}
              onStartAgent={onStartAgent}
            />
          ) : null}
          {ungrouped.length > 0 ? (
            <CollapsibleAgentGroup
              agents={ungrouped}
              collapsed={collapsed}
              getAvailability={getAvailability}
              defaultModel={defaultModel}
              groupKey="__ungrouped__"
              bestiePubkey={bestiePubkey}
              label="Custom agents"
              restartingAgentPubkey={restartingAgentPubkey}
              startingAgentPubkey={startingAgentPubkey}
              onToggle={toggle}
              onOpenAgentProfile={onOpenAgentProfile}
              onRestartAgent={onRestartAgent}
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
  isBestie,
  getAvailability,
  persona,
  restartingAgentPubkey,
  startingAgentPubkey,
  startingPersonaIds,
  onOpenAgentProfile,
  onOpenPersonaProfile,
  onRestartAgent,
  onStartAgent,
  onStartPersona,
}: {
  actions?: (
    effectiveAvatarUrl: string | null,
    isEffectiveAvatarLoading: boolean,
  ) => React.ReactNode;
  agent: ManagedAgent | undefined;
  defaultModel: string;
  isBestie: boolean;
  getAvailability: AgentAvailabilityReader;
  persona: AgentPersona;
  restartingAgentPubkey: string | null;
  startingAgentPubkey: string | null;
  startingPersonaIds: ReadonlySet<string>;
  onOpenAgentProfile: (
    pubkey: string,
    options?: ProfilePanelOpenOptions,
  ) => void;
  onOpenPersonaProfile: (persona: AgentPersona) => void;
  onRestartAgent: (pubkey: string) => void;
  onStartAgent: (pubkey: string) => void;
  onStartPersona: (persona: AgentPersona) => void;
}) {
  const availability = getAvailability(agent?.pubkey);
  const title = persona.displayName;
  // Card face second line: the authored description when one exists;
  // otherwise fall back to the model label as before.
  const subtitle =
    effectiveAgentDescription(persona) ??
    resolveAgentCardModelLabel({
      agent,
      personaModel: persona.model,
      provider: persona.provider,
      defaultModel,
    });
  const isActive = agent ? isManagedAgentActive(agent) : false;
  const profileQuery = useUserProfileQuery(agent?.pubkey);
  const avatarUrl = agent
    ? resolveAgentCardAvatarUrl(profileQuery.data?.avatarUrl, persona.avatarUrl)
    : persona.avatarUrl;
  const friendlyError = agent
    ? friendlyAgentLastError(agent.lastError, agent.lastErrorCode)?.copy
    : null;

  return (
    <AgentIdentityCard
      actions={actions?.(
        avatarUrl,
        isAgentCardAvatarLoading(Boolean(agent), profileQuery.isPending),
      )}
      ariaLabel={`${title} agent profile`}
      avatar={
        agent ? (
          <AgentRuntimeAvatarControl
            activeTestId={`agent-runtime-active-${agent.pubkey}`}
            avatarUrl={avatarUrl}
            errorLabel={friendlyError}
            errorTestId={`agent-runtime-error-${agent.pubkey}`}
            isActive={isActive}
            availability={availability}
            isRestarting={restartingAgentPubkey === agent.pubkey}
            isStarting={startingAgentPubkey === agent.pubkey}
            label={title}
            requiresRestart={agent.needsRestart}
            startTestId={`agent-runtime-start-${agent.pubkey}`}
            onOpenError={() => {
              onOpenAgentProfile(agent.pubkey, { tab: "runtime" });
            }}
            onStart={() =>
              agent.needsRestart
                ? onRestartAgent(agent.pubkey)
                : onStartAgent(agent.pubkey)
            }
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
      footerAccessory={
        agent ? (
          <ProtectedBestieCardBadge agent={agent} isBestie={isBestie} />
        ) : null
      }
      label={title}
      subtitle={subtitle}
      onClick={() => {
        // The card's main click always opens the PERSONA target, never an
        // explicit pubkey. A pubkey target is durable in the panel, so a pick
        // made during the archive-snapshot fail-open window would strand the
        // panel on an archived identity after hydration (Carl's cold-hydration
        // race). A persona target re-resolves every render through the shared
        // archive-aware selector, so it self-corrects to a live sibling — or
        // persona-only mode when every instance is archived. Deliberate
        // instance navigation and the runtime-error affordance keep their
        // explicit-pubkey path via the avatar control below.
        onOpenPersonaProfile(persona);
      }}
      statusBadge={
        agent?.personaOrphaned ? (
          <Badge className="gap-1" variant="warning">
            <AlertTriangle className="h-3 w-3" />
            Configuration missing
          </Badge>
        ) : null
      }
    />
  );
}

function StandaloneAgentCard({
  agent,
  isBestie,
  defaultModel,
  getAvailability,
  restartingAgentPubkey,
  startingAgentPubkey,
  onOpenAgentProfile,
  onRestartAgent,
  onStartAgent,
}: {
  agent: ManagedAgent;
  isBestie: boolean;
  defaultModel: string;
  getAvailability: AgentAvailabilityReader;
  restartingAgentPubkey: string | null;
  startingAgentPubkey: string | null;
  onOpenAgentProfile: (
    pubkey: string,
    options?: ProfilePanelOpenOptions,
  ) => void;
  onRestartAgent: (pubkey: string) => void;
  onStartAgent: (pubkey: string) => void;
}) {
  const availability = getAvailability(agent.pubkey);
  const title = agent.name;
  const profileQuery = useUserProfileQuery(agent.pubkey);
  const friendlyError = friendlyAgentLastError(
    agent.lastError,
    agent.lastErrorCode,
  )?.copy;
  const isActive = isManagedAgentActive(agent);
  const opensRuntimeTab = Boolean(friendlyError && !isActive);

  return (
    <AgentIdentityCard
      ariaLabel={`${title} agent profile`}
      avatar={
        <AgentRuntimeAvatarControl
          activeTestId={`agent-runtime-active-${agent.pubkey}`}
          avatarUrl={profileQuery.data?.avatarUrl}
          errorLabel={friendlyError}
          errorTestId={`agent-runtime-error-${agent.pubkey}`}
          isActive={isActive}
          availability={availability}
          isRestarting={restartingAgentPubkey === agent.pubkey}
          isStarting={startingAgentPubkey === agent.pubkey}
          label={title}
          requiresRestart={agent.needsRestart}
          startTestId={`agent-runtime-start-${agent.pubkey}`}
          onOpenError={() => {
            onOpenAgentProfile(agent.pubkey, { tab: "runtime" });
          }}
          onStart={() =>
            agent.needsRestart
              ? onRestartAgent(agent.pubkey)
              : onStartAgent(agent.pubkey)
          }
        />
      }
      avatarUrl={profileQuery.data?.avatarUrl}
      dataTestId={`managed-agent-${agent.pubkey}`}
      footerAccessory={
        <ProtectedBestieCardBadge agent={agent} isBestie={isBestie} />
      }
      label={title}
      subtitle={
        // Definition-less instance: no authored description exists, so fall
        // back to the model label.
        resolveAgentCardModelLabel({
          agent,
          personaModel: null,
          provider: agent.provider,
          defaultModel,
        })
      }
      onClick={() => {
        onOpenAgentProfile(
          agent.pubkey,
          opensRuntimeTab ? { tab: "runtime" } : undefined,
        );
      }}
      statusBadge={
        agent.personaOrphaned ? (
          <Badge className="gap-1" variant="warning">
            <AlertTriangle className="h-3 w-3" />
            Configuration missing
          </Badge>
        ) : null
      }
    />
  );
}

function LoadingSkeleton() {
  return (
    <div className={IDENTITY_CARD_GRID_CLASS}>
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
  bestiePubkey,
  collapsed,
  defaultModel,
  getAvailability,
  restartingAgentPubkey,
  startingAgentPubkey,
  onToggle,
  onOpenAgentProfile,
  onRestartAgent,
  onStartAgent,
}: {
  groupKey: string;
  label: string;
  agents: ManagedAgent[];
  bestiePubkey: string | null;
  collapsed: ReadonlySet<string>;
  defaultModel: string;
  getAvailability: AgentAvailabilityReader;
  restartingAgentPubkey: string | null;
  startingAgentPubkey: string | null;
  onToggle: (key: string) => void;
  onOpenAgentProfile: (
    pubkey: string,
    options?: ProfilePanelOpenOptions,
  ) => void;
  onRestartAgent: (pubkey: string) => void;
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
        <div className={IDENTITY_CARD_GRID_CLASS}>
          {agents.map((agent) => (
            <StandaloneAgentCard
              agent={agent}
              getAvailability={getAvailability}
              defaultModel={defaultModel}
              isBestie={agent.pubkey.toLowerCase() === bestiePubkey}
              key={agent.pubkey}
              restartingAgentPubkey={restartingAgentPubkey}
              startingAgentPubkey={startingAgentPubkey}
              onOpenAgentProfile={onOpenAgentProfile}
              onRestartAgent={onRestartAgent}
              onStartAgent={onStartAgent}
            />
          ))}
        </div>
      ) : null}
    </div>
  );
}
