import * as React from "react";
import {
  consumePendingOpenCreateAgent,
  subscribeOpenCreateAgent,
} from "@/features/agents/openCreateAgentEvent";
import { AddAgentToChannelDialog } from "./AddAgentToChannelDialog";
import { AddTeamToChannelDialog } from "./AddTeamToChannelDialog";
import { BatchImportDialog } from "./BatchImportDialog";
import { CreateAgentDialog } from "./CreateAgentDialog";
import { PersonaCatalogDialog } from "./PersonaCatalogDialog";
import { PersonaDialog } from "./PersonaDialog";
import { PersonaDeleteDialog } from "./PersonaDeleteDialog";
import { PersonaImportUpdateDialog } from "./PersonaImportUpdateDialog";
import { PersonaShareDialog } from "./PersonaShareDialog";
import { RelayDirectorySection } from "./RelayDirectorySection";
import { SecretRevealDialog } from "./SecretRevealDialog";
import { TeamDeleteDialog } from "./TeamDeleteDialog";
import { TeamDialog } from "./TeamDialog";
import { TeamImportDialog } from "./TeamImportDialog";
import { TeamImportUpdateDialog } from "./TeamImportUpdateDialog";
import { TeamsSection } from "./TeamsSection";
import { UnifiedAgentsSection } from "./UnifiedAgentsSection";
import { useManagedAgentActions } from "./useManagedAgentActions";
import { usePersonaActions } from "./usePersonaActions";
import { useTeamActions } from "./useTeamActions";
import { useProfilePanel } from "@/shared/context/ProfilePanelContext";

