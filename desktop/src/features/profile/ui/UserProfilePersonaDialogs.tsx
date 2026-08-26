import * as React from "react";

import type {
  AcpRuntimeCatalogEntry,
  AgentPersona,
  CreatePersonaInput,
  ManagedAgent,
  UpdatePersonaInput,
} from "@/shared/api/types";
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

/**
 * Card-mint dialog state plus the callback that opens it. `create` is
 * undefined when no persona resolves; owner gating is the caller's job.
 */
export function useCardMint(
  persona: AgentPersona | undefined,
  managedAgent: ManagedAgent | undefined,
) {
  const [target, setTarget] = React.useState<CardMintTarget | null>(null);
  const close = React.useCallback(() => setTarget(null), []);
  const create = persona
    ? () =>
        setTarget({
          // Prefer the live instance pubkey; fall back to the
          // persona/definition id (same resolution as export).
          id: managedAgent?.pubkey ?? persona.id,
          name: persona.displayName,
          // Locking needs an instance keypair to encrypt to.
          canLock: Boolean(managedAgent?.pubkey),
        })
    : undefined;
  return { close, create, target };
}

export function UserProfilePersonaDialogs({
  cardMintTarget,
  createError,
  instanceCount,
  isPending,
  linkedAgentPubkey,
  personaDialogState,
  personaToDelete,
  personaToExportSnapshot,
  resolvedPersona,
  runtimes,
  runtimesLoading,
  runtimesError = false,
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
  /** Number of managed-agent instances backed by the persona being deleted. */
  instanceCount: number;
  isPending: boolean;
  linkedAgentPubkey: string | null;
  personaDialogState: PersonaDialogState | null;
  personaToDelete: AgentPersona | null;
  personaToExportSnapshot: AgentPersona | null;
  resolvedPersona: AgentPersona | undefined;
  runtimes: AcpRuntimeCatalogEntry[];
  runtimesLoading: boolean;
  runtimesError?: boolean;
  updateError: Error | null;
  onCloseCardMint: () => void;
  onCloseDelete: () => void;
  onCloseDialog: () => void;
  onCloseExportSnapshot: () => void;
  onConfirmDelete: (persona: AgentPersona) => void;
  onExportSnapshot: (persona: AgentPersona) => void;
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
