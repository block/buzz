import * as React from "react";

import type {
  AcpRuntimeCatalogEntry,
  CreatePersonaInput,
  ManagedAgent,
  UpdatePersonaInput,
} from "@/shared/api/types";
import {
  runLocationForBackend,
  runLocationForRunOn,
} from "../lib/agentAccessWarning";
import { AgentRunLocationProvider } from "./AgentRunLocationContext";
import type { BackendIntent } from "../lib/instanceInputForDefinition";
import type {
  AgentCreateIntent,
  AgentLaunchContext,
} from "./agentCreateIntent";
import type { EditAgentFocusTarget } from "@/features/agents/openEditAgentEvent";
import { AgentInstanceEditDialog } from "./AgentInstanceEditDialog";
import { createPersonaDialogState } from "./personaDialogState";
import { AgentDefinitionDialog } from "./AgentDefinitionDialog";
import type { AgentDefinitionSubmitOptions } from "./agentDefinitionDialogTypes";
import { WhereToRunSection } from "./WhereToRunSection";
import {
  canSubmitWhereToRun,
  emptyWhereToRunDraft,
  resolveBackendIntent,
} from "./whereToRunIntent";
import { useProjectsQuery } from "@/features/projects/hooks";
import { useCommunities } from "@/features/communities/useCommunities";
import { useIdentityQuery } from "@/shared/api/hooks";
import { durableProjectAddress } from "@/shared/api/agentProjectTypes";
import {
  AgentProjectAccessSection,
  emptyAgentProjectAccessDraft,
  type AgentProjectAccessReadiness,
} from "./AgentProjectAccessSection";

type AgentDialogCreateProps = {
  mode: "definition";
  initialValues?: CreatePersonaInput | null;
  onOpenChange: (open: boolean) => void;
  definitionError: Error | null;
  isDefinitionPending: boolean;
  runtimes: AcpRuntimeCatalogEntry[];
  runtimeCatalogStatus: "loading" | "ready" | "error";
  onSubmitDefinition: (
    input: CreatePersonaInput | UpdatePersonaInput,
    intent: AgentCreateIntent,
    backendIntent: BackendIntent | null,
    launchContext?: AgentLaunchContext,
  ) => Promise<boolean>;
};

type AgentDialogInstanceEditProps = {
  mode: "instance-edit";
  agent: ManagedAgent;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onUpdated?: (agent: ManagedAgent) => void;
  initialFocus?: EditAgentFocusTarget;
  /**
   * Called when the user clicks "Edit avatar" inside the instance-edit dialog.
   * Caller (UserProfilePanel) is responsible for closing this dialog and
   * opening the definition-edit dialog. Only passed when the linked definition
   * is editable (non-built-in, resolved).
   */
  onEditLinkedPersona?: () => void;
};

type AgentDialogDefinitionEditProps = {
  mode: "definition-edit";
  open: boolean;
  title: string;
  description: string;
  submitLabel: string;
  initialValues: CreatePersonaInput | UpdatePersonaInput | null;
  error: Error | null;
  isPending: boolean;
  runtimes: AcpRuntimeCatalogEntry[];
  runtimeCatalogStatus?: "loading" | "ready" | "error";
  onOpenChange: (open: boolean) => void;
  onSubmit: (
    input: CreatePersonaInput | UpdatePersonaInput,
    options: AgentDefinitionSubmitOptions,
  ) => Promise<unknown>;
  publishCatalogUpdatesOnSave?: boolean;
};

type AgentDialogProps =
  | AgentDialogCreateProps
  | AgentDialogInstanceEditProps
  | AgentDialogDefinitionEditProps;

/**
 * Unified entry point (Phase 1B.2/1B.3b/1B.3c): routes an intent to the form
 * that owns it. The definition family renders AgentDefinitionDialog — create
 * mode always starts the agent and includes a WhereToRunSection;
 * definition-edit passes the caller's PersonaDialogState-derived props
 * through unchanged (edit/duplicate/import). instance-edit renders
 * AgentInstanceEditDialog (persistent mount + `open` toggle — its reset
 * lifecycle is keyed on [open, agent.pubkey]).
 */
export function AgentDialog(props: AgentDialogProps) {
  if (props.mode === "instance-edit") {
    return (
      // A running instance knows its own backend, so the respond-to warning can
      // name the machine it will actually run on.
      <AgentRunLocationProvider
        runLocation={runLocationForBackend(props.agent.backend)}
      >
        <AgentInstanceEditDialog
          agent={props.agent}
          onEditLinkedPersona={props.onEditLinkedPersona}
          onOpenChange={props.onOpenChange}
          onUpdated={props.onUpdated}
          open={props.open}
          initialFocus={props.initialFocus}
        />
      </AgentRunLocationProvider>
    );
  }
  if (props.mode === "definition-edit") {
    // A definition has no instance and no run draft, so the run location stays
    // unknown and the warning uses its local-wording fallback.
    const { mode: _mode, ...definitionProps } = props;
    return <AgentDefinitionDialog {...definitionProps} />;
  }
  return <AgentCreateDialogRouter {...props} />;
}

