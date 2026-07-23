import * as React from "react";
import { OctagonX, Settings2 } from "lucide-react";
import {
  consumePendingSnapshotImport,
  subscribeSnapshotImport,
} from "@/features/agents/openSnapshotImportFromUrlEvent";
import { AddAgentToChannelDialog } from "./AddAgentToChannelDialog";
import { AddTeamToChannelDialog } from "./AddTeamToChannelDialog";
import { AgentDefaultsDialog } from "./AgentDefaultsDialog";
import { AgentDialog } from "./AgentDialog";
import { PersonaCatalogDialog } from "./PersonaCatalogDialog";
import { PersonaDeleteDialog } from "./PersonaDeleteDialog";
import { PersonaShareDialog } from "./PersonaShareDialog";
import { AgentSnapshotExportDialog } from "./AgentSnapshotExportDialog";
import { AgentSnapshotImportDialog } from "./AgentSnapshotImportDialog";
import { TeamSnapshotExportDialog } from "./TeamSnapshotExportDialog";
import { TeamSnapshotImportDialog } from "./TeamSnapshotImportDialog";
import { TeamShareDialog } from "./TeamShareDialog";
import { RelayDirectorySection } from "./RelayDirectorySection";
import { SecretRevealDialog } from "./SecretRevealDialog";
import { TeamDeleteDialog } from "./TeamDeleteDialog";
import { TeamDialog } from "./TeamDialog";
import { TeamsSection } from "./TeamsSection";
import {
  AGENT_CARD_GRID_COLUMNS_CLASS,
  UnifiedAgentsSection,
} from "./UnifiedAgentsSection";
import { useManagedAgentActions } from "./useManagedAgentActions";
import { usePersonaActions } from "./usePersonaActions";
import { useTeamActions } from "./useTeamActions";
import { useProfilePanel } from "@/shared/context/ProfilePanelContext";
import { useBakedBuildEnvQuery } from "@/features/agents/hooks";
import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import { useGlobalAgentConfig } from "@/features/agents/useGlobalAgentConfig";
import { Button } from "@/shared/ui/button";
import { PageHeader } from "@/shared/ui/PageHeader";
import { getInheritedAgentDefaults } from "./bakedEnvHelpers";

