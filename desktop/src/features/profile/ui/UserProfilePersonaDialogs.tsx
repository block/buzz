import type {
  AcpRuntimeCatalogEntry,
  AgentPersona,
  CreatePersonaInput,
  UpdatePersonaInput,
} from "@/shared/api/types";
import type { PersonaRemoteCascadeInstance } from "@/features/agents/lib/personaCascade";
import { PersonaDeleteDialog } from "@/features/agents/ui/PersonaDeleteDialog";
import { AgentDialog } from "@/features/agents/ui/AgentDialog";
import type { PersonaDialogState } from "@/features/agents/ui/personaDialogState";

export function UserProfilePersonaDialogs({
  createError,
  editsProviderRecord = false,
  instanceCount,
  isPending,
  personaDialogState,
  personaToDelete,
  remoteInstances = [],
  runtimes,
  runtimesLoading,
  updateError,
  onCloseDelete,
  onCloseDialog,
  onConfirmDelete,
  onSubmit,
}: {
  createError: Error | null;
  /**
   * The persona in this panel backs a provider record, so its blank runtime is
   * the host's harness being deliberately withheld — never a local default to
   * seed. See `createRuntimeSeedAction`.
   */
  editsProviderRecord?: boolean;
  /** Number of managed-agent instances backed by the persona being deleted. */
  instanceCount: number;
  isPending: boolean;
  personaDialogState: PersonaDialogState | null;
  personaToDelete: AgentPersona | null;
  /** Cascade instances whose remote deployment survives the delete. */
  remoteInstances?: readonly PersonaRemoteCascadeInstance[];
  runtimes: AcpRuntimeCatalogEntry[];
  runtimesLoading: boolean;
  updateError: Error | null;
  onCloseDelete: () => void;
  onCloseDialog: () => void;
  onConfirmDelete: (persona: AgentPersona) => void;
  onSubmit: (input: CreatePersonaInput | UpdatePersonaInput) => Promise<void>;
}) {
  return (
    <>
      <AgentDialog
        description={personaDialogState?.description ?? ""}
        editsProviderRecord={editsProviderRecord}
        error={updateError ?? createError}
        initialValues={personaDialogState?.initialValues ?? null}
        isPending={isPending}
        mode="definition-edit"
        runtimes={runtimes}
        runtimesLoading={runtimesLoading}
        onOpenChange={(open) => {
          if (!open) {
            onCloseDialog();
          }
        }}
        onSubmit={onSubmit}
        open={personaDialogState !== null}
        submitLabel={personaDialogState?.submitLabel ?? "Save"}
        title={personaDialogState?.title ?? "Agent"}
      />
      <PersonaDeleteDialog
        instanceCount={instanceCount}
        onConfirm={onConfirmDelete}
        onOpenChange={(open) => {
          if (!open) {
            onCloseDelete();
          }
        }}
        open={personaToDelete !== null}
        persona={personaToDelete}
        remoteInstances={remoteInstances}
      />
    </>
  );
}