export function AgentsView() {
  const { openPersonaProfilePanel, openProfilePanel } = useProfilePanel();
  const agents = useManagedAgentActions();
  const personas = usePersonaActions();
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
    teamActions.exportTeamJsonMutation.isPending ||
    teamActions.createTeamMutation.isPending ||
    teamActions.updateTeamMutation.isPending ||
    teamActions.deleteTeamMutation.isPending;

  React.useEffect(() => {
    if (consumePendingOpenCreateAgent()) {
      agents.setIsCreateOpen(true);
    }

    return subscribeOpenCreateAgent(() => {
      agents.setIsCreateOpen(true);
    });
  }, [agents.setIsCreateOpen]);

  return (
    <>
      <div className="flex-1 overflow-y-auto overflow-x-hidden overscroll-contain px-4 py-7 sm:px-6 sm:py-8">
        <div className="mx-auto flex w-full max-w-6xl flex-col gap-8">
          <div className="flex flex-col gap-8">
            <UnifiedAgentsSection
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
              onBulkRemoveStopped={() => {
                void agents.handleBulkRemoveStopped();
              }}
              onBulkStopRunning={() => {
                void agents.handleBulkStopRunning();
              }}
              onCreateAgent={() => {
                agents.setIsCreateOpen(true);
              }}
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
              canChooseCatalog={personas.catalogPersonas.length > 0}
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
              onCreatePersona={personas.openCreate}
              onChooseCatalog={personas.openCatalog}
              onDuplicatePersona={personas.openDuplicate}
              onEditPersona={personas.openEdit}
              onSharePersona={personas.openShare}
              onDeactivatePersona={(persona) => {
                void personas.handleSetActive(persona, false, "library");
              }}
              onDeletePersona={personas.openDelete}
              onImportPersonaFile={(fileBytes, fileName) => {
                void personas.handleImportFile(fileBytes, fileName);
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
              onExport={teamActions.handleExportTeam}
              onImportFile={teamActions.handleImportFile}
              onInstallFromDirectory={teamActions.handleInstallFromDirectory}
              onSync={teamActions.handleSyncTeam}
              onRevealInFinder={teamActions.handleRevealInFinder}
              onAddToChannel={teamActions.setTeamToAddToChannel}
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

      {agents.isCreateOpen ? (
        <CreateAgentDialog
          onCreated={(result) => {
            agents.setLogAgentPubkey(result.agent.pubkey);
            agents.setCreatedAgent(result);
          }}
          onOpenChange={agents.setIsCreateOpen}
          open={agents.isCreateOpen}
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
            if (!open) {
              personas.setCreatedAgent(null);
            }
          }}
        />
      ) : null}
      {personas.personaDialogState ? (
        <PersonaDialog
          description={personas.personaDialogState.description}
          error={
            personas.updatePersonaMutation.error instanceof Error
              ? personas.updatePersonaMutation.error
              : personas.createPersonaMutation.error instanceof Error
                ? personas.createPersonaMutation.error
                : null
          }
          initialValues={personas.personaDialogState.initialValues}
          isImportPending={
            personas.personaImportActions.isApplyingPersonaImportUpdate
          }
          isPending={personas.isPending}
          runtimes={personas.acpRuntimesQuery.data ?? []}
          runtimesLoading={personas.acpRuntimesQuery.isLoading}
          onImportUpdateFile={
            personas.personaImportActions.handleEditDialogImportUpdateFile
          }
          onOpenChange={(open) => {
            if (!open) {
              personas.setPersonaDialogState(null);
            }
          }}
          onSubmit={personas.handleSubmit}
          open={personas.personaDialogState !== null}
          submitLabel={personas.personaDialogState.submitLabel}
          title={personas.personaDialogState.title}
        />
      ) : null}
      {personas.personaToDelete ? (
        <PersonaDeleteDialog
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
          isCatalogVisible={
            personas.personaToShare.isBuiltIn ||
            personas.sharedCatalogPersonaIdSet.has(personas.personaToShare.id)
          }
          isPending={personas.isPending}
          onCatalogVisibilityChange={(visible) => {
            if (personas.personaToShare) {
              personas.setPersonaCatalogVisibility(
                personas.personaToShare,
                visible,
              );
            }
          }}
          onExport={() => {
            if (personas.personaToShare) {
              personas.handleExport(personas.personaToShare);
            }
          }}
          onOpenChange={(open) => {
            if (!open) {
              personas.setPersonaToShare(null);
            }
          }}
          open={personas.personaToShare !== null}
          persona={personas.personaToShare}
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
          isImportPending={teamActions.isApplyingTeamImportUpdate}
          isPending={
            teamActions.createTeamMutation.isPending ||
            teamActions.updateTeamMutation.isPending
          }
          onImportUpdateFile={teamActions.handleEditDialogImportUpdateFile}
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
      {personas.batchImportResult ? (
        <BatchImportDialog
          fileName={personas.batchImportFileName}
          onComplete={personas.handleBatchImportComplete}
          onOpenChange={(open) => {
            if (!open) {
              personas.setBatchImportResult(null);
            }
          }}
          open={personas.batchImportResult !== null}
          result={personas.batchImportResult}
        />
      ) : null}
      {teamActions.teamImportPreview ? (
        <TeamImportDialog
          fileName={teamActions.teamImportPreview.fileName}
          onComplete={teamActions.handleTeamImportComplete}
          onOpenChange={(open) => {
            if (!open) {
              teamActions.setTeamImportPreview(null);
            }
          }}
          open={teamActions.teamImportPreview !== null}
          preview={teamActions.teamImportPreview.preview}
        />
      ) : null}
      {teamActions.teamImportTarget ? (
        <TeamImportUpdateDialog
          fileName={teamActions.teamImportTargetPreview?.fileName ?? ""}
          isPending={
            teamActions.isApplyingTeamImportUpdate ||
            teamActions.updateTeamMutation.isPending
          }
          onApply={teamActions.handleTeamImportUpdateApply}
          onClear={teamActions.clearImportUpdateAndReturnToEdit}
          onOpenChange={(open) => {
            if (!open) {
              teamActions.closeImportUpdateDialog();
            }
          }}
          open={teamActions.teamImportTarget !== null}
          personas={personas.libraryPersonas}
          preview={teamActions.teamImportTargetPreview?.preview ?? null}
          team={teamActions.teamImportTarget}
        />
      ) : null}
      {personas.personaImportActions.personaImportTarget ? (
        <PersonaImportUpdateDialog
          fileName={
            personas.personaImportActions.personaImportTargetPreview
              ?.fileName ?? ""
          }
          isPending={
            personas.personaImportActions.isApplyingPersonaImportUpdate ||
            personas.updatePersonaMutation.isPending
          }
          onApply={personas.personaImportActions.handleImportUpdateApply}
          onClear={
            personas.personaImportActions.clearImportUpdateAndReturnToEdit
          }
          onOpenChange={(open) => {
            if (!open) {
              personas.personaImportActions.closeImportUpdateDialog();
            }
          }}
          open={personas.personaImportActions.personaImportTarget !== null}
          persona={personas.personaImportActions.personaImportTarget}
          preview={
            personas.personaImportActions.personaImportTargetPreview?.preview ??
            null
          }
        />
      ) : null}
    </>
  );
}
