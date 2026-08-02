import type {
  AcpRuntimeCatalogEntry,
  AgentPersona,
  CreatePersonaInput,
  UpdatePersonaInput,
} from "@/shared/api/types";
import type { PersonaRemoteCascadeInstance } from "@/features/agents/lib/personaCascade";
import { AgentCardMintDialog } from "@/features/agents/ui/AgentCardMintDialog";
import { PersonaDeleteDialog } from "@/features/agents/ui/PersonaDeleteDialog";
import { AgentDialog } from "@/features/agents/ui/AgentDialog";
import type { PersonaDialogState } from "@/features/agents/ui/personaDialogState";
import { UserProfileSnapshotExportDialog } from "@/features/profile/ui/UserProfileSnapshotExportDialog";

/** Agent selected for card minting. */
export type CardMintTarget = {
  id: string;
  name: string;
  canLock: boolean;
};

export function UserProfilePersonaDialogs({
  cardMintTarget,
  createError,
  editsProviderRecord = false,
  instanceCount,
  isPending,
  linkedAgentPubkey,
  personaDialogState,
  personaToDelete,
  personaToExportSnapshot,
  remoteInstances = [],
  resolvedPersona,
  runtimes,
  runtimesLoading,
  updateError,
  onCloseCardMint,
  onCloseDelete,
  onCloseDialog,
  onCloseExportSnapshot,
  onConfirmDelete,
  onExportSnapshot,
  onSubmit,
}: {
  cardMintTarget: CardMintTarget | null;
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
  linkedAgentPubkey: string | null;
  personaDialogState: PersonaDialogState | null;
  personaToDelete: AgentPersona | null;
  personaToExportSnapshot: AgentPersona | null;
  /** Cascade instances whose remote deployment survives the delete. */
  remoteInstances?: readonly PersonaRemoteCascadeInstance[];
  resolvedPersona: AgentPersona | undefined;
  runtimes: AcpRuntimeCatalogEntry[];
  runtimesLoading: boolean;
  updateError: Error | null;
  onCloseCardMint: () => void;
  onCloseDelete: () => void;
  onCloseDialog: () => void;
  onCloseExportSnapshot: () => void;
  onConfirmDelete: (persona: AgentPersona) => void;
  onExportSnapshot: (persona: AgentPersona) => void;
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
      {personaToExportSnapshot ? (
        <UserProfileSnapshotExportDialog
          linkedAgentPubkey={linkedAgentPubkey}
          onOpenChange={(open) => {
            if (!open) onCloseExportSnapshot();
          }}
          persona={personaToExportSnapshot}
        />
      ) : null}
      {cardMintTarget ? (
        <AgentCardMintDialog
          agentId={cardMintTarget.id}
          agentName={cardMintTarget.name}
          canLock={cardMintTarget.canLock}
          onExportInstead={
            resolvedPersona
              ? () => {
                  // Free path: swap the mint dialog for the ordinary
                  // snapshot export flow (same importable agent, no spend).
                  onCloseCardMint();
                  onExportSnapshot(resolvedPersona);
                }
              : undefined
          }
          onOpenChange={(open) => {
            if (!open) onCloseCardMint();
          }}
        />
      ) : null}
    </>
  );
}
