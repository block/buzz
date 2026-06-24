import * as React from "react";
import { ArrowLeft, X } from "lucide-react";
import { toast } from "sonner";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import {
  useAgentMemoryQuery,
  useIsManagedAgent,
} from "@/features/agent-memory/hooks";
import { MemoryRefreshButton } from "@/features/agent-memory/ui/MemorySection";
import {
  type AttachManagedAgentToChannelResult,
  useAcpRuntimesQuery,
  useAvailableAcpRuntimes,
  useCreateManagedAgentMutation,
  useCreatePersonaMutation,
  useDeleteManagedAgentMutation,
  useDeletePersonaMutation,
  useExportPersonaJsonMutation,
  useManagedAgentLogQuery,
  useRelayAgentsQuery,
  useManagedAgentsQuery,
  usePersonasQuery,
  useSetManagedAgentStartOnAppLaunchMutation,
  useSetPersonaActiveMutation,
  useStartManagedAgentMutation,
  useStopManagedAgentMutation,
  useUpdateManagedAgentMutation,
  useUpdatePersonaMutation,
} from "@/features/agents/hooks";
import { AddAgentToChannelDialog } from "@/features/agents/ui/AddAgentToChannelDialog";
import { useActiveAgentTurnsBridge } from "@/features/agents/activeAgentTurnsStore";
import { resolvePersonaRuntime } from "@/features/agents/lib/resolvePersonaRuntime";
import {
  isManagedAgentActive,
  startManagedAgentWithRules,
  stopManagedAgentWithRules,
} from "@/features/agents/lib/managedAgentControlActions";
import { useManagedAgentObserverBridge } from "@/features/agents/observerRelayStore";
import { EditAgentDialog } from "@/features/agents/ui/EditAgentDialog";
import {
  duplicatePersonaDialogState,
  editPersonaDialogState,
  type PersonaDialogState,
} from "@/features/agents/ui/personaDialogState";
import { useChannelsQuery } from "@/features/channels/hooks";
import { usePresenceQuery } from "@/features/presence/hooks";
import {
  useContactListQuery,
  useFollowMutation,
  useProfileQuery,
  useUnfollowMutation,
  useUserProfileQuery,
} from "@/features/profile/hooks";
import {
  AgentInfoFocusedView,
  ChannelsFocusedView,
  DiagnosticsFocusedView,
  MemoryFocusedView,
  ProfileSummaryView,
} from "@/features/profile/ui/UserProfilePanelSections";
import { AgentConfigurationFocusedView } from "@/features/profile/ui/UserProfilePanelAgentDetails";
import { useProfileAgentDeletion } from "@/features/profile/ui/UserProfilePanelDeletion";
import { useProfileFieldBuckets } from "@/features/profile/ui/UserProfilePanelFields";
import { submitProfilePersonaDialog } from "@/features/profile/ui/UserProfilePanelPersonaSubmit";
import { UserProfilePersonaDialogs } from "@/features/profile/ui/UserProfilePersonaDialogs";
import {
  deriveProfileChannels,
  PROFILE_PANEL_VIEW_TITLES,
  type ProfilePanelView,
  resolveAgentInstruction,
  resolveOwnerHandle,
  resolvePanelProfile,
  resolveProfileDisplayName,
  type UserProfilePanelProps,
  useRetainedPersona,
} from "@/features/profile/ui/UserProfilePanelUtils";
import { useUserStatusQuery } from "@/features/user-status/hooks";
import { useAgentSession } from "@/shared/context/AgentSessionContext";
import { useEscapeKey } from "@/shared/hooks/useEscapeKey";
import { useIsThreadPanelOverlay } from "@/shared/hooks/use-mobile";
import { THREAD_PANEL_MIN_WIDTH_PX } from "@/shared/hooks/useThreadPanelWidth";
import {
  AuxiliaryPanelHeader,
  AuxiliaryPanelHeaderGroup,
  AuxiliaryPanelTitle,
  auxiliaryPanelContentPaddingClass,
} from "@/shared/layout/AuxiliaryPanelHeader";
import { cn } from "@/shared/lib/cn";
import type {
  AgentPersona,
  Channel,
  CreateManagedAgentInput,
  CreatePersonaInput,
  UpdatePersonaInput,
} from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import {
  OverlayPanelBackdrop,
  PANEL_BASE_CLASS,
  PANEL_OVERLAY_CLASS,
  PANEL_SINGLE_COLUMN_HEADER_LAYER_CLASS,
} from "@/shared/ui/OverlayPanelBackdrop";

export type { ProfilePanelView };

