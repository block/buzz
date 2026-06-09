import * as React from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";

import {
  managedAgentsQueryKey,
  personasQueryKey,
  teamsQueryKey,
  useCreateTeamMutation,
  useDeleteTeamMutation,
  useTeamsQuery,
  useUpdateTeamMutation,
} from "@/features/agents/hooks";
import type { CreateChannelManagedAgentsResult } from "@/features/agents/channelAgents";
import {
  type ParsedTeamPreview,
  createTeam as createTeamApi,
  exportTeamToJson,
  parseTeamFile,
} from "@/shared/api/tauriTeams";
import {
  createPersona,
  deletePersona,
  updatePersona,
} from "@/shared/api/tauriPersonas";
import type {
  AgentPersona,
  AgentTeam,
  Channel,
  CreateTeamInput,
  UpdateTeamInput,
} from "@/shared/api/types";
import { buildTeamImportPlan } from "./teamImportPlan";

type TeamDialogState = {
  description: string;
  initialValues: CreateTeamInput | UpdateTeamInput;
  submitLabel: string;
  title: string;
} | null;

type ActionMessages = {
  setActionNoticeMessage: (message: string | null) => void;
  setActionErrorMessage: (message: string | null) => void;
};

type RefetchCallbacks = {
  refetchManagedAgents: () => void;
  refetchRelayAgents: () => void;
};

type TeamImportUpdateApplyInput = {
  personas: AgentPersona[];
  updateTeamInfo: boolean;
  selectedUpdatedPersonaIds: string[];
  selectedNewMemberIndexes: number[];
  missingPersonaIdsToRemove: string[];
  deleteRemovedAgents: boolean;
};

