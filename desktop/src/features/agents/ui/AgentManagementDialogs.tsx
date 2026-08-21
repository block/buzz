import * as React from "react";

import { useAgentManagement } from "@/features/agents/useAgentManagement";
import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import { AgentCardDialogs } from "./AgentCardViewerDialog";
import { AgentDialog } from "./AgentDialog";
import { RunOnSummarySection } from "./RunOnSummarySection";
import { WhereToRunSection } from "./WhereToRunSection";
import {
  canSubmitWhereToRun,
  emptyWhereToRunDraft,
  resolveBackendIntent,
} from "./whereToRunIntent";

/** Global review surfaces opened by owned agents through the Buzz harness. */
export function AgentManagementDialogs() {
  const management = useAgentManagement();

  return (
    <>
      {management.request?.action === "create" ? (
        <AgentDialog
          definitionError={
            management.error ? new Error(management.error) : null
          }
          initialValues={management.createInitialValues}
          isDefinitionPending={management.isPending}
          mode="definition"
          onOpenChange={(open) => {
            if (!open) management.dismiss();
          }}
          onSubmitDefinition={management.submitCreate}
          runtimes={management.runtimes}
          runtimeCatalogStatus={management.runtimeCatalogStatus}
        />
      ) : null}
      {management.request?.action === "update" ? (
        <AgentManagementUpdateDialog management={management} />
      ) : null}
      <AgentCardDialogs />
    </>
  );
}

function AgentManagementUpdateDialog({
  management,
}: {
  management: ReturnType<typeof useAgentManagement>;
}) {
  const [runDraft, setRunDraft] = React.useState(emptyWhereToRunDraft);

  const managedAgent = management.currentManagedAgent;
  const canMigrateBackend = Boolean(
    managedAgent?.backend.type === "local" &&
      !isManagedAgentActive(managedAgent),
  );
  const backendIntent = canMigrateBackend
    ? resolveBackendIntent(runDraft)
    : null;
  const editRunSection = managedAgent ? (
    canMigrateBackend ? (
      <div className="space-y-2" data-testid="agent-management-run-on">
        <WhereToRunSection
          draft={runDraft}
          isPending={management.isPending}
          onDraftChange={setRunDraft}
        />
        {backendIntent ? (
          <p className="text-xs text-muted-foreground">
            Saving preserves this agent&apos;s identity and settings, but leaves
            it stopped until you explicitly deploy it.
          </p>
        ) : null}
      </div>
    ) : (
      <RunOnSummarySection
        backend={managedAgent.backend}
        migrationBlockedReason={
          managedAgent.backend.type === "local"
            ? "Stop this agent before changing where it runs."
            : undefined
        }
      />
    )
  ) : null;

  return (
    <AgentDialog
      description=""
      error={management.editError ? new Error(management.editError) : null}
      editRunSection={editRunSection}
      editSubmitBlocked={canMigrateBackend && !canSubmitWhereToRun(runDraft)}
      initialValues={management.editInitialValues}
      isPending={management.isPending}
      mode="definition-edit"
      onOpenChange={(open) => {
        if (!open) management.dismiss();
      }}
      onSubmit={(input) => management.submitUpdate(input, backendIntent)}
      open
      runtimes={management.runtimes}
      runtimeCatalogStatus={management.runtimeCatalogStatus}
      submitLabel="Save changes"
      title="Edit agent"
    />
  );
}
