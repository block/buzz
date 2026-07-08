import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import { personasQueryKey } from "@/features/agents/hooks";
import {
  parsePersonaFiles,
  updatePersona as updatePersonaApi,
  type ParsedPersonaPreview,
} from "@/shared/api/tauriPersonas";
import type { AgentPersona } from "@/shared/api/types";
import { resolveManagedAgentAvatarUrl } from "./managedAgentAvatar";
import { buildPersonaImportPlan } from "./personaImportPlan";
import { buildPersonaImportUpdateInput } from "./personaImportUpdateInput";
import {
  editPersonaDialogState,
  type PersonaDialogState,
} from "./personaDialogState";

type FeedbackCallbacks = {
  clearPersonaFeedback: () => void;
  setPersonaNoticeMessage: (message: string | null) => void;
  setPersonaErrorMessage: (message: string | null) => void;
  setPersonaDialogState: (state: PersonaDialogState | null) => void;
};

export function usePersonaImportActions(
  libraryPersonas: AgentPersona[],
  feedback: FeedbackCallbacks,
) {
  const queryClient = useQueryClient();
  const [personaImportTarget, setPersonaImportTarget] =
    React.useState<AgentPersona | null>(null);
  const [personaImportTargetPreview, setPersonaImportTargetPreview] =
    React.useState<{ preview: ParsedPersonaPreview; fileName: string } | null>(
      null,
    );
  const [isApplyingPersonaImportUpdate, setIsApplyingPersonaImportUpdate] =
    React.useState(false);

  async function handleEditDialogImportUpdateFile(
    personaId: string,
    fileBytes: number[],
    fileName: string,
  ) {
    feedback.clearPersonaFeedback();

    const persona = libraryPersonas.find(
      (candidate) => candidate.id === personaId,
    );
    if (!persona) {
      const message = "Agent not found. Refresh and try again.";
      feedback.setPersonaErrorMessage(message);
      throw new Error(message);
    }

    try {
      const result = await parsePersonaFiles(fileBytes, fileName);
      if (result.personas.length === 0) {
        const message = "No valid agents found in file.";
        feedback.setPersonaErrorMessage(message);
        throw new Error(message);
      }
      const preview = result.personas[0];
      setPersonaImportTarget(persona);
      setPersonaImportTargetPreview({ preview, fileName });
      feedback.setPersonaDialogState(null);
    } catch (err) {
      const message =
        err instanceof Error ? err.message : "Failed to parse agent file.";
      feedback.setPersonaErrorMessage(message);
      throw err instanceof Error ? err : new Error(message);
    }
  }

  function closeImportUpdateDialog() {
    setPersonaImportTarget(null);
    setPersonaImportTargetPreview(null);
    setIsApplyingPersonaImportUpdate(false);
  }

  function clearImportUpdateAndReturnToEdit() {
    if (!personaImportTarget) {
      closeImportUpdateDialog();
      return;
    }

    const persona = personaImportTarget;
    closeImportUpdateDialog();
    feedback.clearPersonaFeedback();
    feedback.setPersonaDialogState(editPersonaDialogState(persona));
  }

  async function handleImportUpdateApply({
    selectedFields,
  }: {
    selectedFields: string[];
  }) {
    if (!personaImportTarget || !personaImportTargetPreview) {
      throw new Error("No persona import update is currently open.");
    }

    feedback.clearPersonaFeedback();
    setIsApplyingPersonaImportUpdate(true);

    const plan = buildPersonaImportPlan({
      persona: personaImportTarget,
      preview: personaImportTargetPreview.preview,
    });

    const preview = personaImportTargetPreview.preview;
    const existing = personaImportTarget;

    try {
      const updateInput = buildPersonaImportUpdateInput({
        existing,
        preview,
        selectedFields,
      });
      if (selectedFields.includes("avatarUrl")) {
        updateInput.avatarUrl = await resolveManagedAgentAvatarUrl(
          updateInput.avatarUrl,
          undefined,
          existing.avatarUrl,
        );
      }

      await updatePersonaApi(updateInput);

      const updatedFieldCount = plan.fields.filter((field) =>
        selectedFields.includes(field.field),
      ).length;

      feedback.setPersonaNoticeMessage(
        `Updated "${updateInput.displayName}" from import. ${updatedFieldCount} field${updatedFieldCount === 1 ? "" : "s"} updated.`,
      );

      closeImportUpdateDialog();
      await queryClient.invalidateQueries({ queryKey: personasQueryKey });
    } catch (error) {
      const message =
        error instanceof Error
          ? error.message
          : "Failed to apply imported persona update.";
      feedback.setPersonaErrorMessage(message);
      throw error instanceof Error ? error : new Error(message);
    } finally {
      setIsApplyingPersonaImportUpdate(false);
    }
  }

  return {
    personaImportTarget,
    personaImportTargetPreview,
    isApplyingPersonaImportUpdate,
    handleEditDialogImportUpdateFile,
    handleImportUpdateApply,
    closeImportUpdateDialog,
    clearImportUpdateAndReturnToEdit,
  };
}
