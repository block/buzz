import * as React from "react";
import { toast } from "sonner";

import { useCommunities } from "@/features/communities/useCommunities";
import { useProjectsQuery } from "@/features/projects/hooks";
import { useActiveAgentTurns } from "@/features/agents/activeAgentTurnsStore";
import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import { useIdentityQuery } from "@/shared/api/hooks";
import { durableProjectAddress } from "@/shared/api/agentProjectTypes";
import type { AgentProjectScope, ManagedAgent } from "@/shared/api/types";
import { useManagedAgentRuntimeAction } from "../managedAgentRuntimeHooks";
import {
  AgentProjectAccessSection,
  emptyAgentProjectAccessDraft,
  type AgentProjectAccessReadiness,
} from "./AgentProjectAccessSection";

function recordsEqual(
  left: Record<string, string>,
  right: Record<string, string>,
) {
  const entries = Object.entries(left);
  return (
    entries.length === Object.keys(right).length &&
    entries.every(([key, value]) => right[key] === value)
  );
}

function projectScopesEqual(
  left: AgentProjectScope | null,
  right: AgentProjectScope | null,
) {
  return (
    left?.relayUrl === right?.relayUrl &&
    left?.operatorPubkey === right?.operatorPubkey &&
    left?.projectAddress === right?.projectAddress &&
    left?.channelId === right?.channelId
  );
}

export function useAgentConnectionBindingsDraft({
  agent,
  open,
  updatePending,
}: {
  agent: ManagedAgent;
  open: boolean;
  updatePending: boolean;
}) {
  const runtimeActionMutation = useManagedAgentRuntimeAction();
  const activeTurns = useActiveAgentTurns(agent.pubkey);
  const projectsQuery = useProjectsQuery();
  const { activeCommunity } = useCommunities();
  const identityQuery = useIdentityQuery();
  const [draft, setDraft] = React.useState(emptyAgentProjectAccessDraft);
  const [readiness, setReadiness] = React.useState<AgentProjectAccessReadiness>(
    {
      ready: !agent.toolRequirements.some(
        (requirement) => requirement.required,
      ),
      reason: null,
    },
  );
  const [projectTouched, setProjectTouched] = React.useState(false);
  const seededAgentRef = React.useRef<string | null>(null);
  const projects = React.useMemo(
    () => projectsQuery.data ?? [],
    [projectsQuery.data],
  );

  React.useEffect(() => {
    if (!open) {
      seededAgentRef.current = null;
      return;
    }
    if (seededAgentRef.current === agent.pubkey) return;
    if (agent.projectScope && projectsQuery.isPending) return;

    const projectId =
      projects.find(
        (project) =>
          durableProjectAddress(project) === agent.projectScope?.projectAddress,
      )?.id ?? "";
    setDraft({
      projectId,
      connectionBindings: agent.connectionBindings,
    });
    setReadiness({
      ready: !agent.toolRequirements.some(
        (requirement) => requirement.required,
      ),
      reason: null,
    });
    setProjectTouched(false);
    seededAgentRef.current = agent.pubkey;
  }, [
    agent.connectionBindings,
    agent.projectScope,
    agent.pubkey,
    agent.toolRequirements,
    open,
    projects,
    projectsQuery.isPending,
  ]);

  const handleReadinessChange = React.useCallback(
    (nextReadiness: AgentProjectAccessReadiness) =>
      setReadiness((current) =>
        current.ready === nextReadiness.ready &&
        current.reason === nextReadiness.reason
          ? current
          : nextReadiness,
      ),
    [],
  );
  const handleDraftChange = React.useCallback(
    (nextDraft: typeof draft) => {
      if (nextDraft.projectId !== draft.projectId) {
        setProjectTouched(true);
      }
      setDraft(nextDraft);
    },
    [draft.projectId],
  );

  const selectedProject =
    projects.find((project) => project.id === draft.projectId) ?? null;
  const selectedProjectScope: AgentProjectScope | null =
    selectedProject?.projectChannelId &&
    activeCommunity?.relayUrl &&
    identityQuery.data?.pubkey
      ? {
          relayUrl: activeCommunity.relayUrl,
          operatorPubkey: identityQuery.data.pubkey,
          projectAddress: durableProjectAddress(selectedProject),
          channelId: selectedProject.projectChannelId,
        }
      : null;
  const projectScopeUpdate =
    !projectTouched ||
    projectScopesEqual(selectedProjectScope, agent.projectScope)
      ? undefined
      : selectedProjectScope;
  const update = recordsEqual(
    draft.connectionBindings,
    agent.connectionBindings,
  )
    ? undefined
    : draft.connectionBindings;
  const hasUpdate = update !== undefined || projectScopeUpdate !== undefined;
  const shouldRestart = hasUpdate && isManagedAgentActive(agent);
  const restartAfterCurrentTask = shouldRestart && activeTurns.length > 0;
  const isSaving = updatePending || runtimeActionMutation.isPending;

  async function restartAfterSave(
    savedAgent: ManagedAgent,
    autoRestartEnabled: boolean,
  ) {
    if (restartAfterCurrentTask) {
      toast.success(
        autoRestartEnabled
          ? `${savedAgent.name}'s changes are saved. Buzz will restart it after its current task.`
          : `${savedAgent.name}'s changes are saved. Restart it when the current task is finished.`,
      );
      return;
    }
    const relayUrl =
      savedAgent.projectScope?.relayUrl ??
      agent.projectScope?.relayUrl ??
      activeCommunity?.relayUrl;
    if (!shouldRestart || !relayUrl) return;
    try {
      await runtimeActionMutation.mutateAsync({
        action: "restart",
        pubkey: savedAgent.pubkey,
        relayUrl,
      });
      toast.success(`${savedAgent.name} restarted with its changes.`);
    } catch (error) {
      const message =
        error instanceof Error ? error.message : "The restart failed.";
      toast.error(
        `${savedAgent.name} was saved, but could not restart: ${message}`,
      );
    }
  }

  return {
    isSaving,
    projectScopeUpdate,
    restartAfterSave,
    saveLabel: runtimeActionMutation.isPending
      ? "Restarting..."
      : updatePending
        ? "Saving..."
        : shouldRestart
          ? restartAfterCurrentTask
            ? "Save changes"
            : "Save and restart"
          : "Save changes",
    valid: !hasUpdate || readiness.ready,
    update,
    section: (
      <>
        <AgentProjectAccessSection
          allowUnassigned={
            !agent.toolRequirements.some((requirement) => requirement.required)
          }
          description="Choose where this agent works and which Project connections it can use. Existing messages stay where they are."
          draft={draft}
          disabled={isSaving}
          idPrefix="edit-agent"
          onDraftChange={handleDraftChange}
          onReadinessChange={handleReadinessChange}
          operatorPubkey={identityQuery.data?.pubkey ?? null}
          projects={projects}
          projectsLoading={projectsQuery.isPending}
          relayUrl={activeCommunity?.relayUrl ?? null}
          toolRequirements={agent.toolRequirements}
        />
        {!readiness.ready && readiness.reason ? (
          <p className="text-xs text-muted-foreground" aria-live="polite">
            {readiness.reason}
          </p>
        ) : null}
      </>
    ),
  };
}
