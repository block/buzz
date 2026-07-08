import * as React from "react";

import type {
  AcpRuntimeCatalogEntry,
  CreateManagedAgentResponse,
  CreatePersonaInput,
  UpdatePersonaInput,
} from "@/shared/api/types";
import { Switch } from "@/shared/ui/switch";
import {
  definitionCreateDialogState,
  intentForStartToggle,
  type AgentCreateIntent,
} from "./agentCreateIntent";
import { CreateAgentDialog } from "./CreateAgentDialog";
import { createPersonaDialogState } from "./personaDialogState";
import { AgentDefinitionDialog } from "./AgentDefinitionDialog";

export type AgentDialogMode = "definition" | "instance";

type AgentDialogProps = {
  mode: AgentDialogMode;
  onOpenChange: (open: boolean) => void;
  definitionError: Error | null;
  isDefinitionPending: boolean;
  runtimes: AcpRuntimeCatalogEntry[];
  runtimesLoading: boolean;
  onSubmitDefinition: (
    input: CreatePersonaInput | UpdatePersonaInput,
    intent: AgentCreateIntent,
  ) => Promise<boolean>;
  onInstanceCreated: (result: CreateManagedAgentResponse) => void;
};

/**
 * Unified create entry point (Phase 1B.2): routes a create intent to the
 * form that owns it. The definition family renders AgentDefinitionDialog in
 * create mode with a "start after create" toggle; the standalone-instance
 * intent renders CreateAgentDialog unchanged. Physical consolidation of the
 * two forms is Phase 1B.3.
 */
export function AgentDialog({
  mode,
  onOpenChange,
  definitionError,
  isDefinitionPending,
  runtimes,
  runtimesLoading,
  onSubmitDefinition,
  onInstanceCreated,
}: AgentDialogProps) {
  const [startAfterCreate, setStartAfterCreate] = React.useState(true);
  // Stable identity across toggle flips — AgentDefinitionDialog re-initializes its
  // fields whenever `initialValues` changes.
  const initialValues = React.useMemo(
    () => createPersonaDialogState().initialValues,
    [],
  );

  if (mode === "instance") {
    return (
      <CreateAgentDialog
        onCreated={onInstanceCreated}
        onOpenChange={onOpenChange}
        open
      />
    );
  }

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
          Start agent after create
        </label>
      }
      description={copy.description}
      error={definitionError}
      initialValues={initialValues}
      isPending={isDefinitionPending}
      onOpenChange={onOpenChange}
      onSubmit={async (input) => {
        const submitted = await onSubmitDefinition(
          input,
          intentForStartToggle(startAfterCreate),
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
