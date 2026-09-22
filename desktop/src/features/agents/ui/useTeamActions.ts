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
import type { CatalogTeamShareLevel } from "@/features/agents/lib/teamCatalogRelay";
import {
  catalogTeamsFromPublications,
  type CatalogTeam,
} from "@/features/agents/lib/teamCatalogRelay";
import {
  useAddTeamFromCatalogMutation,
  useSetTeamCatalogSharedMutation,
  useTeamCatalogLiveUpdates,
  useTeamCatalogQuery,
} from "@/features/agents/lib/useTeamCatalogRelay";
import { useCommunities } from "@/features/communities/useCommunities";
import type { CreateChannelManagedAgentsResult } from "@/features/agents/channelAgents";
import { useIdentityQuery } from "@/shared/api/hooks";
import { deletePersona } from "@/shared/api/tauriPersonas";
import {
  confirmTeamSnapshotImport,
  exportTeamSnapshot,
  previewTeamSnapshotImport,
  type SnapshotFormat,
  type SnapshotMemoryLevel,
  type TeamSnapshotImportPreview,
  type TeamSnapshotImportResult,
} from "@/shared/api/tauriTeams";
import type {
  AgentTeam,
  Channel,
  CreateTeamInput,
  UpdateTeamInput,
} from "@/shared/api/types";
import { useTranslation } from "@/i18n";
import { deriveImportToast } from "./teamSnapshotImport.lib";
import { teamShareNotice } from "./teamLibraryCopy";

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