export function AgentsView() {
  const { openPersonaProfilePanel, openProfilePanel } = useProfilePanel();
  const { globalConfig } = useGlobalAgentConfig();
  const { data: bakedEnv } = useBakedBuildEnvQuery({ enabled: true });
  const inheritedDefaults = getInheritedAgentDefaults(globalConfig, bakedEnv);
  const agents = useManagedAgentActions();
  const personas = usePersonaActions();
  const teamImportInputRef = React.useRef<HTMLInputElement | null>(null);
  const aiDefaultsTriggerRef = React.useRef<HTMLButtonElement>(null);
  const [isAiDefaultsOpen, setIsAiDefaultsOpen] = React.useState(false);
  // Exclusivity: create never sets `personaDialogState` (edit/dup/import do),
  // so the create-mode and definition-edit AgentDialog mounts never coexist.
  const [isCreateDialogOpen, setIsCreateDialogOpen] = React.useState(false);

  function openUnifiedCreate() {
    personas.prepareCreate();
    setIsCreateDialogOpen(true);
  }
  const teamActions = useTeamActions(
    {
      setActionNoticeMessage: agents.setActionNoticeMessage,
      setActionErrorMessage: agents.setActionErrorMessage,
    },
    {
      refetchManagedAgents: agents.refetchManagedAgents,
      refetchRelayAgents: agents.refetchRelayAgents,
    },
  );

  const isActionPending =
    agents.isPending ||
    personas.isPending ||
    teamActions.createTeamMutation.isPending ||
    teamActions.updateTeamMutation.isPending ||
    teamActions.deleteTeamMutation.isPending;
  const runningAgentCount = agents.managedAgents.filter((agent) =>
    isManagedAgentActive(agent),
  ).length;
  const hasSavedAgentDefaults = Boolean(
    globalConfig.preferred_runtime?.trim() ||
      globalConfig.provider?.trim() ||
      globalConfig.model?.trim() ||
      Object.values(globalConfig.env_vars).some(
        (value) => value.trim().length > 0,
      ),
  );
  // biome-ignore lint/correctness/useExhaustiveDependencies: mount-only; personas.handleImportSnapshotFile and teamActions.handleImportTeamSnapshotFile are stable
  React.useEffect(() => {
    // Consume a snapshot import that was enqueued before navigation (e.g. from
    // a timeline AgentSnapshotCard click that navigated here).
    const pending = consumePendingSnapshotImport();
    if (pending) {
      if (pending.snapshotKind === "team") {
        void teamActions.handleImportTeamSnapshotFile(
          pending.fileBytes,
          pending.fileName,
        );
      } else {
        void personas.handleImportSnapshotFile(
          pending.fileBytes,
          pending.fileName,
        );
      }
    }

    return subscribeSnapshotImport(({ fileBytes, fileName, snapshotKind }) => {
      if (snapshotKind === "team") {
        void teamActions.handleImportTeamSnapshotFile(fileBytes, fileName);
      } else {
        void personas.handleImportSnapshotFile(fileBytes, fileName);
      }
    });
  }, []);

  return (
    <>
      <div className="flex-1 overflow-y-auto overflow-x-hidden overscroll-contain px-4 py-7 sm:px-6 sm:py-8">
        <div
          className={`mx-auto grid w-full max-w-6xl ${AGENT_CARD_GRID_COLUMNS_CLASS} justify-start gap-x-3 gap-y-8`}
        >
          <PageHeader
            className="col-[1/-1]"
            action={
              <div className="flex flex-wrap justify-end gap-2">
                <Button
                  data-testid="agent-defaults-button"
                  ref={aiDefaultsTriggerRef}
                  onClick={() => setIsAiDefaultsOpen(true)}
                  size="sm"
                  variant="outline"
                >
                  <Settings2 />
                  {hasSavedAgentDefaults
                    ? "Agent defaults"
                    : "Set agent defaults"}
                </Button>
                {runningAgentCount > 0 ? (
                  <Button
                    disabled={isActionPending}
                    onClick={() => {
                      void agents.handleBulkStopRunning();
                    }}
                    size="sm"
                    variant="outline"
                  >
                    <OctagonX />
                    Stop running agents
                  </Button>
                ) : null}
              </div>
            }
            description="Set up and manage your agents."
            title="Agents"
          />
          <div className="col-[1/-1] flex flex-col gap-8">
            <UnifiedAgentsSection
              defaultModel={inheritedDefaults.model.value}
              actionErrorMessage={agents.actionErrorMessage}
              actionNoticeMessage={agents.actionNoticeMessage}
              agents={agents.managedAgents}
              agentsError={
                agents.managedAgentsQuery.error instanceof Error
                  ? agents.managedAgentsQuery.error
                  : null
              }
              isActionPending={isActionPending}
              isAgentsLoading={agents.managedAgentsQuery.isLoading}
              startingAgentPubkey={agents.startingAgentPubkey}
              startingPersonaIds={agents.startingPersonaIds}
              onOpenAgentProfile={(pubkey, options) => {
                openProfilePanel?.(pubkey, options);
              }}
              onOpenPersonaProfile={(persona) => {
                openPersonaProfilePanel?.(persona);
              }}
              onStartAgent={(pubkey) => {
                void agents.handleStart(pubkey);
              }}
              onStartPersona={(persona) => {
                void agents.handleStartPersona(persona);
              }}
              // Persona props
              personas={personas.libraryPersonas}
              personasError={
                personas.personasQuery.error instanceof Error
                  ? personas.personasQuery.error
                  : null
              }
              personaFeedbackErrorMessage={
                personas.personaFeedbackSurface === "library"
                  ? personas.personaErrorMessage
                  : null
              }
              personaFeedbackNoticeMessage={
                personas.personaFeedbackSurface === "library"
                  ? personas.personaNoticeMessage
                  : null
              }
              isPersonasLoading={personas.personasQuery.isLoading}
              isPersonasPending={personas.isPending}
              onCreatePersona={openUnifiedCreate}
              onDiscoverPersonas={personas.openCatalog}
              onDuplicatePersona={personas.openDuplicate}
              onEditPersona={personas.openEdit}
              onSharePersona={personas.openShare}
              onDeactivatePersona={(persona) => {
                void personas.handleSetActive(persona, false, "library");
              }}
              onDeletePersona={personas.openDelete}
              onImportSnapshotFile={(fileBytes, fileName) => {
                void personas.handleImportSnapshotFile(fileBytes, fileName);
              }}
            />

            <TeamsSection
              error={
                teamActions.teamsQuery.error instanceof Error
                  ? teamActions.teamsQuery.error
                  : null
              }
              isLoading={teamActions.teamsQuery.isLoading}
              isPending={
                teamActions.createTeamMutation.isPending ||
                teamActions.updateTeamMutation.isPending ||
                teamActions.deleteTeamMutation.isPending
              }
              onCreate={teamActions.openCreateDialog}
              onDelete={teamActions.setTeamToDelete}
              onDuplicate={teamActions.openDuplicateDialog}
              onEdit={teamActions.openEditDialog}
              onAddToChannel={teamActions.setTeamToAddToChannel}
              onShare={teamActions.openShare}
              onImport={() => {
                teamImportInputRef.current?.click();
              }}
              personas={personas.libraryPersonas}
              teams={teamActions.teams}
            />

            <RelayDirectorySection
              error={
                agents.relayAgentsQuery.error instanceof Error
                  ? agents.relayAgentsQuery.error
                  : null
              }
              isLoading={agents.relayAgentsQuery.isLoading}
              managedPubkeys={agents.managedPubkeys}
              relayAgents={agents.relayAgentsQuery.data ?? []}
            />
          </div>
        </div>
      </div>

      <AgentDefaultsDialog
        onOpenChange={setIsAiDefaultsOpen}
        open={isAiDefaultsOpen}
        returnFocusRef={aiDefaultsTriggerRef}
      />

      {isCreateDialogOpen ? (
        <AgentDialog
          definitionError={
            personas.createPersonaMutation.error instanceof Error
              ? personas.createPersonaMutation.error
              : null
          }
          isDefinitionPending={personas.isPending}
          mode="definition"
          onOpenChange={(open) => {
            if (!open) setIsCreateDialogOpen(false);
          }}
          onSubmitDefinition={personas.handleSubmit}
          runtimes={personas.acpRuntimesQuery.data ?? []}
          runtimesLoading={personas.acpRuntimesQuery.isLoading}
        />
      ) : null}
      {agents.agentToAddToChannel ? (
        <AddAgentToChannelDialog
          agent={agents.agentToAddToChannel}
          onAdded={agents.handleAddedToChannel}
          onOpenChange={(open) => {
            if (!open) {
              agents.setAgentToAddToChannel(null);
            }
          }}
          open={agents.agentToAddToChannel !== null}
        />
      ) : null}
      {agents.createdAgent ? (
        <SecretRevealDialog
          created={agents.createdAgent}
          onOpenChange={(open) => {
            if (!open) {
              agents.setCreatedAgent(null);
            }
          }}
        />
      ) : null}
      {personas.createdAgent ? (
        <SecretRevealDialog
          created={personas.createdAgent}
          onOpenChange={(open) => {
            if (!open) personas.dismissCreatedAgent();
          }}
        />
      ) : null}
      {personas.personaDialogState ? (
        <AgentDialog
          description={personas.personaDialogState.description}
          error={
            personas.updatePersonaMutation.error instanceof Error
              ? personas.updatePersonaMutation.error
              : personas.createPersonaMutation.error instanceof Error
                ? personas.createPersonaMutation.error
                : null
          }
          initialValues={personas.personaDialogState.initialValues}
          isPending={personas.isPending}
          mode="definition-edit"
          runtimes={personas.acpRuntimesQuery.data ?? []}
          runtimesLoading={personas.acpRuntimesQuery.isLoading}
          onOpenChange={(open) => {
            if (!open) {
              personas.setPersonaDialogState(null);
            }
          }}
          onSubmit={(input, options) =>
            personas.handleSubmit(
              input,
              undefined,
              undefined,
              undefined,
              options,
            )
          }
          open={personas.personaDialogState !== null}
          publishCatalogUpdatesOnSave={
            "id" in personas.personaDialogState.initialValues &&
            personas.sharedCatalogPersonaIdSet.has(
              personas.personaDialogState.initialValues.id,
            )
          }
          submitLabel={personas.personaDialogState.submitLabel}
          title={personas.personaDialogState.title}
        />
      ) : null}
      {personas.personaToDelete ? (
        <PersonaDeleteDialog
          instanceCount={
            (agents.managedAgents ?? []).filter(
              (a) => a.personaId === personas.personaToDelete?.id,
            ).length
          }
          onConfirm={(persona) => {
            void personas.handleDelete(persona);
          }}
          onOpenChange={(open) => {
            if (!open) {
              personas.setPersonaToDelete(null);
            }
          }}
          open={personas.personaToDelete !== null}
          persona={personas.personaToDelete}
        />
      ) : null}
      {personas.personaToShare ? (
        <PersonaShareDialog
          hasCatalogUpdates={personas.hasPersonaCatalogUpdates(
            personas.personaToShare.persona,
          )}
          catalogShareLevel={personas.getPersonaCatalogShareLevel(
            personas.personaToShare.persona,
          )}
          isPending={personas.isPending}
          linkedAgentPubkey={personas.personaToShare.linkedAgentPubkey}
          onCatalogShareLevelChange={(shareLevel) => {
            const shareTarget = personas.personaToShare;
            if (!shareTarget) return;
            personas.setPersonaCatalogShareLevel(
              shareTarget.persona,
              shareLevel,
            );
          }}
          onExport={() => {
            const shareTarget = personas.personaToShare;
            if (!shareTarget) return;
            personas.setPersonaToShare(null);
            personas.setPersonaToExportSnapshot(shareTarget);
          }}
          onOpenChange={(open) => {
            if (!open) {
              personas.setPersonaToShare(null);
            }
          }}
          onPublishCatalogUpdates={() => {
            const shareTarget = personas.personaToShare;
            if (!shareTarget) return;
            personas.publishPersonaCatalogUpdates(shareTarget.persona);
          }}
          open={personas.personaToShare !== null}
          persona={personas.personaToShare.persona}
        />
      ) : null}
      {personas.personaToExportSnapshot ? (
        <AgentSnapshotExportDialog
          agentName={personas.personaToExportSnapshot.persona.displayName}
          isSavePending={personas.isPending}
          open={personas.personaToExportSnapshot !== null}
          linkedAgentPubkey={personas.personaToExportSnapshot.linkedAgentPubkey}
          onSaveFile={(memoryLevel, format) => {
            if (personas.personaToExportSnapshot) {
              personas.handleExportSnapshot(
                personas.personaToExportSnapshot.persona,
                personas.personaToExportSnapshot.linkedAgentPubkey,
                memoryLevel,
                format,
              );
            }
          }}
          onOpenChange={(open) => {
            if (!open) {
              personas.setPersonaToExportSnapshot(null);
            }
          }}
        />
      ) : null}
      {personas.snapshotImportState ? (
        <AgentSnapshotImportDialog
          open={personas.snapshotImportState !== null}
          preview={personas.snapshotImportState.preview}
          isConfirming={personas.isSnapshotImportConfirming}
          result={personas.snapshotImportResult}
          confirmError={personas.snapshotImportConfirmError}
          onConfirm={(keepAllowlist) => {
            void personas.handleConfirmSnapshotImport(keepAllowlist);
          }}
          onOpenChange={(open) => {
            if (!open) {
              personas.closeSnapshotImportDialog();
            }
          }}
        />
      ) : null}
      {personas.isCatalogDialogOpen ? (
        <PersonaCatalogDialog
          error={
            personas.personasQuery.error instanceof Error
              ? personas.personasQuery.error
              : null
          }
          feedbackErrorMessage={
            personas.personaFeedbackSurface === "catalog"
              ? personas.personaErrorMessage
              : null
          }
          feedbackNoticeMessage={
            personas.personaFeedbackSurface === "catalog"
              ? personas.personaNoticeMessage
              : null
          }
          isLoading={personas.personasQuery.isLoading}
          isPending={personas.setPersonaActiveMutation.isPending}
          onClearFeedback={() => {
            personas.clearFeedback("catalog");
          }}
          onOpenChange={personas.setIsCatalogDialogOpen}
          onSelectPersona={(persona, active) => {
            void personas.handleSetActive(persona, active, "catalog");
          }}
          open={personas.isCatalogDialogOpen}
          personas={personas.catalogPersonas}
        />
      ) : null}
      {teamActions.teamDialogState ? (
        <TeamDialog
          description={teamActions.teamDialogState.description}
          error={
            teamActions.updateTeamMutation.error instanceof Error
              ? teamActions.updateTeamMutation.error
              : teamActions.createTeamMutation.error instanceof Error
                ? teamActions.createTeamMutation.error
                : null
          }
          initialValues={teamActions.teamDialogState.initialValues}
          isPending={
            teamActions.createTeamMutation.isPending ||
            teamActions.updateTeamMutation.isPending
          }
          onOpenChange={(open) => {
            if (!open) {
              teamActions.setTeamDialogState(null);
            }
          }}
          onDeleteRemovedPersonas={teamActions.handleDeleteRemovedPersonas}
          onSubmit={teamActions.handleTeamSubmit}
          open={teamActions.teamDialogState !== null}
          personas={personas.libraryPersonas}
          submitLabel={teamActions.teamDialogState.submitLabel}
          title={teamActions.teamDialogState.title}
        />
      ) : null}
      {teamActions.teamToDelete ? (
        <TeamDeleteDialog
          onConfirm={(team) => {
            void teamActions.handleDeleteTeam(team);
          }}
          onOpenChange={(open) => {
            if (!open) {
              teamActions.setTeamToDelete(null);
            }
          }}
          open={teamActions.teamToDelete !== null}
          team={teamActions.teamToDelete}
        />
      ) : null}
      {teamActions.teamToAddToChannel ? (
        <AddTeamToChannelDialog
          onDeployed={teamActions.handleTeamDeployed}
          onOpenChange={(open) => {
            if (!open) {
              teamActions.setTeamToAddToChannel(null);
            }
          }}
          open={teamActions.teamToAddToChannel !== null}
          personas={personas.libraryPersonas}
          team={teamActions.teamToAddToChannel}
        />
      ) : null}
      {teamActions.teamToShare ? (
        <TeamShareDialog
          isPending={
            teamActions.createTeamMutation.isPending ||
            teamActions.updateTeamMutation.isPending ||
            teamActions.deleteTeamMutation.isPending
          }
          onExport={() => {
            if (teamActions.teamToShare) {
              const team = teamActions.teamToShare;
              teamActions.setTeamToShare(null);
              teamActions.openExportSnapshot(team);
            }
          }}
          onOpenChange={(open) => {
            if (!open) {
              teamActions.setTeamToShare(null);
            }
          }}
          open={teamActions.teamToShare !== null}
          team={teamActions.teamToShare}
        />
      ) : null}
      {teamActions.teamToExport ? (
        <TeamSnapshotExportDialog
          isSavePending={teamActions.exportTeamSnapshotMutation.isPending}
          open={teamActions.teamToExport !== null}
          team={teamActions.teamToExport}
          onSaveFile={(memoryLevel, format) => {
            if (teamActions.teamToExport) {
              teamActions.handleExportTeamSnapshot(
                teamActions.teamToExport,
                memoryLevel,
                format,
              );
            }
          }}
          onOpenChange={(open) => {
            if (!open) {
              teamActions.setTeamToExport(null);
            }
          }}
        />
      ) : null}
      {teamActions.teamSnapshotImportState ? (
        <TeamSnapshotImportDialog
          open={teamActions.teamSnapshotImportState !== null}
          preview={teamActions.teamSnapshotImportState.preview}
          isConfirming={teamActions.isTeamSnapshotImportConfirming}
          result={teamActions.teamSnapshotImportResult}
          confirmError={teamActions.teamSnapshotImportConfirmError}
          onConfirm={(keepAllowlist) => {
            void teamActions.handleConfirmTeamSnapshotImport(keepAllowlist);
          }}
          onOpenChange={(open) => {
            if (!open) {
              teamActions.closeTeamSnapshotImportDialog();
            }
          }}
        />
      ) : null}
      {/* Hidden file input for team snapshot import via file picker */}
      <input
        accept=".team.json,.team.png"
        className="hidden"
        data-testid="team-snapshot-import-input"
        ref={teamImportInputRef}
        type="file"
        onChange={(e) => {
          const file = e.target.files?.[0];
          if (!file) return;
          const reader = new FileReader();
          reader.onload = () => {
            const buffer = reader.result as ArrayBuffer;
            const fileBytes = Array.from(new Uint8Array(buffer));
            void teamActions.handleImportTeamSnapshotFile(fileBytes, file.name);
          };
          reader.readAsArrayBuffer(file);
          // Reset so the same file can be picked again.
          e.target.value = "";
        }}
      />
    </>
  );
}
