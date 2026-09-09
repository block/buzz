import * as React from "react";

import type {
  AcpRuntimeCatalogEntry,
  CreatePersonaInput,
  UpdatePersonaInput,
} from "@/shared/api/types";
import { runLocationForRunOn } from "../lib/agentAccessWarning";
import { AgentRunLocationProvider } from "./AgentRunLocationContext";
import type { BackendIntent } from "../lib/instanceInputForDefinition";
import type { AgentCreateIntent } from "./agentCreateIntent";
import { createPersonaDialogState } from "./personaDialogState";
import {
  AgentDefinitionDialog,
  type AgentDefinitionSubmitOptions,
} from "./AgentDefinitionDialog";
import { WhereToRunSection } from "./WhereToRunSection";
import {
  canSubmitWhereToRun,
  emptyWhereToRunDraft,
  resolveBackendIntent,
} from "./whereToRunIntent";

type AgentDialogCreateProps = {
  mode: "definition";
  embedded?: boolean;
  submitLabel?: string;
  initialValues?: CreatePersonaInput | null;
  onDirtyChange?: (dirty: boolean) => void;
  onOpenChange: (open: boolean) => void;
  definitionError: Error | null;
  isDefinitionPending: boolean;
  runtimes: AcpRuntimeCatalogEntry[];
  runtimeCatalogStatus: "loading" | "ready" | "error";
  onSubmitDefinition: (
    input: CreatePersonaInput | UpdatePersonaInput,
    intent: AgentCreateIntent,
    backendIntent: BackendIntent | null,
  ) => Promise<boolean>;
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

type AgentDialogProps = AgentDialogCreateProps | AgentDialogDefinitionEditProps;

/**
 * Unified entry point (Phase 1B.2/1B.3b/1B.3c): routes an intent to the form
 * that owns it. The definition family renders AgentDefinitionDialog — create
 * mode always starts the agent and includes a WhereToRunSection;
 * definition-edit passes the caller's PersonaDialogState-derived props
 * through unchanged (duplicate/import). Edit routes use AgentEditDialog
 * → AgentEditMergedDialog (one merged surface for all R1–R7 contexts).
 */
export function AgentDialog(props: AgentDialogProps) {
  if (props.mode === "definition-edit") {
    // A definition has no instance and no run draft, so the run location stays
    // unknown and the warning uses its local-wording fallback.
    const { mode: _mode, ...definitionProps } = props;
    return <AgentDefinitionDialog {...definitionProps} />;
  }
  return <AgentCreateDialogRouter {...props} />;
}

function AgentCreateDialogRouter({
  embedded,
  initialValues: providedInitialValues,
  onOpenChange,
  definitionError,
  isDefinitionPending,
  runtimes,
  runtimeCatalogStatus,
  submitLabel,
  onDirtyChange,
  onSubmitDefinition,
}: AgentDialogCreateProps) {
  const [runDraft, setRunDraft] = React.useState(emptyWhereToRunDraft);
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
        createRunSection={
          <WhereToRunSection
            draft={runDraft}
            isPending={isDefinitionPending}
            onDraftChange={(nextDraft) => {
              setRunDraft(nextDraft);
              onDirtyChange?.(true);
            }}
          />
        }
        createSubmitBlocked={!canSubmitWhereToRun(runDraft)}
        description={copy.description}
        embedded={embedded}
        error={definitionError}
        initialValues={initialValues}
        isPending={isDefinitionPending}
        onDirtyChange={onDirtyChange}
        onOpenChange={onOpenChange}
        onSubmit={async (input) => {
          const submitted = await onSubmitDefinition(
            input,
            "definition_start",
            resolveBackendIntent(runDraft),
          );
          if (submitted) {
            onDirtyChange?.(false);
            onOpenChange(false);
          }
        }}
        open
        runtimes={runtimes}
        runtimeCatalogStatus={runtimeCatalogStatus}
        submitLabel={submitLabel ?? copy.submitLabel}
        title={copy.title}
      />
    </AgentRunLocationProvider>
  );
}