export function useTeamActions(
  actions: ActionMessages,
  refetch: RefetchCallbacks,
) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const { activeCommunity } = useCommunities();
  const identityQuery = useIdentityQuery();
  const communityId = activeCommunity?.id ?? null;
  const teamsQuery = useTeamsQuery();
  const catalogQuery = useTeamCatalogQuery(communityId);
  useTeamCatalogLiveUpdates(communityId);
  const setCatalogSharedMutation = useSetTeamCatalogSharedMutation(communityId);
  const addTeamFromCatalogMutation = useAddTeamFromCatalogMutation();
  const createTeamMutation = useCreateTeamMutation();
  const updateTeamMutation = useUpdateTeamMutation();
  const deleteTeamMutation = useDeleteTeamMutation();

  const [teamDialogState, setTeamDialogState] =
    React.useState<TeamDialogState>(null);
  const [teamToDelete, setTeamToDelete] = React.useState<AgentTeam | null>(
    null,
  );
  const [teamToAddToChannel, setTeamToAddToChannel] =
    React.useState<AgentTeam | null>(null);
  const [teamToExport, setTeamToExport] = React.useState<AgentTeam | null>(
    null,
  );
  const [teamToShare, setTeamToShare] = React.useState<AgentTeam | null>(null);
  const [teamSnapshotImportState, setTeamSnapshotImportState] = React.useState<{
    fileBytes: number[];
    fileName: string;
    preview: TeamSnapshotImportPreview;
  } | null>(null);
  const [teamSnapshotImportResult, setTeamSnapshotImportResult] =
    React.useState<TeamSnapshotImportResult | null>(null);
  const [teamSnapshotImportConfirmError, setTeamSnapshotImportConfirmError] =
    React.useState<string | null>(null);

  const exportTeamSnapshotMutation = useMutation({
    mutationFn: async ({
      id,
      memoryLevel,
      format,
    }: {
      id: string;
      memoryLevel: SnapshotMemoryLevel;
      format: SnapshotFormat;
    }) => exportTeamSnapshot(id, memoryLevel, format),
  });
  const previewTeamSnapshotImportMutation = useMutation({
    mutationFn: ({
      fileBytes,
      fileName,
    }: {
      fileBytes: number[];
      fileName: string;
    }) => previewTeamSnapshotImport(fileBytes, fileName),
  });
  const confirmTeamSnapshotImportMutation = useMutation({
    mutationFn: (input: { fileBytes: number[]; keepAllowlist: boolean }) =>
      confirmTeamSnapshotImport(input),
  });

  const teams = teamsQuery.data ?? [];
  const publications = catalogQuery.data ?? [];
  const catalogTeams = React.useMemo(
    () =>
      catalogTeamsFromPublications(
        publications,
        teams,
        identityQuery.data?.pubkey,
      ),
    [identityQuery.data?.pubkey, publications, teams],
  );

  async function handleTeamSubmit(input: CreateTeamInput | UpdateTeamInput) {
    actions.setActionNoticeMessage(null);
    actions.setActionErrorMessage(null);

    try {
      if ("id" in input) {
        await updateTeamMutation.mutateAsync(input);
        actions.setActionNoticeMessage(
          t("agents.team-actions.updated", { name: input.name }),
        );
      } else {
        await createTeamMutation.mutateAsync(input);
        actions.setActionNoticeMessage(
          t("agents.team-actions.created", { name: input.name }),
        );
      }
      setTeamDialogState(null);
    } catch (error) {
      actions.setActionErrorMessage(
        error instanceof Error
          ? error.message
          : t("agents.team-actions.save-failed"),
      );
    }
  }

  async function handleDeleteTeam(team: AgentTeam) {
    actions.setActionNoticeMessage(null);
    actions.setActionErrorMessage(null);

    try {
      await deleteTeamMutation.mutateAsync(team.id);
      actions.setActionNoticeMessage(
        t("agents.team-actions.deleted", { name: team.name }),
      );
      setTeamToDelete(null);
    } catch (error) {
      actions.setActionErrorMessage(
        error instanceof Error
          ? error.message
          : t("agents.team-actions.delete-failed"),
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
        t("agents.team-actions.deployed", {
          count: successCount,
          channelName: channel.name,
        }),
      );
    } else {
      actions.setActionNoticeMessage(
        t("agents.team-actions.deployed-partial", {
          count: successCount,
          channelName: channel.name,
          failed: failCount,
        }),
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
      title: t("agents.team-actions.create-team"),
      description: t("agents.team-actions.create-description"),
      submitLabel: t("agents.team-actions.create-team"),
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
      title: t("agents.team-actions.duplicate-title", { name: team.name }),
      description: t("agents.team-actions.duplicate-description"),
      submitLabel: t("agents.team-actions.create-team"),
      initialValues: {
        name: t("agents.team-actions.copy-name", { name: team.name }),
        description: team.description ?? "",
        personaIds: [...team.personaIds],
      },
    });
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
      title: t("agents.team-actions.edit-team"),
      description: "",
      submitLabel: t("agents.team-actions.save-changes"),
      initialValues: {
        id: team.id,
        name: team.name,
        description: team.description ?? "",
        instructions: team.instructions ?? undefined,
        personaIds: [...team.personaIds],
      },
    });
  }

  function openExportSnapshot(team: AgentTeam) {
    actions.setActionNoticeMessage(null);
    actions.setActionErrorMessage(null);
    setTeamToExport(team);
  }

  function openShare(team: AgentTeam) {
    actions.setActionNoticeMessage(null);
    actions.setActionErrorMessage(null);
    setTeamToShare(team);
  }

  function getTeamCatalogShareLevel(team: AgentTeam): CatalogTeamShareLevel {
    return team.shared ? "none" : "not-shared";
  }

  async function setTeamCatalogShareLevel(
    team: AgentTeam,
    shareLevel: CatalogTeamShareLevel,
  ): Promise<void> {
    if (team.isBuiltin) return;

    actions.setActionNoticeMessage(null);
    actions.setActionErrorMessage(null);
    const shared = shareLevel !== "not-shared";
    try {
      const result = await setCatalogSharedMutation.mutateAsync({
        id: team.id,
        shared,
      });
      // The open dialog holds its own copy of the team, so re-point it at the
      // returned record — otherwise the toggle snaps back to its old value.
      setTeamToShare((current) =>
        current?.id === result.team.id ? result.team : current,
      );
      actions.setActionNoticeMessage(
        teamShareNotice(team.name, shared, result.publicationStatus),
      );
    } catch (error) {
      actions.setActionErrorMessage(
        error instanceof Error
          ? error.message
          : shared
            ? t("agents.team-actions.share-failed")
            : t("agents.team-actions.unshare-failed"),
      );
    }
  }

  /**
   * Add a published team.
   *
   * Only the coordinate is sent; the backend re-verifies the head, so an entry
   * retracted or republished while the dialog sat open fails loudly here
   * rather than copying a stale projection.
   */
  async function handleAddTeamFromCatalog(
    team: CatalogTeam,
    onSuccess?: () => void,
  ): Promise<void> {
    actions.setActionNoticeMessage(null);
    actions.setActionErrorMessage(null);
    try {
      const result = await addTeamFromCatalogMutation.mutateAsync({
        ownerPubkey: team.ownerPubkey,
        teamDTag: team.teamDTag,
        eventId: team.eventId,
      });
      actions.setActionNoticeMessage(
        result.alreadyPresent
          ? t("agents.team-actions.already-added", {
              name: result.team.name,
            })
          : t("agents.team-actions.added", { name: result.team.name }),
      );
      onSuccess?.();
    } catch (error) {
      actions.setActionErrorMessage(
        error instanceof Error
          ? error.message
          : t("agents.team-actions.add-failed"),
      );
    }
  }

  function handleExportTeamSnapshot(
    team: AgentTeam,
    memoryLevel: SnapshotMemoryLevel,
    format: SnapshotFormat,
  ) {
    setTeamToExport(null);
    exportTeamSnapshotMutation.mutate(
      { id: team.id, memoryLevel, format },
      {
        onSuccess: (saved) => {
          if (saved) {
            actions.setActionNoticeMessage(
              t("agents.team-actions.exported", { name: team.name }),
            );
          }
        },
        onError: (error) => {
          actions.setActionErrorMessage(
            error instanceof Error
              ? error.message
              : t("agents.team-actions.export-snapshot-failed"),
          );
        },
      },
    );
  }

  async function handleImportTeamSnapshotFile(
    fileBytes: number[],
    fileName: string,
  ) {
    actions.setActionNoticeMessage(null);
    actions.setActionErrorMessage(null);
    try {
      const preview = await previewTeamSnapshotImportMutation.mutateAsync({
        fileBytes,
        fileName,
      });
      setTeamSnapshotImportState({ fileBytes, fileName, preview });
      setTeamSnapshotImportResult(null);
      setTeamSnapshotImportConfirmError(null);
    } catch (err) {
      actions.setActionErrorMessage(
        err instanceof Error
          ? err.message
          : t("agents.team-actions.read-snapshot-failed"),
      );
    }
  }

  async function handleConfirmTeamSnapshotImport(keepAllowlist: boolean) {
    if (!teamSnapshotImportState) {
      return;
    }
    setTeamSnapshotImportConfirmError(null);
    try {
      const result = await confirmTeamSnapshotImportMutation.mutateAsync({
        fileBytes: teamSnapshotImportState.fileBytes,
        keepAllowlist,
      });
      setTeamSnapshotImportResult(result);
      void queryClient.invalidateQueries({ queryKey: personasQueryKey });
      void queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey });
      void queryClient.invalidateQueries({ queryKey: teamsQueryKey });
      const toast = deriveImportToast(result);
      if (toast.type === "error") {
        actions.setActionErrorMessage(toast.message);
      } else {
        actions.setActionNoticeMessage(toast.message);
      }
    } catch (err) {
      setTeamSnapshotImportConfirmError(
        err instanceof Error
          ? err.message
          : t("agents.team-actions.import-snapshot-failed"),
      );
    }
  }

  function closeTeamSnapshotImportDialog() {
    setTeamSnapshotImportState(null);
    setTeamSnapshotImportResult(null);
    setTeamSnapshotImportConfirmError(null);
  }

  return {
    teams,
    teamsQuery,
    catalogQuery,
    catalogTeams,
    isAddingFromCatalog: addTeamFromCatalogMutation.isPending,
    isCatalogSharePending: setCatalogSharedMutation.isPending,
    createTeamMutation,
    updateTeamMutation,
    deleteTeamMutation,
    teamDialogState,
    setTeamDialogState,
    teamToDelete,
    setTeamToDelete,
    teamToAddToChannel,
    setTeamToAddToChannel,
    teamToExport,
    setTeamToExport,
    teamToShare,
    setTeamToShare,
    teamSnapshotImportState,
    teamSnapshotImportResult,
    teamSnapshotImportConfirmError,
    isTeamSnapshotImportConfirming: confirmTeamSnapshotImportMutation.isPending,
    exportTeamSnapshotMutation,
    handleTeamSubmit,
    handleDeleteRemovedPersonas,
    handleDeleteTeam,
    handleTeamDeployed,
    openCreateDialog,
    openDuplicateDialog,
    openEditDialog,
    openExportSnapshot,
    openShare,
    getTeamCatalogShareLevel,
    setTeamCatalogShareLevel,
    handleAddTeamFromCatalog,
    handleExportTeamSnapshot,
    handleImportTeamSnapshotFile,
    handleConfirmTeamSnapshotImport,
    closeTeamSnapshotImportDialog,
  };
}