export function UserProfilePanel({
  canResetWidth,
  currentPubkey,
  isSinglePanelView = false,
  layout = "standalone",
  onClose,
  onOpenDm,
  onOpenProfile,
  onResetWidth,
  onResizeStart,
  onViewChange,
  persona,
  pubkey,
  splitPaneClamp = false,
  view: controlledView,
  widthPx,
}: UserProfilePanelProps) {
  const isOverlay = useIsThreadPanelOverlay();
  const isFloatingOverlay = isOverlay && !isSinglePanelView;
  const isSplitLayout = layout === "split";
  useEscapeKey(onClose, isOverlay || isSinglePanelView);

  const [internalView, setInternalView] =
    React.useState<ProfilePanelView>("summary");
  const view = controlledView ?? internalView;
  const setView = React.useCallback(
    (nextView: ProfilePanelView, options?: { replace?: boolean }) => {
      if (onViewChange) {
        onViewChange(nextView, options);
        return;
      }
      setInternalView(nextView);
    },
    [onViewChange],
  );
  const [editAgentOpen, setEditAgentOpen] = React.useState(false);
  const [addToChannelOpen, setAddToChannelOpen] = React.useState(false);
  const [personaDialogState, setPersonaDialogState] =
    React.useState<PersonaDialogState | null>(null);
  const [personaToDelete, setPersonaToDelete] =
    React.useState<AgentPersona | null>(null);

  const personasQuery = usePersonasQuery();
  const managedAgentsQuery = useManagedAgentsQuery({ enabled: true });
  const managedAgent = React.useMemo(() => {
    const agents = managedAgentsQuery.data ?? [];
    if (pubkey) {
      const pubkeyLower = pubkey.toLowerCase();
      return agents.find((agent) => agent.pubkey.toLowerCase() === pubkeyLower);
    }
    if (persona) {
      return agents.find((agent) => agent.personaId === persona.id);
    }
    return undefined;
  }, [managedAgentsQuery.data, persona, pubkey]);
  const resolvedPersonaFromSource = React.useMemo(() => {
    const personaId = persona?.id ?? managedAgent?.personaId;
    if (personaId) {
      const refreshedPersona = personasQuery.data?.find(
        (candidate) => candidate.id === personaId,
      );
      if (refreshedPersona) {
        return refreshedPersona;
      }
    }
    if (persona) {
      return persona;
    }
    if (!managedAgent?.personaId) {
      return undefined;
    }
    return personasQuery.data?.find(
      (candidate) => candidate.id === managedAgent.personaId,
    );
  }, [managedAgent?.personaId, persona, personasQuery.data]);
  const profileIdentityKey =
    pubkey ?? managedAgent?.pubkey ?? `persona:${persona?.id ?? "unknown"}`;
  const resolvedPersona = useRetainedPersona(
    resolvedPersonaFromSource,
    profileIdentityKey,
  );
  const effectivePubkey = pubkey ?? managedAgent?.pubkey ?? null;
  const pubkeyLower = effectivePubkey?.toLowerCase() ?? "";

  const profileQuery = useUserProfileQuery(effectivePubkey ?? undefined);
  const currentProfileQuery = useProfileQuery(currentPubkey !== undefined);

  React.useEffect(() => {
    if (!effectivePubkey) return;
    void profileQuery.refetch();
  }, [effectivePubkey, profileQuery.refetch]);

  const relayAgentsQuery = useRelayAgentsQuery({ enabled: true });
  const availableRuntimesQuery = useAvailableAcpRuntimes();
  const acpRuntimesQuery = useAcpRuntimesQuery();
  const createAgentMutation = useCreateManagedAgentMutation();
  const updateManagedAgentMutation = useUpdateManagedAgentMutation();
  const startAgentMutation = useStartManagedAgentMutation();
  const stopAgentMutation = useStopManagedAgentMutation();
  const deleteAgentMutation = useDeleteManagedAgentMutation();
  const startOnLaunchMutation = useSetManagedAgentStartOnAppLaunchMutation();
  const createPersonaMutation = useCreatePersonaMutation();
  const updatePersonaMutation = useUpdatePersonaMutation();
  const deletePersonaMutation = useDeletePersonaMutation();
  const setPersonaActiveMutation = useSetPersonaActiveMutation();
  const exportPersonaJsonMutation = useExportPersonaJsonMutation();
  const channelsQuery = useChannelsQuery();
  const presenceQuery = usePresenceQuery(
    effectivePubkey ? [effectivePubkey] : [],
  );
  const userStatusQuery = useUserStatusQuery(
    effectivePubkey ? [effectivePubkey] : [],
  );
  const contactListQuery = useContactListQuery(currentPubkey);
  const followMutation = useFollowMutation(currentPubkey);
  const unfollowMutation = useUnfollowMutation(currentPubkey);
  const { onOpenAgentSession } = useAgentSession();
  const { goChannel } = useAppNavigation();
  const profile = resolvePanelProfile({
    managedAgent,
    persona: resolvedPersona,
    profile: profileQuery.data,
  });
  const ownerPubkey = profile?.ownerPubkey ?? null;
  const ownerProfileQuery = useUserProfileQuery(ownerPubkey ?? undefined);
  const presenceStatus = pubkeyLower
    ? presenceQuery.data?.[pubkeyLower]
    : undefined;
  const userStatus = pubkeyLower
    ? userStatusQuery.data?.[pubkeyLower]
    : undefined;

  const relayAgent = relayAgentsQuery.data?.find(
    (agent) => agent.pubkey.toLowerCase() === pubkeyLower,
  );
  const managedAgentLogQuery = useManagedAgentLogQuery(
    (view === "diagnostics" || view === "logs") &&
      managedAgent?.backend.type === "local"
      ? managedAgent.pubkey
      : null,
  );
  const isBot = Boolean(relayAgent || managedAgent || resolvedPersona);
  const managedAgentOwner = useIsManagedAgent(isBot ? effectivePubkey : null);
  const isOwner = resolvedPersona ? true : managedAgentOwner;

  // Populate the active-turns store for this agent so useActiveAgentTurns works
  // even if the Agents page hasn't been visited yet.
  const bridgeAgents = React.useMemo(
    () =>
      managedAgent
        ? [{ pubkey: managedAgent.pubkey, status: managedAgent.status }]
        : [],
    [managedAgent],
  );
  useActiveAgentTurnsBridge(bridgeAgents);
  useManagedAgentObserverBridge(bridgeAgents);
  const canEditAgent =
    isOwner === true &&
    (managedAgent !== undefined ||
      (resolvedPersona !== undefined && !resolvedPersona.isBuiltIn));
  const memoryQuery = useAgentMemoryQuery(effectivePubkey, {
    enabled: isOwner === true && Boolean(effectivePubkey),
  });
  const isSelf =
    currentPubkey !== undefined &&
    pubkeyLower.length > 0 &&
    pubkeyLower === currentPubkey.toLowerCase();
  const canViewActivity =
    isOwner === true && Boolean(onOpenAgentSession) && Boolean(effectivePubkey);
  const canOpenAgentLogs =
    isOwner === true && managedAgent?.backend.type === "local";
  const canInstantiateAgent =
    isOwner === true &&
    resolvedPersona !== undefined &&
    managedAgent === undefined;
  const isAgentActionPending =
    createAgentMutation.isPending ||
    updateManagedAgentMutation.isPending ||
    startAgentMutation.isPending ||
    stopAgentMutation.isPending ||
    deleteAgentMutation.isPending ||
    startOnLaunchMutation.isPending ||
    createPersonaMutation.isPending ||
    updatePersonaMutation.isPending ||
    deletePersonaMutation.isPending ||
    setPersonaActiveMutation.isPending ||
    exportPersonaJsonMutation.isPending;
  const isFollowing =
    !isSelf &&
    pubkeyLower.length > 0 &&
    (contactListQuery.data?.contacts.some(
      (contact) => contact.pubkey.toLowerCase() === pubkeyLower,
    ) ??
      false);

  const profileChannels = React.useMemo(
    () =>
      deriveProfileChannels(
        pubkeyLower,
        relayAgent,
        managedAgent,
        channelsQuery.data,
      ),
    [pubkeyLower, relayAgent, managedAgent, channelsQuery.data],
  );

  const channelIdToName = React.useMemo(() => {
    const map: Record<string, string> = {};
    for (const channel of channelsQuery.data ?? []) {
      map[channel.id] = channel.name;
    }
    return map;
  }, [channelsQuery.data]);

  const targetKey =
    effectivePubkey ?? `persona:${resolvedPersona?.id ?? "unknown"}`;
  const prevTargetKeyRef = React.useRef(targetKey);
  React.useEffect(() => {
    if (prevTargetKeyRef.current === targetKey) return;
    prevTargetKeyRef.current = targetKey;
    setView("summary", { replace: true });
  }, [setView, targetKey]);
  const handleMessage = React.useCallback(() => {
    if (!effectivePubkey) return;
    onOpenDm?.([effectivePubkey]);
    onClose();
  }, [effectivePubkey, onClose, onOpenDm]);

  const handleEditAgent = React.useCallback(() => {
    if (resolvedPersona && !resolvedPersona.isBuiltIn) {
      setPersonaDialogState(editPersonaDialogState(resolvedPersona));
      return;
    }
    setEditAgentOpen(true);
  }, [resolvedPersona]);

  const { deleteManagedAgentRecord, deleteManagedAgentsForPersona } =
    useProfileAgentDeletion({
      channels: channelsQuery.data,
      deleteManagedAgent: deleteAgentMutation.mutateAsync,
      managedAgent,
      managedAgents: managedAgentsQuery.data,
      presenceLookup: presenceQuery.data,
      relayAgents: relayAgentsQuery.data,
    });

  const createManagedAgentForPersona = React.useCallback(
    async (personaToStart: AgentPersona) => {
      const runtimes = availableRuntimesQuery.data ?? [];
      const defaultRuntime = runtimes[0] ?? null;
      const { runtime, warnings } = resolvePersonaRuntime(
        personaToStart.runtime,
        runtimes,
        defaultRuntime,
      );

      for (const warning of warnings) {
        toast.warning(warning);
      }

      if (!runtime) {
        throw new Error("No available runtime found for this agent.");
      }

      const input: CreateManagedAgentInput = {
        name: personaToStart.displayName,
        acpCommand: "buzz-acp",
        agentCommand: runtime.command,
        agentArgs: runtime.defaultArgs,
        mcpCommand: runtime.mcpCommand ?? "",
        personaId: personaToStart.id,
        systemPrompt: personaToStart.systemPrompt,
        avatarUrl: personaToStart.avatarUrl ?? undefined,
        model: personaToStart.model ?? undefined,
        envVars: personaToStart.envVars,
        spawnAfterCreate: true,
        startOnAppLaunch: true,
        backend: { type: "local" },
      };

      const created = await createAgentMutation.mutateAsync(input);
      void managedAgentsQuery.refetch();
      void relayAgentsQuery.refetch();
      return created;
    },
    [
      availableRuntimesQuery.data,
      createAgentMutation.mutateAsync,
      managedAgentsQuery.refetch,
      relayAgentsQuery.refetch,
    ],
  );

  const handleAgentPrimaryAction = React.useCallback(async () => {
    if (!managedAgent) return;

    try {
      if (isManagedAgentActive(managedAgent)) {
        const result = await stopManagedAgentWithRules({
          agent: managedAgent,
          channels: channelsQuery.data ?? [],
          relayAgents: relayAgentsQuery.data ?? [],
          stopManagedAgent: stopAgentMutation.mutateAsync,
        });
        toast.success(result.noticeMessage ?? `Stopped ${managedAgent.name}.`);
        return;
      }

      await startManagedAgentWithRules({
        agent: managedAgent,
        startManagedAgent: startAgentMutation.mutateAsync,
      });
      toast.success(
        managedAgent.backend.type === "provider"
          ? `Deploying ${managedAgent.name}.`
          : `Started ${managedAgent.name}.`,
      );
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Agent action failed.",
      );
    }
  }, [
    channelsQuery.data,
    managedAgent,
    relayAgentsQuery.data,
    startAgentMutation.mutateAsync,
    stopAgentMutation.mutateAsync,
  ]);

  const handleInstantiateAgent = React.useCallback(async () => {
    if (!resolvedPersona) return;

    try {
      const created = await createManagedAgentForPersona(resolvedPersona);
      if (created.spawnError) {
        toast.error(created.spawnError);
      } else {
        toast.success(`Started ${created.agent.name}.`);
      }
      if (created.profileSyncError) {
        toast.warning(created.profileSyncError);
      }
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Failed to start agent.",
      );
    }
  }, [createManagedAgentForPersona, resolvedPersona]);

  const handleToggleAgentAutoStart = React.useCallback(async () => {
    if (managedAgent?.backend.type !== "local") return;

    try {
      const updated = await startOnLaunchMutation.mutateAsync({
        pubkey: managedAgent.pubkey,
        startOnAppLaunch: !managedAgent.startOnAppLaunch,
      });
      toast.success(
        updated.startOnAppLaunch
          ? `Will start ${updated.name} automatically.`
          : `${updated.name} will stay manual-start only.`,
      );
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : "Failed to update startup preference.",
      );
    }
  }, [managedAgent, startOnLaunchMutation.mutateAsync]);

  const handleDeleteAgent = React.useCallback(async () => {
    if (!managedAgent) return;

    try {
      const result = await deleteManagedAgentRecord(managedAgent);
      if (result.cancelled) return;

      toast.success(`Deleted ${managedAgent.name}.`);
      onClose();
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Failed to delete agent.",
      );
    }
  }, [deleteManagedAgentRecord, managedAgent, onClose]);

  const handleSubmitPersona = React.useCallback(
    async (input: CreatePersonaInput | UpdatePersonaInput) => {
      await submitProfilePersonaDialog({
        createManagedAgentForPersona,
        createPersona: createPersonaMutation.mutateAsync,
        input,
        managedAgent,
        onDone: () => {
          setPersonaDialogState(null);
          void personasQuery.refetch();
        },
        previousPersona: resolvedPersona,
        runtimes: acpRuntimesQuery.data ?? [],
        updateManagedAgent: updateManagedAgentMutation.mutateAsync,
        updatePersona: updatePersonaMutation.mutateAsync,
      });
    },
    [
      createPersonaMutation.mutateAsync,
      createManagedAgentForPersona,
      managedAgent,
      personasQuery.refetch,
      resolvedPersona,
      acpRuntimesQuery.data,
      updateManagedAgentMutation.mutateAsync,
      updatePersonaMutation.mutateAsync,
    ],
  );

  const handleEditPersona = React.useCallback(() => {
    if (!resolvedPersona || resolvedPersona.isBuiltIn) return;
    setPersonaDialogState(editPersonaDialogState(resolvedPersona));
  }, [resolvedPersona]);

  const handleDuplicatePersona = React.useCallback(() => {
    if (!resolvedPersona) return;
    setPersonaDialogState(duplicatePersonaDialogState(resolvedPersona));
  }, [resolvedPersona]);

  const handleExportPersona = React.useCallback(() => {
    if (!resolvedPersona) return;
    exportPersonaJsonMutation.mutate(resolvedPersona.id, {
      onSuccess: (saved) => {
        if (saved) {
          toast.success(`Exported ${resolvedPersona.displayName}.`);
        }
      },
      onError: (error) => {
        toast.error(
          error instanceof Error ? error.message : "Failed to export agent.",
        );
      },
    });
  }, [exportPersonaJsonMutation, resolvedPersona]);

  const handleDeletePersona = React.useCallback(async () => {
    if (!resolvedPersona) return;

    if (resolvedPersona.isBuiltIn) {
      try {
        const deletedInstances =
          await deleteManagedAgentsForPersona(resolvedPersona);
        if (deletedInstances.cancelled) return;

        await setPersonaActiveMutation.mutateAsync({
          id: resolvedPersona.id,
          active: false,
        });
        toast.success(`Removed ${resolvedPersona.displayName} from My Agents.`);
        onClose();
      } catch (error) {
        toast.error(
          error instanceof Error ? error.message : "Failed to delete agent.",
        );
      }
      return;
    }

    if (resolvedPersona.sourceTeam) {
      toast.error("This agent is managed by a team.");
      return;
    }

    setPersonaToDelete(resolvedPersona);
  }, [
    deleteManagedAgentsForPersona,
    onClose,
    resolvedPersona,
    setPersonaActiveMutation.mutateAsync,
  ]);

  const handleConfirmDeletePersona = React.useCallback(
    async (personaToConfirm: AgentPersona) => {
      if (personaToConfirm.sourceTeam) {
        toast.error("This agent is managed by a team.");
        setPersonaToDelete(null);
        return;
      }

      try {
        const deletedInstances =
          await deleteManagedAgentsForPersona(personaToConfirm);
        if (deletedInstances.cancelled) return;

        await deletePersonaMutation.mutateAsync(personaToConfirm.id);
        toast.success(`Deleted ${personaToConfirm.displayName}.`);
        setPersonaToDelete(null);
        onClose();
      } catch (error) {
        toast.error(
          error instanceof Error ? error.message : "Failed to delete agent.",
        );
      }
    },
    [deleteManagedAgentsForPersona, deletePersonaMutation.mutateAsync, onClose],
  );

  const handleAddedToChannel = React.useCallback(
    (channel: Channel, result: AttachManagedAgentToChannelResult) => {
      if (result.restarted) {
        toast.success(
          `Added ${result.agent.name} to ${channel.name} and restarted it.`,
        );
      } else if (result.started) {
        toast.success(`Added ${result.agent.name} to ${channel.name}.`);
      } else if (result.membershipAdded) {
        toast.success(`Added ${result.agent.name} to ${channel.name}.`);
      } else {
        toast.success(`${result.agent.name} is already in ${channel.name}.`);
      }
      void managedAgentsQuery.refetch();
      void relayAgentsQuery.refetch();
      void channelsQuery.refetch();
    },
    [
      channelsQuery.refetch,
      managedAgentsQuery.refetch,
      relayAgentsQuery.refetch,
    ],
  );

  const handleOpenActivity = React.useCallback(() => {
    if (!effectivePubkey) return;
    onClose();
    onOpenAgentSession?.(effectivePubkey);
  }, [effectivePubkey, onClose, onOpenAgentSession]);

  const handleOpenChannel = React.useCallback(
    (channelId: string) => {
      void goChannel(channelId);
    },
    [goChannel],
  );
  const displayName = resolveProfileDisplayName({
    persona: resolvedPersona,
    profile,
    pubkey: effectivePubkey,
  });
  const ownerHandle = ownerPubkey
    ? resolveOwnerHandle(ownerProfileQuery.data, ownerPubkey)
    : isOwner === true
      ? resolveOwnerHandle(currentProfileQuery.data, currentPubkey)
      : null;
  const isCurrentUserOwner =
    currentPubkey !== undefined &&
    ownerPubkey !== null &&
    ownerPubkey.toLowerCase() === currentPubkey.toLowerCase();
  const ownerDisplayName = ownerHandle
    ? isCurrentUserOwner || (!ownerPubkey && isOwner === true)
      ? `${ownerHandle} (you)`
      : ownerHandle
    : null;
  const ownerAvatarUrl = ownerPubkey
    ? (ownerProfileQuery.data?.avatarUrl ?? null)
    : (currentProfileQuery.data?.avatarUrl ?? null);
  const handleOpenOwner = React.useCallback(() => {
    if (!ownerPubkey) return;
    onOpenProfile?.(ownerPubkey);
  }, [onOpenProfile, ownerPubkey]);
  const onOpenOwner =
    ownerPubkey && onOpenProfile ? handleOpenOwner : undefined;
  const memoryCount = memoryQuery.data
    ? (memoryQuery.data.core ? 1 : 0) + memoryQuery.data.memories.length
    : undefined;
  const agentInstruction = resolveAgentInstruction(
    managedAgent,
    resolvedPersona,
  );
  const canManagePersona = isOwner === true && resolvedPersona !== undefined;
  const canEditPersona =
    canManagePersona && resolvedPersona?.isBuiltIn !== true;
  const canDeletePersona = canManagePersona && !resolvedPersona?.sourceTeam;
  const {
    agentInfoFields,
    agentSettingsFields,
    diagnosticsFields,
    diagnosticsSummary,
    modelLabel,
  } = useProfileFieldBuckets({
    isBot,
    isOwner,
    managedAgent,
    onOpenOwner,
    ownerAvatarUrl,
    ownerDisplayName,
    ownerHandle,
    ownerPubkey,
    persona: resolvedPersona,
    presenceLoaded: presenceQuery.isSuccess,
    presenceStatus,
    profile,
    pubkey: effectivePubkey,
    relayAgent,
  });

  const headerLeftContent = (
    <AuxiliaryPanelHeaderGroup>
      {view !== "summary" ? (
        <Button
          aria-label="Back to profile"
          className="shrink-0"
          data-testid="user-profile-panel-back"
          onClick={() => setView("summary")}
          size="icon"
          type="button"
          variant="outline"
        >
          <ArrowLeft />
        </Button>
      ) : null}
      <AuxiliaryPanelTitle>
        {PROFILE_PANEL_VIEW_TITLES[view]}
      </AuxiliaryPanelTitle>
    </AuxiliaryPanelHeaderGroup>
  );

  const headerActions = (
    <div className="ml-auto flex shrink-0 items-center gap-2">
      {view === "memories" && isOwner === true && effectivePubkey ? (
        <MemoryRefreshButton agentPubkey={effectivePubkey} variant="outline" />
      ) : null}
      <Button
        aria-label="Close profile"
        data-testid="user-profile-panel-close"
        onClick={onClose}
        size="icon"
        type="button"
        variant="ghost"
      >
        <X />
      </Button>
    </div>
  );
  const isDiagnosticsLikeView = view === "diagnostics" || view === "logs";

  const profileBody = (
    <div
      className={cn(
        "min-h-0 flex-1 px-4 pb-6",
        isDiagnosticsLikeView
          ? "flex flex-col overflow-hidden"
          : "overflow-y-auto",
        isSplitLayout && auxiliaryPanelContentPaddingClass,
        !isSplitLayout && !isFloatingOverlay && "pt-[4.75rem]",
      )}
    >
      {view === "summary" ? (
        <ProfileSummaryView
          canEditAgent={canEditAgent}
          canInstantiateAgent={canInstantiateAgent}
          canOpenAgentLogs={canOpenAgentLogs}
          canViewActivity={canViewActivity}
          channelCount={profileChannels.length}
          channelIdToName={channelIdToName}
          channelsLoading={channelsQuery.isLoading}
          displayName={displayName}
          followMutation={followMutation}
          agentInstruction={agentInstruction}
          handleAgentPrimaryAction={handleAgentPrimaryAction}
          handleDeleteAgent={handleDeleteAgent}
          handleDeletePersona={
            canDeletePersona ? handleDeletePersona : undefined
          }
          handleDuplicatePersona={
            canManagePersona ? handleDuplicatePersona : undefined
          }
          handleEditAgent={handleEditAgent}
          handleEditPersona={canEditPersona ? handleEditPersona : undefined}
          handleExportPersona={
            canManagePersona ? handleExportPersona : undefined
          }
          handleInstantiateAgent={handleInstantiateAgent}
          handleMessage={handleMessage}
          isBot={isBot}
          isAgentActionPending={isAgentActionPending}
          isFollowing={isFollowing}
          isOwner={isOwner}
          isSelf={isSelf}
          managedAgent={managedAgent}
          memoriesLoading={memoryQuery.isLoading}
          memoryCount={memoryCount}
          agentInfoFields={agentInfoFields}
          agentSettingsFields={agentSettingsFields}
          diagnosticsFields={diagnosticsFields}
          diagnosticsSummary={diagnosticsSummary}
          onOpenActivity={handleOpenActivity}
          onOpenAgentConfiguration={() => setView("configuration")}
          onOpenChannels={() => setView("channels")}
          onOpenDiagnostics={() => setView("diagnostics")}
          onOpenMemories={() => setView("memories")}
          onOpenDm={onOpenDm}
          onToggleAutoStart={handleToggleAgentAutoStart}
          persona={resolvedPersona}
          presenceStatus={presenceStatus}
          profile={profile}
          pubkey={effectivePubkey}
          relayAgent={relayAgent}
          unfollowMutation={unfollowMutation}
          userStatus={userStatus}
        />
      ) : null}
      {view === "memories" && effectivePubkey ? (
        <MemoryFocusedView agentPubkey={effectivePubkey} isOwner={isOwner} />
      ) : null}
      {view === "info" ? (
        <AgentInfoFocusedView metadataFields={agentInfoFields} />
      ) : null}
      {view === "configuration" ? (
        <AgentConfigurationFocusedView
          fields={agentSettingsFields}
          instruction={agentInstruction}
          managedAgent={managedAgent}
          modelLabel={modelLabel}
          onEditInstruction={canEditPersona ? handleEditPersona : undefined}
        />
      ) : null}
      {view === "diagnostics" ? (
        <DiagnosticsFocusedView
          canOpenAgentLogs={canOpenAgentLogs}
          fields={diagnosticsFields}
          logContent={managedAgentLogQuery.data?.content ?? null}
          logError={
            managedAgentLogQuery.error instanceof Error
              ? managedAgentLogQuery.error
              : null
          }
          logLoading={managedAgentLogQuery.isLoading}
          managedAgent={managedAgent}
        />
      ) : null}
      {view === "channels" ? (
        <ChannelsFocusedView
          canAddToChannel={managedAgent !== undefined && isOwner === true}
          channels={profileChannels}
          isActionPending={isAgentActionPending}
          isLoading={channelsQuery.isLoading}
          onAddToChannel={() => setAddToChannelOpen(true)}
          onOpenChannel={handleOpenChannel}
        />
      ) : null}
      {view === "logs" ? (
        <DiagnosticsFocusedView
          canOpenAgentLogs={canOpenAgentLogs}
          fields={[]}
          logContent={managedAgentLogQuery.data?.content ?? null}
          logError={
            managedAgentLogQuery.error instanceof Error
              ? managedAgentLogQuery.error
              : null
          }
          logLoading={managedAgentLogQuery.isLoading}
          managedAgent={managedAgent}
        />
      ) : null}
    </div>
  );
  const editAgentDialog =
    canEditAgent && managedAgent ? (
      <EditAgentDialog
        agent={managedAgent}
        onOpenChange={setEditAgentOpen}
        open={editAgentOpen}
      />
    ) : null;
  const addAgentToChannelDialog = managedAgent ? (
    <AddAgentToChannelDialog
      agent={managedAgent ?? null}
      onAdded={handleAddedToChannel}
      onOpenChange={setAddToChannelOpen}
      open={addToChannelOpen}
    />
  ) : null;
  const personaDialogs = (
    <UserProfilePersonaDialogs
      createError={
        createPersonaMutation.error instanceof Error
          ? createPersonaMutation.error
          : null
      }
      isPending={
        createPersonaMutation.isPending ||
        updatePersonaMutation.isPending ||
        updateManagedAgentMutation.isPending ||
        createAgentMutation.isPending
      }
      personaDialogState={personaDialogState}
      personaToDelete={personaToDelete}
      runtimes={acpRuntimesQuery.data ?? []}
      runtimesLoading={acpRuntimesQuery.isLoading}
      updateError={
        updatePersonaMutation.error instanceof Error
          ? updatePersonaMutation.error
          : null
      }
      onCloseDelete={() => setPersonaToDelete(null)}
      onCloseDialog={() => setPersonaDialogState(null)}
      onConfirmDelete={(selectedPersona) => {
        void handleConfirmDeletePersona(selectedPersona);
      }}
      onSubmit={handleSubmitPersona}
    />
  );
  if (isSplitLayout) {
    return (
      <>
        <div className="flex min-h-0 flex-1 flex-col">
          <AuxiliaryPanelHeader>
            {headerLeftContent}
            {headerActions}
          </AuxiliaryPanelHeader>
          {profileBody}
        </div>
        {editAgentDialog}
        {addAgentToChannelDialog}
        {personaDialogs}
      </>
    );
  }

  return (
    <>
      {isFloatingOverlay && <OverlayPanelBackdrop onClose={onClose} />}
      <aside
        className={cn(
          PANEL_BASE_CLASS,
          isSinglePanelView && "border-l-0",
          isFloatingOverlay && PANEL_OVERLAY_CLASS,
        )}
        data-testid="user-profile-panel"
        style={{
          width: isSinglePanelView
            ? "100%"
            : splitPaneClamp
              ? `min(${widthPx}px, calc(100% - ${THREAD_PANEL_MIN_WIDTH_PX}px))`
              : `${widthPx}px`,
        }}
      >
        {!isOverlay && !isSinglePanelView && onResizeStart && (
          <button
            aria-label="Resize profile panel"
            className="peer/profile-resize group/profile-resize absolute inset-y-0 left-0 z-40 w-3 -translate-x-1/2 cursor-col-resize"
            data-testid="user-profile-resize-handle"
            onDoubleClick={canResetWidth ? onResetWidth : undefined}
            onPointerDown={onResizeStart}
            title={
              canResetWidth
                ? "Drag to resize. Double-click to reset width."
                : "Drag to resize."
            }
            type="button"
          >
            <span className="absolute bottom-0 left-1/2 top-10 w-px -translate-x-1/2 bg-transparent transition-colors group-hover/profile-resize:bg-border/80 group-focus-visible/profile-resize:bg-border/80" />
          </button>
        )}

        {!isOverlay ? (
          <div
            aria-hidden="true"
            className="pointer-events-none absolute inset-x-0 top-0 z-40 h-[4.75rem] bg-background/80 backdrop-blur-md after:absolute after:left-0 after:right-0 after:top-10 after:h-px after:bg-border/35 supports-[backdrop-filter]:bg-background/70 peer-hover/profile-resize:after:bg-border/80 peer-focus-visible/profile-resize:after:bg-border/80 dark:bg-background/70 dark:backdrop-blur-xl dark:supports-[backdrop-filter]:bg-background/55"
          />
        ) : null}

        <div
          className={cn(
            "flex cursor-default select-none items-center",
            isSinglePanelView
              ? `relative ${PANEL_SINGLE_COLUMN_HEADER_LAYER_CLASS} -mb-[4.75rem] min-h-[4.75rem] shrink-0 gap-2.5 bg-transparent pb-1 pl-4 pr-2 pt-[2.625rem] sm:pl-6 sm:pr-3`
              : isOverlay
                ? "relative z-50 min-h-14 shrink-0 gap-3 bg-background/80 px-5 py-2 backdrop-blur-md supports-[backdrop-filter]:bg-background/70 dark:bg-background/70 dark:backdrop-blur-xl dark:supports-[backdrop-filter]:bg-background/55"
                : "absolute inset-x-0 top-[2.625rem] z-50 min-h-8 gap-3 bg-transparent px-3 py-1 after:absolute after:bottom-0 after:-left-px after:top-0 after:w-px after:bg-border/45 after:transition-colors peer-hover/profile-resize:after:bg-border/80 peer-focus-visible/profile-resize:after:bg-border/80",
          )}
          data-tauri-drag-region
        >
          {headerLeftContent}
          {headerActions}
        </div>

        {profileBody}
      </aside>
      {editAgentDialog}
      {addAgentToChannelDialog}
      {personaDialogs}
    </>
  );
}
