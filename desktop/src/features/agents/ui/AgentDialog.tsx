import * as React from "react";

import type {
  AcpRuntimeCatalogEntry,
  CreatePersonaInput,
  ManagedAgent,
  UpdatePersonaInput,
} from "@/shared/api/types";
import { Switch } from "@/shared/ui/switch";
import type { BackendIntent } from "../lib/instanceInputForDefinition";
import {
  definitionCreateDialogState,
  intentForStartToggle,
  type AgentCreateIntent,
} from "./agentCreateIntent";
import type { EditAgentFocusTarget } from "@/features/agents/openEditAgentEvent";
import { AgentInstanceEditDialog } from "./AgentInstanceEditDialog";
import { createPersonaDialogState } from "./personaDialogState";
import { AgentDefinitionDialog } from "./AgentDefinitionDialog";
import { WhereToRunSection } from "./WhereToRunSection";
import {
  canSubmitWhereToRun,
  emptyWhereToRunDraft,
  resolveBackendIntent,
} from "./whereToRunIntent";

type AgentDialogCreateProps = {
  mode: "definition";
  onOpenChange: (open: boolean) => void;
  definitionError: Error | null;
  isDefinitionPending: boolean;
  runtimes: AcpRuntimeCatalogEntry[];
  runtimesLoading: boolean;
  onSubmitDefinition: (
    input: CreatePersonaInput | UpdatePersonaInput,
    intent: AgentCreateIntent,
    backendIntent: BackendIntent | null,
  ) => Promise<boolean>;
};

type AgentDialogInstanceEditProps = {
  mode: "instance-edit";
  agent: ManagedAgent;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onUpdated?: (agent: ManagedAgent) => void;
  initialFocus?: EditAgentFocusTarget;
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
  isImportPending?: boolean;
  runtimes: AcpRuntimeCatalogEntry[];
  runtimesLoading?: boolean;
  onOpenChange: (open: boolean) => void;
  onSubmit: (
    input: CreatePersonaInput | UpdatePersonaInput,
  ) => Promise<unknown>;
  onImportUpdateFile?: (
    personaId: string,
    fileBytes: number[],
    fileName: string,
  ) => Promise<void>;
};

type AgentDialogProps =
  | AgentDialogCreateProps
  | AgentDialogInstanceEditProps
  | AgentDialogDefinitionEditProps;

/**
 * Unified entry point (Phase 1B.2/1B.3b/1B.3c): routes an intent to the form
 * that owns it. The definition family renders AgentDefinitionDialog — create
 * mode adds a "start after create" toggle, definition-edit passes the caller's
 * PersonaDialogState-derived props through unchanged (edit/duplicate/import).
 * instance-edit renders AgentInstanceEditDialog (persistent mount + `open`
 * toggle — its reset lifecycle is keyed on [open, agent.pubkey]).
 */
export function AgentDialog(props: AgentDialogProps) {
  if (props.mode === "instance-edit") {
    return (
      <AgentInstanceEditDialog
        agent={props.agent}
        onOpenChange={props.onOpenChange}
        onUpdated={props.onUpdated}
        open={props.open}
        initialFocus={props.initialFocus}
      />
    );
  }
  if (props.mode === "definition-edit") {
    const { mode: _mode, ...definitionProps } = props;
    return <AgentDefinitionDialog {...definitionProps} />;
  }
  return <AgentCreateDialogRouter {...props} />;
}

function AgentCreateDialogRouter({
  onOpenChange,
  definitionError,
  isDefinitionPending,
  runtimes,
  runtimesLoading,
  onSubmitDefinition,
}: AgentDialogCreateProps) {
  const [startAfterCreate, setStartAfterCreate] = React.useState(true);
  const [runDraft, setRunDraft] = React.useState(emptyWhereToRunDraft);
  // Stable identity across toggle flips — AgentDefinitionDialog re-initializes its
  // fields whenever `initialValues` changes.
  const initialValues = React.useMemo(
    () => createPersonaDialogState().initialValues,
    [],
  );

  const copy = definitionCreateDialogState(startAfterCreate);

  return (
    <AgentDefinitionDialog
      createFooterSlot={
        <label
          className="flex cursor-pointer items-center gap-2 text-sm text-muted-foreground"
          htmlFor="agent-dialog-start-toggle"
        >
          <Switch
            checked={startAfterCreate}
            data-testid="agent-dialog-start-toggle"
            disabled={isDefinitionPending}
            id="agent-dialog-start-toggle"
            onCheckedChange={setStartAfterCreate}
          />
          Start agent after creation
        </label>
      }
      createRunSection={
        // "Where to run" is instance state: with the start toggle off no
        // instance exists, so the section disappears instead of dangling.
        startAfterCreate ? (
          <WhereToRunSection
            draft={runDraft}
            isPending={isDefinitionPending}
            onDraftChange={setRunDraft}
          />
        ) : null
      }
      createSubmitBlocked={!canSubmitWhereToRun(runDraft, startAfterCreate)}
      description={copy.description}
      error={definitionError}
      initialValues={initialValues}
      isPending={isDefinitionPending}
      onOpenChange={onOpenChange}
      onSubmit={async (input) => {
        const submitted = await onSubmitDefinition(
          input,
          intentForStartToggle(startAfterCreate),
          resolveBackendIntent(runDraft, startAfterCreate),
        );
        if (submitted) {
          onOpenChange(false);
        }
      }}
      open
      runtimes={runtimes}
      runtimesLoading={runtimesLoading}
      submitLabel={copy.submitLabel}
      title={copy.title}
    />
  );
}