function AgentCreateDialogRouter({
  initialValues: providedInitialValues,
  onOpenChange,
  definitionError,
  isDefinitionPending,
  runtimes,
  runtimeCatalogStatus,
  onSubmitDefinition,
}: AgentDialogCreateProps) {
  const [runDraft, setRunDraft] = React.useState(emptyWhereToRunDraft);
  const [projectAccessDraft, setProjectAccessDraft] = React.useState(
    emptyAgentProjectAccessDraft,
  );
  const [projectAccessReadiness, setProjectAccessReadiness] =
    React.useState<AgentProjectAccessReadiness>({
      ready: false,
      reason: "Choose a Project for this agent.",
    });
  const projectsQuery = useProjectsQuery();
  const { activeCommunity } = useCommunities();
  const identityQuery = useIdentityQuery();
  const handleProjectAccessReadinessChange = React.useCallback(
    (readiness: AgentProjectAccessReadiness) => {
      setProjectAccessReadiness((current) =>
        current.ready === readiness.ready && current.reason === readiness.reason
          ? current
          : readiness,
      );
    },
    [],
  );
  const initialValues = React.useMemo(
    () => providedInitialValues ?? createPersonaDialogState().initialValues,
    [providedInitialValues],
  );

  const copy = createPersonaDialogState();

  return (
    // The create flow is the one surface that knows where the agent will run,
    // because it owns the "Run on" draft.
    <AgentRunLocationProvider runLocation={runLocationForRunOn(runDraft.runOn)}>
      <AgentDefinitionDialog
        createRunSection={(toolRequirements) => (
          <>
            <AgentProjectAccessSection
              disabled={isDefinitionPending}
              draft={projectAccessDraft}
              onDraftChange={setProjectAccessDraft}
              onReadinessChange={handleProjectAccessReadinessChange}
              operatorPubkey={identityQuery.data?.pubkey ?? null}
              projects={projectsQuery.data ?? []}
              projectsLoading={projectsQuery.isPending}
              relayUrl={activeCommunity?.relayUrl ?? null}
              toolRequirements={toolRequirements}
            />
            <WhereToRunSection
              draft={runDraft}
              isPending={isDefinitionPending}
              onDraftChange={setRunDraft}
            />
            {runDraft.runOn !== "local" && toolRequirements.length > 0 ? (
              <p className="text-xs text-warning">
                Project connections currently run on this computer. Choose This
                computer to launch with connected tools.
              </p>
            ) : null}
          </>
        )}
        createSubmitBlocked={(toolRequirements) =>
          !canSubmitWhereToRun(runDraft) ||
          !projectAccessReadiness.ready ||
          (toolRequirements.length > 0 && runDraft.runOn !== "local")
        }
        createSubmitBlockReason={(toolRequirements) =>
          !canSubmitWhereToRun(runDraft)
            ? "Complete the provider setup."
            : !projectAccessReadiness.ready
              ? projectAccessReadiness.reason
              : toolRequirements.length > 0 && runDraft.runOn !== "local"
                ? "Choose This computer to use Project connections."
                : null
        }
        description={copy.description}
        error={definitionError}
        initialValues={initialValues}
        isPending={isDefinitionPending}
        onOpenChange={onOpenChange}
        onSubmit={async (input) => {
          const selectedProject = (projectsQuery.data ?? []).find(
            (project) => project.id === projectAccessDraft.projectId,
          );
          if (
            !selectedProject?.projectChannelId ||
            !activeCommunity?.relayUrl ||
            !identityQuery.data?.pubkey
          ) {
            return;
          }
          const submitted = await onSubmitDefinition(
            input,
            "definition_start",
            resolveBackendIntent(runDraft),
            {
              projectScope: {
                relayUrl: activeCommunity.relayUrl,
                operatorPubkey: identityQuery.data.pubkey,
                projectAddress: durableProjectAddress(selectedProject),
                channelId: selectedProject.projectChannelId,
              },
              connectionBindings: projectAccessDraft.connectionBindings,
            },
          );
          if (submitted) {
            onOpenChange(false);
          }
        }}
        open
        runtimes={runtimes}
        runtimeCatalogStatus={runtimeCatalogStatus}
        submitLabel={copy.submitLabel}
        title={copy.title}
      />
    </AgentRunLocationProvider>
  );
}
