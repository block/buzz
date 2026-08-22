import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import {
  managedAgentsQueryKey,
  personasQueryKey,
  useAcpRuntimesQuery,
  useCreateManagedAgentMutation,
  useCreatePersonaMutation,
  useDeletePersonaMutation,
  useExportAgentSnapshotMutation,
  usePersonasQuery,
  usePreviewAgentSnapshotImportMutation,
  useConfirmAgentSnapshotImportMutation,
  useSetPersonaActiveMutation,
  useUpdatePersonaMutation,
  type AgentSnapshotImportPreview,
  type AgentSnapshotImportResult,
} from "@/features/agents/hooks";
import {
  getLibraryPersonas,
  getPersonaLabelsById,
} from "@/features/agents/lib/catalog";
import {
  type CatalogPersonaShareLevel,
  catalogPersonasFromPublications,
  findLocalPersonaForCatalogEntry,
  isCatalogPersona,
} from "@/features/agents/lib/personaCatalogRelay";
import {
  usePersonaCatalogLiveUpdates,
  usePersonaCatalogQuery,
  useSetPersonaCatalogSharedMutation,
  useUpdatePersonaAndPublishMutation,
} from "@/features/agents/lib/usePersonaCatalogRelay";
import { personaSaveNotice } from "@/features/agents/lib/personaSaveNotice";
import { useCreatedAgentChannelAttachment } from "@/features/agents/useCreatedAgentChannelAttachment";
import { useCommunities } from "@/features/communities/useCommunities";
import { useIdentityQuery } from "@/shared/api/hooks";
import type {
  SnapshotFormat,
  SnapshotMemoryLevel,
} from "@/shared/api/tauriPersonas";
import type {
  AcpRuntime,
  AgentPersona,
  Channel,
  CreatePersonaInput,
  ManagedAgent,
  UpdatePersonaInput,
} from "@/shared/api/types";
import {
  duplicatePersonaDialogState,
  editPersonaDialogState,
  type PersonaDialogState,
} from "./personaDialogState";
import {
  resolveCreateIntent,
  type AgentCreateIntent,
} from "./agentCreateIntent";
import { resolveManagedAgentAvatarUrl } from "./managedAgentAvatar";
import {
  buildInstanceInputForDefinition,
  type BackendIntent,
} from "../lib/instanceInputForDefinition";

type PersonaFeedbackSurface = "catalog" | "library";

