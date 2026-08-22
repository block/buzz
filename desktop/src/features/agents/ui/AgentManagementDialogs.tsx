import { useAgentManagement } from "@/features/agents/useAgentManagement";
import { AgentCardDialogs } from "./AgentCardViewerDialog";
import { AgentDialog } from "./AgentDialog";
import { AgentEditDialog } from "./AgentEditDialog";
import { toast } from "sonner";
import * as React from "react";

/** Global review surfaces opened by owned agents through the Buzz harness. */
export function AgentManagementDialogs() {
  const management = useAgentManagement();

  // R6 error handling: when an update request arrives but the persona cannot
  // be found or is ambiguous, toast the error and dismiss. This avoids showing
  // a definition-edit dialog for an unresolvable update.
  React.useEffect(() => {
    if (
      management.request?.action === "update" &&
      !management.currentPersona &&
      management.editError
    ) {
      toast.error(management.editError);
      management.dismiss();
    }
  }, [
    management.request?.action,
    management.currentPersona,
    management.editError,
    management.dismiss,
  ]);

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
      {/* R6: agent-origin owner-review update → merged surface.
          assertAgentCanActFromOrigin guard preserved verbatim via onValidate. */}
      {management.request?.action === "update" && management.currentPersona ? (
        <AgentEditDialog
          ctx={{
            kind: "definition-only",
            definition: management.currentPersona,
          }}
          open
          onOpenChange={(open) => {
            if (!open) management.dismiss();
          }}
          onValidate={() => management.verifyOriginPermission()}
          initialValueOverrides={management.reviewOverrides}
        />
      ) : null}
      <AgentCardDialogs />
    </>
  );
}
