import { topChromeInset } from "@/shared/layout/chromeLayout";
import { cn } from "@/shared/lib/cn";
import { AddAgentToChannelDialog } from "./AddAgentToChannelDialog";
import { AddTeamToChannelDialog } from "./AddTeamToChannelDialog";
import { BatchImportDialog } from "./BatchImportDialog";
import { CreateAgentDialog } from "./CreateAgentDialog";
import { PersonaCatalogDialog } from "./PersonaCatalogDialog";
import { PersonaDialog } from "./PersonaDialog";
import { PersonaDeleteDialog } from "./PersonaDeleteDialog";
import { PersonaImportUpdateDialog } from "./PersonaImportUpdateDialog";
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

export function AgentsView() {
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

  return (
    <>
      <div
        className={cn(
          "flex-1 overflow-y-auto overflow-x-hidden overscroll-contain px-4 pb-4 sm:px-6",
          topChromeInset.padding,
        )}
      >
        <div className="mx-auto flex w-full max-w-6xl flex-col gap-6">
          <div className="flex flex-col gap-6">
            <UnifiedAgentsSection
              actionErrorMessage={agents.actionErrorMessage}
              actionNoticeMessage={agents.actionNoticeMessage}
              agents={agents.managedAgents}
              channelIdToName={agents.channelIdToName}
              channelsByPubkey={agents.channelsByPubkey}
              agentsError={
                agents.managedAgentsQuery.error instanceof Error
                  ? agents.managedAgentsQuery.error
                  : null
              }
              isActionPending={isActionPending}
              isAgentsLoading={agents.managedAgentsQuery.isLoading}
              logContent={agents.managedAgentLogQuery.data?.content ?? null}
              logError={
                agents.managedAgentLogQuery.error instanceof Error
                  ? agents.managedAgentLogQuery.error
                  : null
              }
              logLoading={agents.managedAgentLogQuery.isLoading}
              personaLabelsById={personas.personaLabelsById}
              presenceLoaded={agents.managedPresenceQuery.isSuccess}
              presenceLookup={agents.managedPresenceQuery.data ?? {}}
              onAddToChannel={(agent) => {
                agents.setActionNoticeMessage(null);
                agents.setActionErrorMessage(null);
                agents.setAgentToAddToChannel(agent);
              }}
              onBulkRemoveStopped={() => {
                void agents.handleBulkRemoveStopped();
              }}
              onBulkStopRunning={() => {
                void agents.handleBulkStopRunning();
              }}
              onCreateAgent={() => {
                agents.setIsCreateOpen(true);
              }}
              onDeleteAgent={(pubkey) => {
                void agents.handleDelete(pubkey);
              }}
              onSelectLogAgent={agents.setLogAgentPubkey}
              onStartAgent={(pubkey) => {
                void agents.handleStart(pubkey);
              }}
              onStopAgent={(pubkey) => {
                void agents.handleStop(pubkey);
              }}
              onToggleStartOnAppLaunch={(pubkey, startOnAppLaunch) => {
                void agents.handleToggleStartOnAppLaunch(
                  pubkey,
                  startOnAppLaunch,
                );
              }}
              selectedLogAgentPubkey={agents.logAgentPubkey}
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
              onExportPersona={personas.handleExport}
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

      <CreateAgentDialog
        onCreated={(result) => {
          agents.setLogAgentPubkey(result.agent.pubkey);
          agents.setCreatedAgent(result);
        }}
        onOpenChange={agents.setIsCreateOpen}
        open={agents.isCreateOpen}
      />
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
      <SecretRevealDialog
        created={agents.createdAgent}
        onOpenChange={(open) => {
          if (!open) {
            agents.setCreatedAgent(null);
          }
        }}
      />
      <PersonaDialog
        description={personas.personaDialogState?.description ?? ""}
        error={
          personas.updatePersonaMutation.error instanceof Error
            ? personas.updatePersonaMutation.error
            : personas.createPersonaMutation.error instanceof Error
              ? personas.createPersonaMutation.error
              : null
        }
        initialValues={personas.personaDialogState?.initialValues ?? null}
        isImportPending={
          personas.personaImportActions.isApplyingPersonaImportUpdate
        }
        isPending={
          personas.createPersonaMutation.isPending ||
          personas.updatePersonaMutation.isPending
        }
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
        submitLabel={personas.personaDialogState?.submitLabel ?? "Save"}
        title={personas.personaDialogState?.title ?? "Persona"}
      />
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
      <TeamDialog
        description={teamActions.teamDialogState?.description ?? ""}
        error={
          teamActions.updateTeamMutation.error instanceof Error
            ? teamActions.updateTeamMutation.error
            : teamActions.createTeamMutation.error instanceof Error
              ? teamActions.createTeamMutation.error
              : null
        }
        initialValues={teamActions.teamDialogState?.initialValues ?? null}
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
        submitLabel={teamActions.teamDialogState?.submitLabel ?? "Save"}
        title={teamActions.teamDialogState?.title ?? "Team"}
      />
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
      <TeamImportDialog
        fileName={teamActions.teamImportPreview?.fileName ?? ""}
        onComplete={teamActions.handleTeamImportComplete}
        onOpenChange={(open) => {
          if (!open) {
            teamActions.setTeamImportPreview(null);
          }
        }}
        open={teamActions.teamImportPreview !== null}
        preview={teamActions.teamImportPreview?.preview ?? null}
      />
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
      <PersonaImportUpdateDialog
        fileName={
          personas.personaImportActions.personaImportTargetPreview?.fileName ??
          ""
        }
        isPending={
          personas.personaImportActions.isApplyingPersonaImportUpdate ||
          personas.updatePersonaMutation.isPending
        }
        onApply={personas.personaImportActions.handleImportUpdateApply}
        onClear={personas.personaImportActions.clearImportUpdateAndReturnToEdit}
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
    </>
  );
}