export function useTeamActions(
  actions: ActionMessages,
  refetch: RefetchCallbacks,
) {
  const queryClient = useQueryClient();
  const teamsQuery = useTeamsQuery();
  const createTeamMutation = useCreateTeamMutation();
  const updateTeamMutation = useUpdateTeamMutation();
  const deleteTeamMutation = useDeleteTeamMutation();

  const exportTeamJsonMutation = useMutation({
    mutationFn: (id: string) => exportTeamToJson(id),
  });

  const [teamDialogState, setTeamDialogState] =
    React.useState<TeamDialogState>(null);
  const [teamToDelete, setTeamToDelete] = React.useState<AgentTeam | null>(
    null,
  );
  const [teamToAddToChannel, setTeamToAddToChannel] =
    React.useState<AgentTeam | null>(null);
  const [teamImportPreview, setTeamImportPreview] = React.useState<{
    preview: ParsedTeamPreview;
    fileName: string;
  } | null>(null);
  const [teamImportTarget, setTeamImportTarget] =
    React.useState<AgentTeam | null>(null);
  const [teamImportTargetPreview, setTeamImportTargetPreview] = React.useState<{
    preview: ParsedTeamPreview;
    fileName: string;
  } | null>(null);
  const [isApplyingTeamImportUpdate, setIsApplyingTeamImportUpdate] =
    React.useState(false);

  const teams = teamsQuery.data ?? [];

  async function handleTeamSubmit(input: CreateTeamInput | UpdateTeamInput) {
    actions.setActionNoticeMessage(null);
    actions.setActionErrorMessage(null);

    try {
      if ("id" in input) {
        await updateTeamMutation.mutateAsync(input);
        actions.setActionNoticeMessage(`Updated team "${input.name}".`);
      } else {
        await createTeamMutation.mutateAsync(input);
        actions.setActionNoticeMessage(`Created team "${input.name}".`);
      }
      setTeamDialogState(null);
    } catch (error) {
      actions.setActionErrorMessage(
        error instanceof Error ? error.message : "Failed to save team.",
      );
    }
  }

  async function handleDeleteTeam(team: AgentTeam) {
    actions.setActionNoticeMessage(null);
    actions.setActionErrorMessage(null);

    try {
      await deleteTeamMutation.mutateAsync(team.id);
      actions.setActionNoticeMessage(`Deleted team "${team.name}".`);
      setTeamToDelete(null);
    } catch (error) {
      actions.setActionErrorMessage(
        error instanceof Error ? error.message : "Failed to delete team.",
      );
    }
  }

  function handleTeamDeployed(
    channel: Channel,
    result: CreateChannelManagedAgentsResult,
  ) {
    actions.setActionErrorMessage(null);
    const successCount = result.successes.length;
    const failCount = result.failures.length;
    if (failCount === 0) {
      actions.setActionNoticeMessage(
        `Deployed ${successCount} ${successCount === 1 ? "agent" : "agents"} to ${channel.name}.`,
      );
    } else {
      actions.setActionNoticeMessage(
        `Deployed ${successCount} ${successCount === 1 ? "agent" : "agents"} to ${channel.name}. ${failCount} failed.`,
      );
    }
    setTeamToAddToChannel(null);
    refetch.refetchManagedAgents();
    refetch.refetchRelayAgents();
  }

  function openCreateDialog() {
    actions.setActionNoticeMessage(null);
    actions.setActionErrorMessage(null);
    setTeamDialogState({
      title: "Create team",
      description: "Group personas together for quick deployment to channels.",
      submitLabel: "Create team",
      initialValues: {
        name: "",
        description: "",
        personaIds: [],
      },
    });
  }

  function openDuplicateDialog(team: AgentTeam) {
    actions.setActionNoticeMessage(null);
    actions.setActionErrorMessage(null);
    setTeamDialogState({
      title: `Duplicate ${team.name}`,
      description: "Create a new team by copying this one.",
      submitLabel: "Create team",
      initialValues: {
        name: `${team.name} copy`,
        description: team.description ?? "",
        personaIds: [...team.personaIds],
      },
    });
  }

  function handleExportTeam(team: AgentTeam) {
    exportTeamJsonMutation.mutate(team.id, {
      onSuccess: (saved) => {
        if (saved) {
          actions.setActionNoticeMessage(`Exported team "${team.name}".`);
        }
      },
      onError: (err) => {
        actions.setActionErrorMessage(
          err instanceof Error ? err.message : "Failed to export team.",
        );
      },
    });
  }

  async function handleImportFile(fileBytes: number[], fileName: string) {
    actions.setActionNoticeMessage(null);
    actions.setActionErrorMessage(null);
    try {
      const preview = await parseTeamFile(fileBytes, fileName);
      setTeamImportPreview({ preview, fileName });
    } catch (err) {
      actions.setActionErrorMessage(
        err instanceof Error ? err.message : "Failed to parse team file.",
      );
    }
  }

  async function handleEditDialogImportUpdateFile(
    teamId: string,
    fileBytes: number[],
    fileName: string,
  ) {
    actions.setActionNoticeMessage(null);
    actions.setActionErrorMessage(null);

    const team = teams.find((candidate) => candidate.id === teamId);
    if (!team) {
      const message = "Team not found. Refresh and try again.";
      actions.setActionErrorMessage(message);
      throw new Error(message);
    }

    try {
      const preview = await parseTeamFile(fileBytes, fileName);
      setTeamImportTarget(team);
      setTeamImportTargetPreview({ preview, fileName });
      setTeamDialogState(null);
    } catch (err) {
      const message =
        err instanceof Error ? err.message : "Failed to parse team file.";
      actions.setActionErrorMessage(message);
      throw err instanceof Error ? err : new Error(message);
    }
  }

  function closeImportUpdateDialog() {
    setTeamImportTarget(null);
    setTeamImportTargetPreview(null);
    setIsApplyingTeamImportUpdate(false);
  }

  function clearImportUpdateAndReturnToEdit() {
    if (!teamImportTarget) {
      closeImportUpdateDialog();
      return;
    }

    const team = teamImportTarget;
    closeImportUpdateDialog();
    openEditDialog(team);
  }

  async function handleTeamImportUpdateApply({
    personas,
    updateTeamInfo,
    selectedUpdatedPersonaIds,
    selectedNewMemberIndexes,
    missingPersonaIdsToRemove,
    deleteRemovedAgents,
  }: TeamImportUpdateApplyInput) {
    if (!teamImportTarget || !teamImportTargetPreview) {
      throw new Error("No team import update is currently open.");
    }

    actions.setActionNoticeMessage(null);
    actions.setActionErrorMessage(null);
    setIsApplyingTeamImportUpdate(true);

    const plan = buildTeamImportPlan({
      team: teamImportTarget,
      personas,
      preview: teamImportTargetPreview.preview,
    });
    const selectedUpdatedPersonaIdSet = new Set(selectedUpdatedPersonaIds);
    const selectedNewMemberIndexSet = new Set(selectedNewMemberIndexes);
    const removePersonaIdSet = new Set(missingPersonaIdsToRemove);

    try {
      let updatedMembersCount = 0;
      for (const member of plan.membersToUpdate) {
        if (!selectedUpdatedPersonaIdSet.has(member.existing.id)) {
          continue;
        }
        await updatePersona({
          id: member.existing.id,
          displayName: member.imported.display_name,
          systemPrompt: member.imported.system_prompt,
          avatarUrl: member.imported.avatar_url ?? undefined,
          runtime: member.existing.runtime ?? undefined,
          model: member.existing.model ?? undefined,
          namePool: [...member.existing.namePool],
        });
        updatedMembersCount += 1;
      }

      const createdPersonaIdsByImportedIndex = new Map<number, string>();
      for (const member of plan.newMembers) {
        if (!selectedNewMemberIndexSet.has(member.importedIndex)) {
          continue;
        }
        const created = await createPersona({
          displayName: member.imported.display_name,
          systemPrompt: member.imported.system_prompt,
          avatarUrl: member.imported.avatar_url ?? undefined,
        });
        createdPersonaIdsByImportedIndex.set(member.importedIndex, created.id);
      }

      const matchedPersonaIdsByImportedIndex = new Map<number, string>();
      for (const member of plan.matchedMembers) {
        matchedPersonaIdsByImportedIndex.set(
          member.importedIndex,
          member.existing.id,
        );
      }

      const nextPersonaIds: string[] = [];
      for (
        let importedIndex = 0;
        importedIndex < teamImportTargetPreview.preview.personas.length;
        importedIndex += 1
      ) {
        const matchedId = matchedPersonaIdsByImportedIndex.get(importedIndex);
        if (matchedId) {
          nextPersonaIds.push(matchedId);
          continue;
        }

        const createdId = createdPersonaIdsByImportedIndex.get(importedIndex);
        if (createdId) {
          nextPersonaIds.push(createdId);
        }
      }

      const removedMembers = plan.missingMembers.filter((member) =>
        removePersonaIdSet.has(member.existing.id),
      );
      const keptMissingMembers = plan.missingMembers.filter(
        (member) => !removePersonaIdSet.has(member.existing.id),
      );
      for (const member of keptMissingMembers) {
        nextPersonaIds.push(member.existing.id);
      }

      const nextTeamName = updateTeamInfo
        ? teamImportTargetPreview.preview.name
        : teamImportTarget.name;
      const nextTeamDescription = updateTeamInfo
        ? (teamImportTargetPreview.preview.description ?? undefined)
        : (teamImportTarget.description ?? undefined);

      await updateTeamMutation.mutateAsync({
        id: teamImportTarget.id,
        name: nextTeamName,
        description: nextTeamDescription,
        personaIds: nextPersonaIds,
      });

      let deletedAgentsCount = 0;
      const deleteFailures: string[] = [];
      const addedMembersCount = createdPersonaIdsByImportedIndex.size;
      if (deleteRemovedAgents && removedMembers.length > 0) {
        for (const member of removedMembers) {
          try {
            await deletePersona(member.existing.id);
            deletedAgentsCount += 1;
          } catch (error) {
            const reason =
              error instanceof Error ? error.message : String(error);
            deleteFailures.push(`${member.existing.displayName}: ${reason}`);
          }
        }
      }

      actions.setActionNoticeMessage(
        `Updated "${nextTeamName}" from import. ${updatedMembersCount} member${updatedMembersCount === 1 ? "" : "s"} updated, ${addedMembersCount} added, ${removedMembers.length} removed from the team${deleteRemovedAgents ? `, and ${deletedAgentsCount} removed from My Agents` : ""}.`,
      );

      if (deleteFailures.length > 0) {
        actions.setActionErrorMessage(
          `Team updated, but ${deleteFailures.length} agent${deleteFailures.length === 1 ? "" : "s"} could not be removed: ${deleteFailures.join("; ")}`,
        );
      }

      closeImportUpdateDialog();
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: personasQueryKey }),
        queryClient.invalidateQueries({ queryKey: teamsQueryKey }),
        queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey }),
      ]);
    } catch (error) {
      const message =
        error instanceof Error
          ? error.message
          : "Failed to apply imported team update.";
      actions.setActionErrorMessage(message);
      throw error instanceof Error ? error : new Error(message);
    } finally {
      setIsApplyingTeamImportUpdate(false);
    }
  }

  function handleTeamImportComplete(
    teamName: string,
    teamDescription: string | null,
    personaIds: string[],
  ) {
    setTeamImportPreview(null);
    void (async () => {
      const teamInput = {
        name: teamName,
        description: teamDescription ?? undefined,
        personaIds,
      };

      // Try creating the team, retry once on failure.
      for (let attempt = 0; attempt < 2; attempt++) {
        try {
          await createTeamApi(teamInput);
          actions.setActionNoticeMessage(
            `Imported team "${teamName}" with ${personaIds.length} persona${personaIds.length !== 1 ? "s" : ""}.`,
          );
          void queryClient.invalidateQueries({ queryKey: personasQueryKey });
          void queryClient.invalidateQueries({ queryKey: teamsQueryKey });
          return;
        } catch {
          if (attempt === 0) continue;
        }
      }

      // Both attempts failed — personas exist but team doesn't.
      actions.setActionErrorMessage(
        `Imported ${personaIds.length} persona${personaIds.length !== 1 ? "s" : ""} but failed to create team "${teamName}". The personas are saved — create a team manually to group them.`,
      );
      void queryClient.invalidateQueries({ queryKey: personasQueryKey });
    })();
  }

  async function handleDeleteRemovedPersonas(personaIds: string[]) {
    for (const id of personaIds) {
      try {
        await deletePersona(id);
      } catch {
        // Best-effort: persona may already be deleted or in use elsewhere.
      }
    }
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: personasQueryKey }),
      queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey }),
    ]);
  }

  function openEditDialog(team: AgentTeam) {
    actions.setActionNoticeMessage(null);
    actions.setActionErrorMessage(null);
    setTeamDialogState({
      title: "Edit team",
      description: "",
      submitLabel: "Save changes",
      initialValues: {
        id: team.id,
        name: team.name,
        description: team.description ?? "",
        personaIds: [...team.personaIds],
      },
    });
  }

  return {
    teams,
    teamsQuery,
    createTeamMutation,
    updateTeamMutation,
    deleteTeamMutation,
    exportTeamJsonMutation,
    teamDialogState,
    setTeamDialogState,
    teamToDelete,
    setTeamToDelete,
    teamToAddToChannel,
    setTeamToAddToChannel,
    teamImportPreview,
    setTeamImportPreview,
    teamImportTarget,
    teamImportTargetPreview,
    isApplyingTeamImportUpdate,
    handleTeamSubmit,
    handleDeleteRemovedPersonas,
    handleDeleteTeam,
    handleTeamDeployed,
    handleExportTeam,
    handleImportFile,
    handleEditDialogImportUpdateFile,
    handleTeamImportComplete,
    handleTeamImportUpdateApply,
    closeImportUpdateDialog,
    clearImportUpdateAndReturnToEdit,
    openCreateDialog,
    openDuplicateDialog,
    openEditDialog,
  };
}
