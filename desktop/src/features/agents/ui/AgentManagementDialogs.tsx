import { useAgentManagement } from "@/features/agents/useAgentManagement";
import { AgentDialog } from "./AgentDialog";
import { SecretRevealDialog } from "./SecretRevealDialog";

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
          runtimesLoading={management.runtimesLoading}
        />
      ) : null}
      {management.createdAgent ? (
        <SecretRevealDialog
          attachmentFailure={management.attachmentFailure}
          created={management.createdAgent}
          isRetryingAttachment={management.isRetryingAttachment}
          onOpenChange={(open) => {
            if (!open) management.dismissCreatedAgent();
          }}
          onRetryAttachment={() => {
            void management.retryAttachment();
          }}
        />
      ) : null}
      {management.request?.action === "update" ? (
        <AgentDialog
          description=""
          error={management.editError ? new Error(management.editError) : null}
          initialValues={management.editInitialValues}
          isPending={management.isPending}
          mode="definition-edit"
          onOpenChange={(open) => {
            if (!open) management.dismiss();
          }}
          onSubmit={management.submitUpdate}
          open
          runtimes={management.runtimes}
          runtimesLoading={management.runtimesLoading}
          submitLabel="Save changes"
          title="Edit agent"
        />
      ) : null}
    </>
  );
}
