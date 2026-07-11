import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import {
  personasQueryKey,
  useAcpRuntimesQuery,
  useCreateManagedAgentMutation,
  useCreatePersonaMutation,
  useDeletePersonaMutation,
  useExportAgentSnapshotMutation,
  useExportPersonaJsonMutation,
  usePersonasQuery,
  usePreviewAgentSnapshotImportMutation,
  useConfirmAgentSnapshotImportMutation,
  useSetPersonaActiveMutation,
  useUpdatePersonaMutation,
  type AgentSnapshotImportPreview,
  type AgentSnapshotImportResult,
} from "@/features/agents/hooks";
import { getPersonaLibraryState } from "@/features/agents/lib/catalog";
import {
  parsePersonaFiles,
  type ParsePersonaFilesResult,
  type SnapshotFormat,
  type SnapshotMemoryLevel,
} from "@/shared/api/tauriPersonas";
import { isSingleItemFile } from "@/shared/lib/fileMagic";
import type {
  AcpRuntime,
  AgentPersona,
  CreateManagedAgentResponse,
  CreatePersonaInput,
  ManagedAgent,
  UpdatePersonaInput,
} from "@/shared/api/types";
import {
  duplicatePersonaDialogState,
  editPersonaDialogState,
  importPersonaDialogState,
  type PersonaDialogState,
} from "./personaDialogState";
import {
  resolveCreateIntent,
  type AgentCreateIntent,
} from "./agentCreateIntent";
import { resolveManagedAgentAvatarUrl } from "./managedAgentAvatar";
import {
  buildInstanceInputForDefinition,
  mintDefinitionWithPreflight,
  type BackendIntent,
} from "../lib/instanceInputForDefinition";
import { meshPrepareRelayMeshClient } from "@/shared/api/tauriMesh";
import { usePersonaImportActions } from "./usePersonaImportActions";

type PersonaFeedbackSurface = "catalog" | "library";

const PERSONA_CATALOG_VISIBILITY_STORAGE_KEY =
  "buzz-persona-catalog-visibility-v1";

function readSharedCatalogPersonaIds(): string[] {
  if (typeof window === "undefined") {
    return [];
  }

  try {
    const raw = window.localStorage.getItem(
      PERSONA_CATALOG_VISIBILITY_STORAGE_KEY,
    );
    if (!raw) {
      return [];
    }

    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) {
      return [];
    }

    return parsed.filter((id): id is string => typeof id === "string");
  } catch {
    return [];
  }
}

function writeSharedCatalogPersonaIds(ids: string[]) {
  if (typeof window === "undefined") {
    return;
  }

  try {
    window.localStorage.setItem(
      PERSONA_CATALOG_VISIBILITY_STORAGE_KEY,
      JSON.stringify(ids),
    );
  } catch {
    // Catalog visibility is a local convenience setting; ignore storage failures.
  }
}

