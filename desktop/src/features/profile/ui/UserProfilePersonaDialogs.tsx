import type {
  AcpRuntimeCatalogEntry,
  AgentPersona,
  CreatePersonaInput,
  UpdatePersonaInput,
} from "@/shared/api/types";
import { PersonaDeleteDialog } from "@/features/agents/ui/PersonaDeleteDialog";
import { AgentDialog } from "@/features/agents/ui/AgentDialog";
import type { PersonaDialogState } from "@/features/agents/ui/personaDialogState";

export function UserProfilePersonaDialogs({
  createError,
  isPending,
  personaDialogState,
  personaToDelete,
  runtimes,
  runtimesLoading,
  updateError,
  onCloseDelete,
  onCloseDialog,
  onConfirmDelete,
  onSubmit,
}: {
  createError: Error | null;
  isPending: boolean;
  personaDialogState: PersonaDialogState | null;
  personaToDelete: AgentPersona | null;
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
        onConfirm={onConfirmDelete}
        onOpenChange={(open) => {
          if (!open) {
            onCloseDelete();
          }
        }}
        open={personaToDelete !== null}
        persona={personaToDelete}
      />
    </>
  );
}
