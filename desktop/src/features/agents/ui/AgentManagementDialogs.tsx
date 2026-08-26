import { useAgentManagement } from "@/features/agents/useAgentManagement";
import { AgentCardDialogs } from "./AgentCardViewerDialog";
import { AgentDialog } from "./AgentDialog";
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import { Button } from "@/shared/ui/button";

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
          submitLabel="Save changes"
          title="Edit agent"
        />
      ) : null}
      {management.request?.action === "adopt" ? (
        <AlertDialog
          onOpenChange={(open) => {
            if (!open) management.dismiss();
          }}
          open
        >
          <AlertDialogContent data-testid="register-existing-agent-review">
            <AlertDialogHeader>
              <AlertDialogTitle>Register existing agent</AlertDialogTitle>
              <AlertDialogDescription>
                Buzz will publish one directory entry for this identity. It will
                not import a key or start, stop, or configure the agent.
              </AlertDialogDescription>
            </AlertDialogHeader>
            <dl className="space-y-3 text-sm">
              <div>
                <dt className="text-muted-foreground">Directory name</dt>
                <dd>{management.request.request.displayName}</dd>
              </div>
              <div>
                <dt className="text-muted-foreground">Signed profile</dt>
                <dd>
                  {management.adoptProfile?.displayName ?? "No display name"}
                </dd>
              </div>
              <div>
                <dt className="text-muted-foreground">Public key</dt>
                <dd className="break-all font-mono text-xs">
                  {management.request.request.agentPubkey}
                </dd>
              </div>
              <div>
                <dt className="text-muted-foreground">Verified owner</dt>
                <dd className="break-all font-mono text-xs">
                  {management.verifiedAdoptOwner ?? "Verifying…"}
                </dd>
              </div>
              <div>
                <dt className="text-muted-foreground">Who can instruct it</dt>
                <dd>Owner only</dd>
              </div>
            </dl>
            {management.adoptPreviewError ? (
              <p className="text-sm text-destructive" role="alert">
                {management.adoptPreviewError}
              </p>
            ) : null}
            <AlertDialogFooter>
              <AlertDialogCancel asChild>
                <Button type="button" variant="outline">
                  Cancel
                </Button>
              </AlertDialogCancel>
              <Button
                disabled={
                  management.isPending ||
                  management.isAdoptPreviewPending ||
                  !management.verifiedAdoptOwner ||
                  Boolean(management.adoptPreviewError)
                }
                onClick={() => void management.submitAdopt()}
                type="button"
              >
                {management.isPending ? "Registering…" : "Register identity"}
              </Button>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      ) : null}
      <AgentCardDialogs />
    </>
  );
}
