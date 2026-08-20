import * as React from "react";
import { toast } from "sonner";

import type { AgentPersona, ManagedAgent } from "@/shared/api/types";
import type { deleteProfileManagedAgentsForPersona } from "@/features/profile/ui/UserProfilePanelDeletion";

type DeleteManagedAgentsForPersona = (
  persona: AgentPersona,
) => ReturnType<typeof deleteProfileManagedAgentsForPersona>;

type UseProfilePersonaDeleteInput = {
  deleteManagedAgentsForPersona: DeleteManagedAgentsForPersona;
  deletePersona: (id: string) => Promise<unknown>;
  managedAgents?: readonly ManagedAgent[];
  onClose: () => void;
};

/**
 * Confirm-and-delete state for a persona in the profile panel: the pending
 * persona, the two instance counts the confirm dialog discloses, and the
 * confirmed-delete handler.
 *
 * Split out of `UserProfilePanel` to keep that component under the desktop
 * file-size ratchet, and kept beside `UserProfilePanelDeletion` because it is
 * the same concern — that module owns the cascade, this one owns the dialog
 * state driving it.
 */
export function useProfilePersonaDelete({
  deleteManagedAgentsForPersona,
  deletePersona,
  managedAgents,
  onClose,
}: UseProfilePersonaDeleteInput) {
  const [personaToDelete, setPersonaToDelete] =
    React.useState<AgentPersona | null>(null);

  // Instances backed by the persona being deleted, shown in the confirm dialog
  // so the user knows what the cascade takes with it.
  const instanceCount = React.useMemo(
    () =>
      personaToDelete
        ? (managedAgents ?? []).filter(
            (a) => a.personaId === personaToDelete.id,
          ).length
        : 0,
    [managedAgents, personaToDelete],
  );

  // Only instances actually deployed to a provider — the same condition the
  // backend rejects on and under which `deleteManagedAgentWithRules` forces —
  // so the dialog does not warn about a provider that was never involved.
  const providerInstanceCount = React.useMemo(
    () =>
      personaToDelete
        ? (managedAgents ?? []).filter(
            (a) =>
              a.personaId === personaToDelete.id &&
              a.backend.type === "provider" &&
              a.backendAgentId,
          ).length
        : 0,
    [managedAgents, personaToDelete],
  );

  const handleConfirmDeletePersona = React.useCallback(
    async (personaToConfirm: AgentPersona) => {
      if (personaToConfirm.sourceTeam) {
        toast.error("This agent is managed by a team.");
        setPersonaToDelete(null);
        return;
      }

      try {
        // Instances first: delete_persona refuses to cascade over a
        // provider-deployed instance, so the persona delete fails outright
        // unless they are torn down.
        const instances = await deleteManagedAgentsForPersona(personaToConfirm);
        if (instances.cancelled) {
          // Cancelling stops further deletes but cannot undo the ones that
          // already ran. Say so — otherwise an instance destroyed by a flow the
          // user cancelled disappears with no feedback, on a path that toasts
          // every other outcome.
          if (instances.deletedCount > 0) {
            toast.warning(
              `Deleted ${instances.deletedCount} agent instance${
                instances.deletedCount === 1 ? "" : "s"
              }; kept ${personaToConfirm.displayName} because you cancelled.`,
            );
          }
          return;
        }

        await deletePersona(personaToConfirm.id);
        toast.success(`Deleted ${personaToConfirm.displayName}.`);
        setPersonaToDelete(null);
        onClose();
      } catch (error) {
        toast.error(
          error instanceof Error ? error.message : "Failed to delete agent.",
        );
      }
    },
    [deleteManagedAgentsForPersona, deletePersona, onClose],
  );

  return {
    personaToDelete,
    setPersonaToDelete,
    instanceCount,
    providerInstanceCount,
    handleConfirmDeletePersona,
  };
}
