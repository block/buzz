import { useTranslation } from "@/i18n";
import { useAgentManagement } from "@/features/agents/useAgentManagement";
import { ProjectChannelRequestDialog } from "@/features/projects/ui/ProjectChannelRequestDialog";
import { AgentCardDialogs } from "./AgentCardViewerDialog";
import { AgentDialog } from "./AgentDialog";

/** Global review surfaces opened by owned agents through the Buzz harness. */
export function AgentManagementDialogs() {
  const { t } = useTranslation();
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
          runtimeCatalogStatus={management.runtimeCatalogStatus}
          submitLabel={t("agents.persona-dialog.save-changes")}
          title={t("agents.persona-dialog.edit-agent")}
        />
      ) : null}
      <ProjectChannelRequestDialog />
      <AgentCardDialogs />
    </>
  );
}