export function usePersonaActions() {
  const queryClient = useQueryClient();
  const { activeCommunity } = useCommunities();
  const identityQuery = useIdentityQuery();
  const communityId = activeCommunity?.id ?? null;
  const personasQuery = usePersonasQuery();
  const catalogQuery = usePersonaCatalogQuery(communityId);
  usePersonaCatalogLiveUpdates(communityId);
  const setCatalogSharedMutation =
    useSetPersonaCatalogSharedMutation(communityId);
  const [shouldLoadAcpRuntimes, setShouldLoadAcpRuntimes] =
    React.useState(false);
  const acpRuntimesQuery = useAcpRuntimesQuery({
    enabled: shouldLoadAcpRuntimes,
  });
  const createAgentMutation = useCreateManagedAgentMutation();
  const createPersonaMutation = useCreatePersonaMutation();
  const updatePersonaMutation = useUpdatePersonaMutation();
  const updatePersonaAndPublishMutation =
    useUpdatePersonaAndPublishMutation(communityId);
  const deletePersonaMutation = useDeletePersonaMutation();
  const setPersonaActiveMutation = useSetPersonaActiveMutation();
  const exportAgentSnapshotMutation = useExportAgentSnapshotMutation();
  const previewSnapshotImportMutation = usePreviewAgentSnapshotImportMutation();
  const confirmSnapshotImportMutation = useConfirmAgentSnapshotImportMutation();

  const [personaDialogState, setPersonaDialogState] =
    React.useState<PersonaDialogState | null>(null);
  const [personaToDelete, setPersonaToDelete] =
    React.useState<AgentPersona | null>(null);
  const [personaToShare, setPersonaToShare] = React.useState<{
    persona: AgentPersona;
    linkedAgentPubkey: string | null;
    effectiveAvatarUrl: string | null;
  } | null>(null);
  const [personaToExportSnapshot, setPersonaToExportSnapshot] = React.useState<{
    persona: AgentPersona;
    linkedAgentPubkey: string | null;
    effectiveAvatarUrl: string | null;
  } | null>(null);
  const [snapshotImportState, setSnapshotImportState] = React.useState<{
    fileBytes: number[];
    fileName: string;
    preview: AgentSnapshotImportPreview;
  } | null>(null);
  const [snapshotImportResult, setSnapshotImportResult] =
    React.useState<AgentSnapshotImportResult | null>(null);
  const [snapshotImportConfirmError, setSnapshotImportConfirmError] =
    React.useState<string | null>(null);
  const [isCatalogDialogOpen, setIsCatalogDialogOpen] = React.useState(false);
  const [personaNoticeMessage, setPersonaNoticeMessage] = React.useState<
    string | null
  >(null);
  const [personaErrorMessage, setPersonaErrorMessage] = React.useState<
    string | null
  >(null);
  const [personaFeedbackSurface, setPersonaFeedbackSurface] =
    React.useState<PersonaFeedbackSurface>("library");
  const createdAgentAttachment = useCreatedAgentChannelAttachment();
  const [isPersonaSubmitPending, setIsPersonaSubmitPending] =
    React.useState(false);

  const personas = personasQuery.data ?? [];
  const publications = catalogQuery.data ?? [];
  const sharedCatalogPersonaIdSet = React.useMemo(() => {
    const currentPubkey = identityQuery.data?.pubkey.toLowerCase();
    return new Set(
      publications
        .filter((publication) => publication.ownerPubkey === currentPubkey)
        .map((publication) => publication.sourcePersonaId),
    );
  }, [identityQuery.data?.pubkey, publications]);
  const availableRuntimes = React.useMemo(
    () =>
      (acpRuntimesQuery.data ?? []).filter(
        (runtime): runtime is AcpRuntime =>
          runtime.availability === "available",
      ),
    [acpRuntimesQuery.data],
  );
  const catalogPersonas = React.useMemo(
    () =>
      catalogPersonasFromPublications(
        publications,
        personas,
        identityQuery.data?.pubkey,
      ),
    [identityQuery.data?.pubkey, personas, publications],
  );
  const libraryPersonas = React.useMemo(
    () => getLibraryPersonas(personas),
    [personas],
  );
  const personaLabelsById = React.useMemo(
    () => getPersonaLabelsById(personas),
    [personas],
  );

  function clearFeedback(
    surface: PersonaFeedbackSurface = personaFeedbackSurface,
  ) {
    setPersonaFeedbackSurface(surface);
    setPersonaNoticeMessage(null);
    setPersonaErrorMessage(null);
  }

  async function handleSubmit(
    input: CreatePersonaInput | UpdatePersonaInput,
    intent?: AgentCreateIntent,
    backendIntent?: BackendIntent | null,
    targetChannel?: Pick<Channel, "id" | "name"> | null,
    options?: { publishCatalogUpdates?: boolean },
  ): Promise<boolean> {
    if (isPersonaSubmitPending) {
      return false;
    }

    clearFeedback("library");
    setIsPersonaSubmitPending(true);
    try {
      if ("id" in input) {
        // "Save and publish" promises the community catalog sees this edit, so
        // it must use the command that awaits the relay. A plain save only
        // enqueues the head and cannot report the outcome.
        if (options?.publishCatalogUpdates) {
          const result =
            await updatePersonaAndPublishMutation.mutateAsync(input);
          if (result.publicationStatus === "queued" && result.relayMessage) {
            console.warn(
              `[updatePersonaAndPublish] relay publication queued: ${result.relayMessage}`,
            );
          }
          setPersonaNoticeMessage(
            personaSaveNotice(input.displayName, result.publicationStatus),
          );
        } else {
          await updatePersonaMutation.mutateAsync(input);
          setPersonaNoticeMessage(personaSaveNotice(input.displayName, null));
        }
      } else {
        const runtime = availableRuntimes.find(
          (candidate) => candidate.id === input.runtime,
        );
        if (!runtime) {
          setPersonaErrorMessage(
            "Choose an available provider for this agent.",
          );
          return false;
        }

        // Stale-intent guard: a definition-only create never carries one.
        const startIntent =
          resolveCreateIntent(intent) === "definition_start"
            ? (backendIntent ?? null)
            : null;

        const avatarUrl = await resolveManagedAgentAvatarUrl(
          input.avatarUrl,
          undefined,
          runtime.avatarUrl,
        );
        const persona = await createPersonaMutation.mutateAsync({
          ...input,
          avatarUrl,
        });

        if (resolveCreateIntent(intent) === "definition") {
          setPersonaNoticeMessage(`Created ${persona.displayName}.`);
          setPersonaDialogState(null);
          return true;
        }
        const agentInput = await buildInstanceInputForDefinition(
          persona,
          runtime,
          undefined,
          startIntent ?? undefined,
        );

        try {
          const created = await createAgentMutation.mutateAsync(agentInput);
          await createdAgentAttachment.presentCreatedAgent(
            created,
            targetChannel,
          );
          if (created.spawnError) {
            setPersonaErrorMessage(
              `${persona.displayName} was created, but it did not start: ${created.spawnError}`,
            );
          }
          if (created.profileSyncError) {
            setPersonaErrorMessage(
              `${created.agent.name} was created, but profile sync failed: ${created.profileSyncError}`,
            );
          }
        } catch (error) {
          setPersonaErrorMessage(
            error instanceof Error
              ? `${persona.displayName} was created, but the agent instance could not be created: ${error.message}`
              : `${persona.displayName} was created, but the agent instance could not be created.`,
          );
        }
      }
      setPersonaDialogState(null);
      return true;
    } catch (error) {
      setPersonaErrorMessage(
        error instanceof Error ? error.message : "Failed to save agent.",
      );
      return false;
    } finally {
      setIsPersonaSubmitPending(false);
    }
  }

  async function handleDelete(persona: AgentPersona) {
    clearFeedback("library");
    try {
      await deletePersonaMutation.mutateAsync(persona.id);
      setPersonaNoticeMessage(`Deleted ${persona.displayName}.`);
      setPersonaToDelete(null);
    } catch (error) {
      setPersonaErrorMessage(
        error instanceof Error ? error.message : "Failed to delete agent.",
      );
    }
  }

  async function handleSetActive(
    persona: AgentPersona,
    active: boolean,
    surface: PersonaFeedbackSurface,
  ): Promise<AgentPersona | null> {
    clearFeedback(surface);
    try {
      let updatedPersona: AgentPersona;
      if (active && isCatalogPersona(persona)) {
        const localPersona = findLocalPersonaForCatalogEntry(
          personas,
          persona.catalogSource,
        );

        if (localPersona) {
          if (!localPersona.isActive) {
            updatedPersona = await setPersonaActiveMutation.mutateAsync({
              id: localPersona.id,
              active: true,
            });
          } else {
            updatedPersona = localPersona;
          }
        } else {
          updatedPersona = await createPersonaMutation.mutateAsync({
            displayName: persona.displayName,
            avatarUrl: persona.avatarUrl ?? undefined,
            systemPrompt: persona.systemPrompt,
            runtime: persona.runtime ?? undefined,
            model: persona.model ?? undefined,
            provider: persona.provider ?? undefined,
            namePool: persona.namePool,
            behavior: {
              respondTo:
                persona.respondTo === "anyone" ? "anyone" : "owner-only",
              parallelism: persona.parallelism ?? undefined,
            },
            // Provenance on the copy: without it the copy's fresh local id is
            // the only identifier, and the catalog offers "Add" again.
            catalogSource: persona.catalogSource.isOwn
              ? undefined
              : {
                  ownerPubkey: persona.catalogSource.ownerPubkey,
                  personaId: persona.catalogSource.personaId,
                },
          });
        }
      } else {
        updatedPersona = await setPersonaActiveMutation.mutateAsync({
          id: persona.id,
          active,
        });
      }
      setPersonaNoticeMessage(
        active
          ? `Selected ${persona.displayName} for My Agents.`
          : `Deselected ${persona.displayName} from My Agents.`,
      );
      return updatedPersona;
    } catch (error) {
      setPersonaErrorMessage(
        error instanceof Error
          ? error.message
          : active
            ? "Failed to select agent for My Agents."
            : "Failed to deselect agent from My Agents.",
      );
      return null;
    }
  }

  async function handleImportSnapshotFile(
    fileBytes: number[],
    fileName: string,
  ) {
    clearFeedback("library");
    try {
      const preview = await previewSnapshotImportMutation.mutateAsync({
        fileBytes,
        fileName,
      });
      setSnapshotImportState({ fileBytes, fileName, preview });
      setSnapshotImportResult(null);
      setSnapshotImportConfirmError(null);
    } catch (err) {
      setPersonaErrorMessage(
        err instanceof Error
          ? err.message
          : "Failed to read agent snapshot file.",
      );
    }
  }

  async function handleConfirmSnapshotImport(keepAllowlist: boolean) {
    if (!snapshotImportState) {
      return;
    }
    setSnapshotImportConfirmError(null);
    try {
      const result = await confirmSnapshotImportMutation.mutateAsync({
        fileBytes: snapshotImportState.fileBytes,
        keepAllowlist,
      });
      setSnapshotImportResult(result);
      void queryClient.invalidateQueries({ queryKey: personasQueryKey });
      void queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey });
      void queryClient.invalidateQueries({
        queryKey: ["user-profile", result.newPubkey.toLowerCase()],
      });
      if (result.memoryErrors.length > 0) {
        setPersonaErrorMessage(
          `${result.displayName} imported, but ${result.memoryErrors.length} memory entr${result.memoryErrors.length === 1 ? "y" : "ies"} failed to restore.`,
        );
      } else {
        setPersonaNoticeMessage(`Imported ${result.displayName}.`);
      }
    } catch (err) {
      setSnapshotImportConfirmError(
        err instanceof Error ? err.message : "Failed to import agent snapshot.",
      );
    }
  }

  function closeSnapshotImportDialog() {
    setSnapshotImportState(null);
    setSnapshotImportResult(null);
    setSnapshotImportConfirmError(null);
  }

  function prepareCreate() {
    clearFeedback("library");
    setShouldLoadAcpRuntimes(true);
  }

  function openEdit(persona: AgentPersona) {
    clearFeedback("library");
    setShouldLoadAcpRuntimes(true);
    setPersonaDialogState(editPersonaDialogState(persona));
  }

  function openDuplicate(persona: AgentPersona) {
    clearFeedback("library");
    setShouldLoadAcpRuntimes(true);
    setPersonaDialogState(duplicatePersonaDialogState(persona));
  }

  function openCatalog() {
    clearFeedback("catalog");
    void catalogQuery.refetch();
    setIsCatalogDialogOpen(true);
  }

  function openDelete(persona: AgentPersona) {
    clearFeedback("library");
    setPersonaToDelete(persona);
  }

  function openShare(
    persona: AgentPersona,
    linkedAgent: ManagedAgent | undefined,
    effectiveAvatarUrl: string | null,
  ) {
    clearFeedback("library");
    setPersonaToShare({
      persona,
      linkedAgentPubkey: linkedAgent?.pubkey ?? null,
      effectiveAvatarUrl,
    });
  }

  function handleExportSnapshot(
    persona: AgentPersona,
    linkedAgentPubkey: string | null,
    effectiveAvatarUrl: string | null,
    memoryLevel: SnapshotMemoryLevel,
    format: SnapshotFormat,
  ) {
    clearFeedback("library");
    setPersonaToExportSnapshot(null);
    exportAgentSnapshotMutation.mutate(
      {
        id: persona.id,
        memoryLevel,
        format,
        memorySourcePubkey: linkedAgentPubkey,
        avatarUrl: effectiveAvatarUrl,
      },
      {
        onSuccess: (saved) => {
          if (saved) {
            setPersonaNoticeMessage(`Exported ${persona.displayName}.`);
          }
        },
        onError: (error) => {
          setPersonaErrorMessage(
            error instanceof Error
              ? error.message
              : "Failed to export agent snapshot.",
          );
        },
      },
    );
  }

  function getPersonaCatalogShareLevel(
    persona: AgentPersona,
  ): CatalogPersonaShareLevel {
    return persona.shared ? "none" : "not-shared";
  }

  async function setPersonaCatalogShareLevel(
    persona: AgentPersona,
    shareLevel: CatalogPersonaShareLevel,
  ): Promise<void> {
    if (persona.isBuiltIn) return;

    clearFeedback("library");
    try {
      const shared = shareLevel !== "not-shared";
      const result = await setCatalogSharedMutation.mutateAsync({
        id: persona.id,
        shared,
      });
      setPersonaToShare((current) =>
        current?.persona.id === result.persona.id
          ? { ...current, persona: result.persona }
          : current,
      );
      if (result.publicationStatus === "queued") {
        if (shared) {
          setPersonaNoticeMessage(
            `Sharing ${persona.displayName} is queued. It will appear after the relay accepts the update.`,
          );
        } else {
          setPersonaNoticeMessage(
            `Removing ${persona.displayName} is queued. It may remain discoverable until the relay accepts the update.`,
          );
        }
        if (result.relayMessage) {
          console.warn(
            `[setPersonaShared] relay publication queued: ${result.relayMessage}`,
          );
        }
      } else if (!shared) {
        setPersonaNoticeMessage(
          `${persona.displayName} is no longer discoverable in the community catalog.`,
        );
      } else {
        setPersonaNoticeMessage(
          `Published ${persona.displayName} to the community catalog.`,
        );
      }
    } catch (error) {
      setPersonaErrorMessage(
        error instanceof Error
          ? error.message
          : "Failed to update catalog sharing.",
      );
    }
  }

  const isPending =
    isPersonaSubmitPending ||
    createPersonaMutation.isPending ||
    createAgentMutation.isPending ||
    updatePersonaMutation.isPending ||
    updatePersonaAndPublishMutation.isPending ||
    deletePersonaMutation.isPending ||
    setPersonaActiveMutation.isPending ||
    exportAgentSnapshotMutation.isPending ||
    previewSnapshotImportMutation.isPending ||
    confirmSnapshotImportMutation.isPending ||
    setCatalogSharedMutation.isPending;

  return {
    personasQuery,
    catalogQuery,
    acpRuntimesQuery,
    createPersonaMutation,
    updatePersonaMutation,
    updatePersonaAndPublishMutation,
    setPersonaActiveMutation,
    catalogPersonas,
    libraryPersonas,
    personaLabelsById,
    isPending,
    personaDialogState,
    setPersonaDialogState,
    personaToDelete,
    setPersonaToDelete,
    personaToShare,
    setPersonaToShare,
    isCatalogDialogOpen,
    setIsCatalogDialogOpen,
    personaNoticeMessage,
    personaErrorMessage,
    personaFeedbackSurface,
    ...createdAgentAttachment,
    handleSubmit,
    handleDelete,
    handleSetActive,
    prepareCreate,
    openEdit,
    openDuplicate,
    openCatalog,
    openDelete,
    openShare,
    personaToExportSnapshot,
    setPersonaToExportSnapshot,
    handleExportSnapshot,
    getPersonaCatalogShareLevel,
    setPersonaCatalogShareLevel,
    sharedCatalogPersonaIdSet,
    clearFeedback,
    snapshotImportState,
    snapshotImportResult,
    snapshotImportConfirmError,
    isSnapshotImportConfirming: confirmSnapshotImportMutation.isPending,
    handleImportSnapshotFile,
    handleConfirmSnapshotImport,
    closeSnapshotImportDialog,
  };
}
