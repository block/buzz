import type {
  AcpRuntimeCatalogEntry,
  AgentPersona,
  CreatePersonaInput,
  UpdatePersonaInput,
} from "@/shared/api/types";
import { PersonaDeleteDialog } from "@/features/agents/ui/PersonaDeleteDialog";
import { AgentDialog } from "@/features/agents/ui/AgentDialog";
import type { PersonaDialogState } from "@/features/agents/ui/personaDialogState";
import { UserProfileSnapshotExportDialog } from "@/features/profile/ui/UserProfileSnapshotExportDialog";

export function UserProfilePersonaDialogs({
  agentAvatarUrl,
  createError,
  instanceCount,
  isPending,
  linkedAgentPubkey,
  personaDialogState,
  personaToDelete,
  personaToExportSnapshot,
  runtimes,
  runtimesLoading,
  runtimesError = false,
  updateError,
  onCloseDelete,
  onCloseDialog,
  onCloseExportSnapshot,
  onConfirmDelete,
  onSubmit,
}: {
  agentAvatarUrl: string | null;
  createError: Error | null;
  /** Number of managed-agent instances backed by the persona being deleted. */
  instanceCount: number;
  isPending: boolean;
  linkedAgentPubkey: string | null;
  personaDialogState: PersonaDialogState | null;
  personaToDelete: AgentPersona | null;
  personaToExportSnapshot: AgentPersona | null;
  runtimes: AcpRuntimeCatalogEntry[];
  runtimesLoading: boolean;
  runtimesError?: boolean;
  updateError: Error | null;
  onCloseDelete: () => void;
  onCloseDialog: () => void;
  onCloseExportSnapshot: () => void;
  onConfirmDelete: (persona: AgentPersona) => void;
  onSubmit: (input: CreatePersonaInput | UpdatePersonaInput) => Promise<void>;
}) {
  const runtimeCatalogStatus = runtimesLoading
    ? "loading"
    : runtimesError
      ? "error"
      : ("ready" as const);
  return (
    <>
      <AgentDialog
        description={personaDialogState?.description ?? ""}
        error={updateError ?? createError}
        initialValues={personaDialogState?.initialValues ?? null}
        isPending={isPending}
        mode="definition-edit"
        runtimes={runtimes}
        runtimeCatalogStatus={runtimeCatalogStatus}
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
      />
      {personaToExportSnapshot ? (
        <UserProfileSnapshotExportDialog
          agentAvatarUrl={agentAvatarUrl}
          linkedAgentPubkey={linkedAgentPubkey}
          onOpenChange={(open) => {
            if (!open) onCloseExportSnapshot();
          }}
          persona={personaToExportSnapshot}
        />
      ) : null}
    </>
  );
}
