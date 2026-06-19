import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import {
  personasQueryKey,
  useAcpRuntimesQuery,
  useCreateManagedAgentMutation,
  useCreatePersonaMutation,
  useDeletePersonaMutation,
  useExportPersonaJsonMutation,
  usePersonasQuery,
  useSetPersonaActiveMutation,
  useUpdatePersonaMutation,
} from "@/features/agents/hooks";
import { getPersonaLibraryState } from "@/features/agents/lib/catalog";
import {
  parsePersonaFiles,
  type ParsePersonaFilesResult,
} from "@/shared/api/tauriPersonas";
import { isSingleItemFile } from "@/shared/lib/fileMagic";
import type {
  AcpRuntime,
  AgentPersona,
  CreateManagedAgentInput,
  CreatePersonaInput,
  UpdatePersonaInput,
} from "@/shared/api/types";
import {
  createPersonaDialogState,
  duplicatePersonaDialogState,
  editPersonaDialogState,
  importPersonaDialogState,
  type PersonaDialogState,
} from "./personaDialogState";
import { usePersonaImportActions } from "./usePersonaImportActions";

type PersonaFeedbackSurface = "catalog" | "library";

export function usePersonaActions() {
  const queryClient = useQueryClient();
  const personasQuery = usePersonasQuery();
  const acpRuntimesQuery = useAcpRuntimesQuery();
  const createAgentMutation = useCreateManagedAgentMutation();
  const createPersonaMutation = useCreatePersonaMutation();
  const updatePersonaMutation = useUpdatePersonaMutation();
  const deletePersonaMutation = useDeletePersonaMutation();
  const setPersonaActiveMutation = useSetPersonaActiveMutation();
  const exportPersonaJsonMutation = useExportPersonaJsonMutation();

  const [personaDialogState, setPersonaDialogState] =
    React.useState<PersonaDialogState | null>(null);
  const [personaToDelete, setPersonaToDelete] =
    React.useState<AgentPersona | null>(null);
  const [isCatalogDialogOpen, setIsCatalogDialogOpen] = React.useState(false);
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

  const personas = personasQuery.data ?? [];
  const availableRuntimes = React.useMemo(
    () =>
      (acpRuntimesQuery.data ?? []).filter(
        (runtime): runtime is AcpRuntime =>
          runtime.availability === "available",
      ),
    [acpRuntimesQuery.data],
  );
  const { catalogPersonas, libraryPersonas, personaLabelsById } = React.useMemo(
    () => getPersonaLibraryState(personas),
    [personas],
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

  async function handleSubmit(input: CreatePersonaInput | UpdatePersonaInput) {
    clearFeedback("library");
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
          return;
        }

        const persona = await createPersonaMutation.mutateAsync(input);
        const agentInput: CreateManagedAgentInput = {
          name: persona.displayName,
          acpCommand: "buzz-acp",
          agentCommand: runtime.command,
          agentArgs: runtime.defaultArgs,
          mcpCommand: runtime.mcpCommand ?? "",
          personaId: persona.id,
          systemPrompt: persona.systemPrompt,
          avatarUrl: persona.avatarUrl ?? undefined,
          model: persona.model ?? undefined,
          envVars: persona.envVars,
          spawnAfterCreate: true,
          startOnAppLaunch: true,
          backend: { type: "local" },
        };

        try {
          const created = await createAgentMutation.mutateAsync(agentInput);
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
    } catch (error) {
      setPersonaErrorMessage(
        error instanceof Error ? error.message : "Failed to save persona.",
      );
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
        error instanceof Error ? error.message : "Failed to delete persona.",
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
            ? "Failed to select persona for My Agents."
            : "Failed to deselect persona from My Agents.",
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
        setPersonaDialogState(importPersonaDialogState(result.personas[0]));
      } else if (result.personas.length > 0) {
        setBatchImportResult(result);
        setBatchImportFileName(fileName);
      } else {
        setPersonaErrorMessage("No valid personas found in file.");
      }
    } catch (err) {
      setPersonaErrorMessage(
        err instanceof Error ? err.message : "Failed to parse persona file.",
      );
    }
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
          error instanceof Error ? error.message : "Failed to export persona.",
        );
      },
    });
  }

  function handleBatchImportComplete(count: number) {
    clearFeedback("library");
    setBatchImportResult(null);
    setPersonaNoticeMessage(
      `Imported ${count} persona${count !== 1 ? "s" : ""}.`,
    );
    void queryClient.invalidateQueries({ queryKey: personasQueryKey });
  }

  function openCreate() {
    clearFeedback("library");
    setPersonaDialogState(createPersonaDialogState());
  }

  function openEdit(persona: AgentPersona) {
    clearFeedback("library");
    setPersonaDialogState(editPersonaDialogState(persona));
  }

  function openDuplicate(persona: AgentPersona) {
    clearFeedback("library");
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

  const isPending =
    createPersonaMutation.isPending ||
    createAgentMutation.isPending ||
    updatePersonaMutation.isPending ||
    deletePersonaMutation.isPending ||
    setPersonaActiveMutation.isPending ||
    exportPersonaJsonMutation.isPending;

  return {
    // Queries
    personasQuery,
    acpRuntimesQuery,
    // Mutations (for error/pending access)
    createPersonaMutation,
    updatePersonaMutation,
    setPersonaActiveMutation,
    // Derived state
    catalogPersonas,
    libraryPersonas,
    personaLabelsById,
    isPending,
    // UI state
    personaDialogState,
    setPersonaDialogState,
    personaToDelete,
    setPersonaToDelete,
    isCatalogDialogOpen,
    setIsCatalogDialogOpen,
    batchImportResult,
    setBatchImportResult,
    batchImportFileName,
    personaNoticeMessage,
    personaErrorMessage,
    personaFeedbackSurface,
    // Import actions (composed)
    personaImportActions,
    // Handlers
    handleSubmit,
    handleDelete,
    handleSetActive,
    handleImportFile,
    handleExport,
    handleBatchImportComplete,
    openCreate,
    openEdit,
    openDuplicate,
    openCatalog,
    openDelete,
    clearFeedback,
  };
}