export function usePersonaActions() {
  const queryClient = useQueryClient();
  const personasQuery = usePersonasQuery();
  const [shouldLoadAcpRuntimes, setShouldLoadAcpRuntimes] =
    React.useState(false);
  const acpRuntimesQuery = useAcpRuntimesQuery({
    enabled: shouldLoadAcpRuntimes,
  });
  const createAgentMutation = useCreateManagedAgentMutation();
  const createPersonaMutation = useCreatePersonaMutation();
  const updatePersonaMutation = useUpdatePersonaMutation();
  const deletePersonaMutation = useDeletePersonaMutation();
  const setPersonaActiveMutation = useSetPersonaActiveMutation();
  const exportPersonaJsonMutation = useExportPersonaJsonMutation();
  const exportAgentSnapshotMutation = useExportAgentSnapshotMutation();
  const previewSnapshotImportMutation = usePreviewAgentSnapshotImportMutation();
  const confirmSnapshotImportMutation = useConfirmAgentSnapshotImportMutation();

  const [personaDialogState, setPersonaDialogState] =
    React.useState<PersonaDialogState | null>(null);
  const [personaToDelete, setPersonaToDelete] =
    React.useState<AgentPersona | null>(null);
  const [personaToShare, setPersonaToShare] =
    React.useState<AgentPersona | null>(null);
  const [personaToExportSnapshot, setPersonaToExportSnapshot] = React.useState<{
    persona: AgentPersona;
    linkedAgentPubkey: string | null;
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
  const [sharedCatalogPersonaIds, setSharedCatalogPersonaIds] = React.useState<
    string[]
  >(readSharedCatalogPersonaIds);
  const [batchImportResult, setBatchImportResult] =
    React.useState<ParsePersonaFilesResult | null>(null);
  const [batchImportFileName, setBatchImportFileName] = React.useState("");
  const [personaNoticeMessage, setPersonaNoticeMessage] = React.useState<
    string | null
  >(null);
  const [personaErrorMessage, setPersonaErrorMessage] = React.useState<
    string | null
  >(null);
  const [personaFeedbackSurface, setPersonaFeedbackSurface] =
    React.useState<PersonaFeedbackSurface>("library");
  const [createdAgent, setCreatedAgent] =
    React.useState<CreateManagedAgentResponse | null>(null);
  const [isPersonaSubmitPending, setIsPersonaSubmitPending] =
    React.useState(false);

  const personas = personasQuery.data ?? [];
  const sharedCatalogPersonaIdSet = React.useMemo(
    () => new Set(sharedCatalogPersonaIds),
    [sharedCatalogPersonaIds],
  );
  const availableRuntimes = React.useMemo(
    () =>
      (acpRuntimesQuery.data ?? []).filter(
        (runtime): runtime is AcpRuntime =>
          runtime.availability === "available",
      ),
    [acpRuntimesQuery.data],
  );
  const { catalogPersonas, libraryPersonas, personaLabelsById } = React.useMemo(
    () => getPersonaLibraryState(personas, sharedCatalogPersonaIdSet),
    [personas, sharedCatalogPersonaIdSet],
  );

  const personaImportActions = usePersonaImportActions(personas, {
    clearPersonaFeedback: () => clearFeedback("library"),
    setPersonaNoticeMessage,
    setPersonaErrorMessage,
    setPersonaDialogState,
  });

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
  ): Promise<boolean> {
    if (isPersonaSubmitPending) {
      return false;
    }

    clearFeedback("library");
    setIsPersonaSubmitPending(true);
    try {
      if ("id" in input) {
        await updatePersonaMutation.mutateAsync(input);
        setPersonaNoticeMessage(`Updated ${input.displayName}.`);
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
        const persona = await mintDefinitionWithPreflight(
          startIntent,
          meshPrepareRelayMeshClient,
          () =>
            createPersonaMutation.mutateAsync({
              ...input,
              avatarUrl,
            }),
        );

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
          setCreatedAgent(created);
          if (created.spawnError) {
            setPersonaErrorMessage(
              `${persona.displayName} was created, but it did not start: ${created.spawnError}`,
            );
          } else {
            setPersonaNoticeMessage(
              `Created and started ${created.agent.name}.`,
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
  ) {
    clearFeedback(surface);
    try {
      await setPersonaActiveMutation.mutateAsync({ id: persona.id, active });
      setPersonaNoticeMessage(
        active
          ? `Selected ${persona.displayName} for My Agents.`
          : `Deselected ${persona.displayName} from My Agents.`,
      );
    } catch (error) {
      setPersonaErrorMessage(
        error instanceof Error
          ? error.message
          : active
            ? "Failed to select agent for My Agents."
            : "Failed to deselect agent from My Agents.",
      );
    }
  }

  async function handleImportFile(fileBytes: number[], fileName: string) {
    clearFeedback("library");
    try {
      const result = await parsePersonaFiles(fileBytes, fileName);
      if (
        isSingleItemFile(fileBytes, fileName) &&
        result.personas.length === 1
      ) {
        setShouldLoadAcpRuntimes(true);
        setPersonaDialogState(importPersonaDialogState(result.personas[0]));
      } else if (result.personas.length > 0) {
        setBatchImportResult(result);
        setBatchImportFileName(fileName);
      } else {
        setPersonaErrorMessage("No valid agents found in file.");
      }
    } catch (err) {
      setPersonaErrorMessage(
        err instanceof Error ? err.message : "Failed to parse agent file.",
      );
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
        fileName: snapshotImportState.fileName,
        keepAllowlist,
      });
      setSnapshotImportResult(result);
      void queryClient.invalidateQueries({ queryKey: personasQueryKey });
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

  function handleExport(persona: AgentPersona) {
    clearFeedback("library");
    exportPersonaJsonMutation.mutate(persona.id, {
      onSuccess: (saved) => {
        if (saved) {
          setPersonaNoticeMessage(`Exported ${persona.displayName}.`);
        }
      },
      onError: (error) => {
        setPersonaErrorMessage(
          error instanceof Error ? error.message : "Failed to export agent.",
        );
      },
    });
  }

  function handleBatchImportComplete(count: number) {
    clearFeedback("library");
    setBatchImportResult(null);
    setPersonaNoticeMessage(
      `Imported ${count} agent${count !== 1 ? "s" : ""}.`,
    );
    void queryClient.invalidateQueries({ queryKey: personasQueryKey });
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
    setIsCatalogDialogOpen(true);
  }

  function openDelete(persona: AgentPersona) {
    clearFeedback("library");
    setPersonaToDelete(persona);
  }

  function openShare(persona: AgentPersona) {
    clearFeedback("library");
    setPersonaToShare(persona);
  }

  function openExportSnapshot(
    persona: AgentPersona,
    linkedAgent: ManagedAgent | undefined,
  ) {
    clearFeedback("library");
    setPersonaToExportSnapshot({
      persona,
      linkedAgentPubkey: linkedAgent?.pubkey ?? null,
    });
  }

  function handleExportSnapshot(
    persona: AgentPersona,
    linkedAgentPubkey: string | null,
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

  function setPersonaCatalogVisibility(
    persona: AgentPersona,
    visible: boolean,
  ) {
    if (persona.isBuiltIn) {
      return;
    }

    clearFeedback("library");
    setSharedCatalogPersonaIds((current) => {
      const next = new Set(current);
      if (visible) {
        next.add(persona.id);
      } else {
        next.delete(persona.id);
      }

      const ids = Array.from(next);
      writeSharedCatalogPersonaIds(ids);
      return ids;
    });
  }

  const isPending =
    isPersonaSubmitPending ||
    createPersonaMutation.isPending ||
    createAgentMutation.isPending ||
    updatePersonaMutation.isPending ||
    deletePersonaMutation.isPending ||
    setPersonaActiveMutation.isPending ||
    exportPersonaJsonMutation.isPending ||
    exportAgentSnapshotMutation.isPending ||
    previewSnapshotImportMutation.isPending ||
    confirmSnapshotImportMutation.isPending;

  return {
    personasQuery,
    acpRuntimesQuery,
    createPersonaMutation,
    updatePersonaMutation,
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
    batchImportResult,
    setBatchImportResult,
    batchImportFileName,
    personaNoticeMessage,
    personaErrorMessage,
    personaFeedbackSurface,
    createdAgent,
    setCreatedAgent,
    personaImportActions,
    handleSubmit,
    handleDelete,
    handleSetActive,
    handleImportFile,
    handleExport,
    handleBatchImportComplete,
    prepareCreate,
    openEdit,
    openDuplicate,
    openCatalog,
    openDelete,
    openShare,
    openExportSnapshot,
    personaToExportSnapshot,
    setPersonaToExportSnapshot,
    handleExportSnapshot,
    setPersonaCatalogVisibility,
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
